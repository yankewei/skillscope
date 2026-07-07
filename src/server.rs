use crate::api::{DoctorResponse, ErrorResponse, ScanRequest, ScanResponse};
use crate::claude;
use crate::codex;
use crate::codex::registry::SkillRegistry;
use crate::config::Config;
use crate::db::Database;
use crate::error::Result;
use crate::stats;
use crate::watch;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

#[derive(Clone)]
struct AppState {
    config: Config,
    db: Arc<Mutex<Database>>,
    scan_lock: Arc<Mutex<()>>,
    shutdown_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

#[derive(Debug, Deserialize)]
struct StatsQuery {
    since: Option<String>,
}

type ApiResult<T> = std::result::Result<Json<T>, (StatusCode, Json<ErrorResponse>)>;

pub async fn run(
    config: Config,
    addr: SocketAddr,
    poll_interval: Duration,
    debounce: Duration,
) -> Result<()> {
    let mut db = Database::open(&config.db_path)?;
    db.init()?;
    let scan_lock = Arc::new(Mutex::new(()));
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    spawn_watcher(config.clone(), poll_interval, debounce, scan_lock.clone());
    let state = AppState {
        config,
        db: Arc::new(Mutex::new(db)),
        scan_lock,
        shutdown_tx: Arc::new(Mutex::new(Some(shutdown_tx))),
    };
    let app = app(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("skillscope service listening on http://{addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(shutdown_rx))
        .await?;
    Ok(())
}

fn app(state: AppState) -> Router {
    Router::new()
        .route("/", get(dashboard))
        .route("/dashboard", get(dashboard))
        .route("/health", get(health))
        .route("/scan", post(scan))
        .route("/stats/skills", get(skill_stats))
        .route("/stats/invocation-types", get(invocation_type_stats))
        .route("/doctor", get(doctor))
        .route("/shutdown", post(shutdown))
        .with_state(state)
}

fn spawn_watcher(
    config: Config,
    poll_interval: Duration,
    debounce: Duration,
    scan_lock: Arc<Mutex<()>>,
) {
    std::thread::spawn(move || {
        let result = (|| -> Result<()> {
            let mut db = Database::open(&config.db_path)?;
            db.init()?;
            watch::run_with_scan_lock(&mut db, &config, poll_interval, debounce, Some(scan_lock))
        })();
        if let Err(err) = result {
            eprintln!("watch error: {err}");
        }
    });
}

async fn shutdown_signal(shutdown_rx: oneshot::Receiver<()>) {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = shutdown_rx => {}
    }
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

async fn dashboard() -> Html<&'static str> {
    Html(crate::dashboard::HTML)
}

async fn shutdown(
    State(state): State<AppState>,
) -> std::result::Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let mut tx = state.shutdown_tx.lock().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "shutdown lock poisoned".to_string(),
            }),
        )
    })?;
    if let Some(tx) = tx.take() {
        let _ = tx.send(());
    }
    Ok(Json(serde_json::json!({ "status": "stopping" })))
}

async fn scan(
    State(state): State<AppState>,
    Json(request): Json<ScanRequest>,
) -> ApiResult<ScanResponse> {
    let scan_lock = state.scan_lock.clone();
    let _guard = scan_lock.lock().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "scan lock poisoned".to_string(),
            }),
        )
    })?;
    run_with_db(state, move |db, config| {
        let registry = SkillRegistry::scan(config)?;
        let mut result =
            codex::scan::scan_all_with_registry(db, config, &registry, request.rescan)?;
        let claude_result =
            claude::scan::scan_all_with_registry(db, config, &registry, request.rescan)?;
        result.files_scanned += claude_result.files_scanned;
        result.events_inserted += claude_result.events_inserted;
        result.errors += claude_result.errors;
        result.events.extend(claude_result.events);
        Ok(result)
    })
}

async fn skill_stats(
    State(state): State<AppState>,
    Query(query): Query<StatsQuery>,
) -> ApiResult<crate::api::SkillStatsResponse> {
    run_with_db(state, move |db, _| {
        stats::skill_stats(db, query.since.as_deref())
    })
}

async fn invocation_type_stats(
    State(state): State<AppState>,
    Query(query): Query<StatsQuery>,
) -> ApiResult<crate::api::InvocationTypeStatsResponse> {
    run_with_db(state, move |db, _| {
        stats::invocation_type_stats(db, query.since.as_deref())
    })
}

async fn doctor(State(state): State<AppState>) -> ApiResult<DoctorResponse> {
    run_with_db(state, |db, config| codex::doctor::report(db, config))
}

