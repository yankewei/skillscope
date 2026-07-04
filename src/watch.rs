use crate::claude::scan as claude_scan;
use crate::codex::registry::SkillRegistry;
use crate::codex::scan::{scan_all_with_registry, scan_file as scan_codex_file, ScanResult};
use crate::config::Config;
use crate::db::Database;
use crate::error::{Result, SkillScopeError};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

pub fn run_with_scan_lock(
    db: &mut Database,
    config: &Config,
    poll_interval: Duration,
    debounce: Duration,
    scan_lock: Option<Arc<Mutex<()>>>,
) -> Result<()> {
    let mut registry = SkillRegistry::scan(config)?;
    let initial = with_scan_lock(&scan_lock, || {
        let mut initial = scan_all_with_registry(db, config, &registry, false)?;
        merge_scan_result(
            &mut initial,
            claude_scan::scan_all_with_registry(db, config, &registry, false)?,
        );
        Ok(initial)
    })?;
    println!(
        "initial scan: {} files, {} new skill invocations",
        initial.files_scanned, initial.events_inserted
    );

    let sessions_dir = config.sessions_dir();
    let claude_projects_dir = config.claude_projects_dir();
    let (tx, rx) = mpsc::channel();
    let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })?;
    let mut watched_paths = HashSet::new();
    watch_if_exists(&mut watcher, &mut watched_paths, &sessions_dir)?;
    watch_if_exists(&mut watcher, &mut watched_paths, &claude_projects_dir)?;
    for path in registry_watch_roots(config) {
        watch_if_exists(&mut watcher, &mut watched_paths, &path)?;
    }

    loop {
        match rx.recv_timeout(poll_interval) {
            Ok(Ok(event)) => {
                let mut pending: HashSet<PathBuf> = event.paths.into_iter().collect();
                loop {
                    match rx.recv_timeout(debounce) {
                        Ok(Ok(ev)) => pending.extend(ev.paths),
                        Ok(Err(err)) => eprintln!("watch error: {err}"),
                        Err(mpsc::RecvTimeoutError::Timeout) => break,
                        Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                    }
                }
                let mut totals = (0u64, 0u64);
                if pending.iter().any(|path| is_registry_path(config, path)) {
                    registry = SkillRegistry::scan(config)?;
                }
                for path in pending {
                    if is_registry_path(config, &path) {
                        continue;
                    }
                    if !is_jsonl(&path) {
                        continue;
                    }
                    with_scan_lock(&scan_lock, || {
                        if path.starts_with(&claude_projects_dir) {
                            match claude_scan::scan_file(db, &path, &registry, false) {
                                Ok(result) => {
                                    totals.0 += result.events_inserted;
                                    totals.1 += result.errors;
                                }
                                Err(err) => eprintln!("scan {}: {err}", path.display()),
                            }
                        } else {
                            match scan_codex_file(db, &path, &registry, false) {
                                Ok(result) => {
                                    totals.0 += result.events_inserted;
                                    totals.1 += result.errors;
                                }
                                Err(err) => eprintln!("scan {}: {err}", path.display()),
                            }
                        }
                        Ok(())
                    })?;
                }
                if totals.0 > 0 || totals.1 > 0 {
                    println!(
                        "scan: {} new skill invocations, {} errors",
                        totals.0, totals.1
                    );
                }
            }
            Ok(Err(err)) => {
                eprintln!("watch error: {err}");
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let mut registry_changed = false;
                for path in watch_roots(config) {
                    if watch_if_exists(&mut watcher, &mut watched_paths, &path)?
                        && is_registry_path(config, &path)
                    {
                        registry_changed = true;
                    }
                }
                if registry_changed {
                    registry = SkillRegistry::scan(config)?;
                }

                let result = with_scan_lock(&scan_lock, || {
                    let mut result = scan_all_with_registry(db, config, &registry, false)?;
                    merge_scan_result(
                        &mut result,
                        claude_scan::scan_all_with_registry(db, config, &registry, false)?,
                    );
                    Ok(result)
                })?;
                if result.events_inserted > 0 || result.errors > 0 {
                    println!(
                        "poll scan: {} files, {} new skill invocations, {} errors",
                        result.files_scanned, result.events_inserted, result.errors
                    );
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

fn is_jsonl(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("jsonl")
}

fn with_scan_lock<T, F>(scan_lock: &Option<Arc<Mutex<()>>>, f: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    let _guard = match scan_lock {
        Some(lock) => Some(
            lock.lock()
                .map_err(|_| SkillScopeError::Service("scan lock poisoned".to_string()))?,
        ),
        None => None,
    };
    f()
}

fn watch_roots(config: &Config) -> Vec<PathBuf> {
    let mut roots = vec![config.sessions_dir(), config.claude_projects_dir()];
    roots.extend(registry_watch_roots(config));
    roots
}

fn registry_watch_roots(config: &Config) -> Vec<PathBuf> {
    vec![
        config.codex_home.join("skills"),
        config.agents_home.join("skills"),
        config.codex_home.join("plugins").join("cache"),
        config.claude_home.join("skills"),
        config.claude_home.join("plugins").join("cache"),
    ]
}

fn is_registry_path(config: &Config, path: &Path) -> bool {
    registry_watch_roots(config)
        .iter()
        .any(|root| path.starts_with(root))
}

fn watch_if_exists(
    watcher: &mut RecommendedWatcher,
    watched_paths: &mut HashSet<PathBuf>,
    path: &Path,
) -> Result<bool> {
    if watched_paths.contains(path) {
        return Ok(false);
    }
    if path.exists() {
        watcher.watch(path, RecursiveMode::Recursive)?;
        watched_paths.insert(path.to_path_buf());
        println!("watching {}", path.to_string_lossy());
        return Ok(true);
    }
    Ok(false)
}

fn merge_scan_result(result: &mut ScanResult, other: ScanResult) {
    result.files_scanned += other.files_scanned;
    result.events_inserted += other.events_inserted;
    result.errors += other.errors;
    result.events.extend(other.events);
}
