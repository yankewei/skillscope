use crate::codex::registry::SkillRegistry;
use crate::codex::scan::session_files;
use crate::config::Config;
use crate::db::Database;
use crate::error::Result;
use serde::Serialize;

#[derive(Debug, Serialize)]
struct DoctorReport {
    codex_home: String,
    codex_home_exists: bool,
    sessions_dir: String,
    sessions_dir_exists: bool,
    session_files: usize,
    parsed_files: u64,
    skills_found: usize,
    latest_parse_error: Option<String>,
}

pub fn run(db: &Database, config: &Config, json: bool) -> Result<()> {
    let registry = SkillRegistry::scan(config)?;
    let session_files = session_files(config)?;
    let report = DoctorReport {
        codex_home: config.codex_home.to_string_lossy().into_owned(),
        codex_home_exists: config.codex_home.exists(),
        sessions_dir: config.sessions_dir().to_string_lossy().into_owned(),
        sessions_dir_exists: config.sessions_dir().exists(),
        session_files: session_files.len(),
        parsed_files: db.parsed_file_count()?,
        skills_found: registry.len(),
        latest_parse_error: db.latest_parse_error()?,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!(
        "codex_home: {} ({})",
        report.codex_home,
        exists(report.codex_home_exists)
    );
    println!(
        "sessions_dir: {} ({})",
        report.sessions_dir,
        exists(report.sessions_dir_exists)
    );
    println!("session_files: {}", report.session_files);
    println!("parsed_files: {}", report.parsed_files);
    println!("skills_found: {}", report.skills_found);
    if let Some(error) = report.latest_parse_error {
        println!("latest_parse_error: {error}");
    }
    Ok(())
}

fn exists(value: bool) -> &'static str {
    if value {
        "ok"
    } else {
        "missing"
    }
}