fn run_with_db<T, F>(state: AppState, f: F) -> ApiResult<T>
where
    F: FnOnce(&mut Database, &Config) -> Result<T>,
{
    let mut db = state.db.lock().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "database lock poisoned".to_string(),
            }),
        )
    })?;
    f(&mut db, &state.config).map(Json).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: err.to_string(),
            }),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ServiceClient;
    use axum::body::{to_bytes, Body};
    use axum::http::{Method, Request, StatusCode};
    use tower::ServiceExt;

    fn test_state() -> (tempfile::TempDir, AppState) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let config = Config {
            codex_home: root.join(".codex"),
            claude_home: root.join(".claude"),
            agents_home: root.join(".agents"),
            db_path: root.join("skillscope.sqlite"),
        };
        let mut db = Database::open(&config.db_path).unwrap();
        db.init().unwrap();
        let (shutdown_tx, _shutdown_rx) = oneshot::channel();
        (
            tmp,
            AppState {
                config,
                db: Arc::new(Mutex::new(db)),
                scan_lock: Arc::new(Mutex::new(())),
                shutdown_tx: Arc::new(Mutex::new(Some(shutdown_tx))),
            },
        )
    }

    #[tokio::test]
    async fn dashboard_endpoint_serves_static_ui() {
        let (_tmp, state) = test_state();
        let response = app(state)
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/dashboard")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_text(response).await;
        assert!(body.contains("SkillScope"));
        assert!(body.contains("/stats/skills"));
        assert!(body.contains("Local metadata only. Prompts and outputs are not stored."));
    }

    #[tokio::test]
    async fn doctor_endpoint_reports_service_configuration() {
        let (_tmp, state) = test_state();
        let expected_codex_home = state.config.codex_home.to_string_lossy().into_owned();
        let response = app(state)
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/doctor")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_text(response).await;
        let report: DoctorResponse = serde_json::from_str(&body).unwrap();
        assert_eq!(report.codex_home, expected_codex_home);
        assert_eq!(report.parsed_files, 0);
    }

    #[tokio::test]
    async fn scan_endpoint_indexes_transcripts_and_stats_endpoint_reports_them() {
        let (_tmp, state) = test_state();
        let sessions_dir = state.config.codex_home.join("sessions");
        let skill_dir = state.config.agents_home.join("skills/diagnose");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "---\nname: diagnose\n---\n").unwrap();
        let skill_path = skill_dir.join("SKILL.md");
        std::fs::write(
            sessions_dir.join("session.jsonl"),
            format!(
                r#"{{"timestamp":"2026-07-02T00:00:00Z","type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"<skill>\n<name>diagnose</name>\n<path>{}</path>\n</skill>"}}]}}}}"#,
                skill_path.to_string_lossy()
            ) + "\n",
        )
        .unwrap();

        let router = app(state);
        let scan_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/scan")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"rescan":false}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(scan_response.status(), StatusCode::OK);
        let body = body_text(scan_response).await;
        let scan: ScanResponse = serde_json::from_str(&body).unwrap();
        assert_eq!(scan.events_inserted, 1);

        let stats_response = router
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/stats/skills")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(stats_response.status(), StatusCode::OK);
        let body = body_text(stats_response).await;
        let stats: crate::api::SkillStatsResponse = serde_json::from_str(&body).unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].skill_name, "diagnose");
        assert_eq!(stats[0].total, 1);
        assert_eq!(stats[0].codex, 1);
        assert_eq!(stats[0].claude_code, 0);
    }

    #[tokio::test]
    async fn service_client_scans_and_reads_stats_over_real_http() {
        let (_tmp, state) = test_state();
        let sessions_dir = state.config.codex_home.join("sessions");
        let claude_project_dir = state.config.claude_home.join("projects/project-one");
        let skill_dir = state.config.agents_home.join("skills/diagnose");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::create_dir_all(&claude_project_dir).unwrap();
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "---\nname: diagnose\n---\n").unwrap();
        let skill_path = skill_dir.join("SKILL.md");
        std::fs::write(
            sessions_dir.join("session.jsonl"),
            format!(
                r#"{{"timestamp":"2026-07-02T00:00:00Z","type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"<skill>\n<name>diagnose</name>\n<path>{}</path>\n</skill>"}}]}}}}"#,
                skill_path.to_string_lossy()
            ) + "\n",
        )
        .unwrap();
        std::fs::write(
            claude_project_dir.join("session.jsonl"),
            r#"{"type":"assistant","timestamp":"2026-07-02T00:00:01Z","sessionId":"session_1","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Skill","input":{"skill":"diagnose","args":"do not persist me"}}]}}"#.to_string()
                + "\n",
        )
        .unwrap();
        let (base_url, server) = spawn_test_server(state).await;

        tokio::task::spawn_blocking(move || {
            let client = ServiceClient::new(base_url);
            client.health().unwrap();
            let scan = client.scan(&ScanRequest { rescan: false }).unwrap();
            assert_eq!(scan.events_inserted, 2);

            let stats = client.skill_stats(None).unwrap();
            assert_eq!(stats.len(), 1);
            assert_eq!(stats[0].skill_name, "diagnose");
            assert_eq!(stats[0].total, 2);
            assert_eq!(stats[0].codex, 1);
            assert_eq!(stats[0].claude_code, 1);
            assert_eq!(stats[0].explicit, 1);
            assert_eq!(stats[0].skill, 1);
        })
        .await
        .unwrap();

        server.abort();
    }

    async fn body_text(response: axum::response::Response) -> String {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    async fn spawn_test_server(
        state: AppState,
    ) -> (String, tokio::task::JoinHandle<std::io::Result<()>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app(state)).await });
        (format!("http://{addr}"), server)
    }
}
