use crate::codex::scan::scan_all;
use crate::config::Config;
use crate::db::Database;
use crate::error::Result;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::sync::mpsc;
use std::time::Duration;

pub fn run(
    db: &mut Database,
    config: &Config,
    poll_interval: Duration,
    debounce: Duration,
) -> Result<()> {
    let initial = scan_all(db, config, false)?;
    println!(
        "initial scan: {} files, {} new skill invocations",
        initial.files_scanned, initial.events_inserted
    );

    let sessions_dir = config.sessions_dir();
    if !sessions_dir.exists() {
        println!(
            "sessions directory does not exist yet: {}",
            sessions_dir.to_string_lossy()
        );
    }

    let (tx, rx) = mpsc::channel();
    let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })?;
    if sessions_dir.exists() {
        watcher.watch(&sessions_dir, RecursiveMode::Recursive)?;
    }

    println!("watching {}", sessions_dir.to_string_lossy());
    loop {
        match rx.recv_timeout(poll_interval) {
            Ok(Ok(_event)) => {
                std::thread::sleep(debounce);
                let result = scan_all(db, config, false)?;
                if result.events_inserted > 0 || result.errors > 0 {
                    println!(
                        "scan: {} files, {} new skill invocations, {} errors",
                        result.files_scanned, result.events_inserted, result.errors
                    );
                }
            }
            Ok(Err(err)) => {
                eprintln!("watch error: {err}");
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let result = scan_all(db, config, false)?;
                if result.events_inserted > 0 || result.errors > 0 {
                    println!(
                        "poll scan: {} files, {} new skill invocations, {} errors",
                        result.files_scanned, result.events_inserted, result.errors
                    );
                }
                if !sessions_dir.exists() {
                    continue;
                }
                let _ = watcher.watch(&sessions_dir, RecursiveMode::Recursive);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}
