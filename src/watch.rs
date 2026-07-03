use crate::claude::scan as claude_scan;
use crate::codex::registry::SkillRegistry;
use crate::codex::scan::{scan_all_with_registry, scan_file, ScanResult};
use crate::config::Config;
use crate::db::Database;
use crate::error::Result;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

pub fn run(
    db: &mut Database,
    config: &Config,
    poll_interval: Duration,
    debounce: Duration,
) -> Result<()> {
    let mut registry = SkillRegistry::scan(config)?;
    let mut initial = scan_all_with_registry(db, config, &registry, false)?;
    merge_scan_result(
        &mut initial,
        claude_scan::scan_all_with_registry(db, config, &registry, false)?,
    );
    println!(
        "initial scan: {} files, {} new skill invocations",
        initial.files_scanned, initial.events_inserted
    );

    let sessions_dir = config.sessions_dir();
    let (tx, rx) = mpsc::channel();
    let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })?;
    let mut watching = false;
    if sessions_dir.exists() {
        watcher.watch(&sessions_dir, RecursiveMode::Recursive)?;
        watching = true;
        println!("watching {}", sessions_dir.to_string_lossy());
    } else {
        println!(
            "sessions directory does not exist yet: {}",
            sessions_dir.to_string_lossy()
        );
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
                for path in pending {
                    if !is_jsonl(&path) {
                        continue;
                    }
                    match scan_file(db, &path, &registry, false) {
                        Ok(result) => {
                            totals.0 += result.events_inserted;
                            totals.1 += result.errors;
                        }
                        Err(err) => eprintln!("scan {}: {err}", path.display()),
                    }
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
                registry = SkillRegistry::scan(config)?;
                let mut result = scan_all_with_registry(db, config, &registry, false)?;
                merge_scan_result(
                    &mut result,
                    claude_scan::scan_all_with_registry(db, config, &registry, false)?,
                );
                if result.events_inserted > 0 || result.errors > 0 {
                    println!(
                        "poll scan: {} files, {} new skill invocations, {} errors",
                        result.files_scanned, result.events_inserted, result.errors
                    );
                }
                if !watching && sessions_dir.exists() {
                    match watcher.watch(&sessions_dir, RecursiveMode::Recursive) {
                        Ok(()) => watching = true,
                        Err(err) => eprintln!("watch register error: {err}"),
                    }
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

fn merge_scan_result(result: &mut ScanResult, other: ScanResult) {
    result.files_scanned += other.files_scanned;
    result.events_inserted += other.events_inserted;
    result.errors += other.errors;
    result.events.extend(other.events);
}
