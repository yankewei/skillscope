mod claude;
mod cli;
mod codex;
mod config;
mod db;
mod error;
mod events;
mod paths;
mod stats;
mod watch;

use crate::cli::{Cli, Command};
use crate::config::Config;
use crate::db::Database;
use crate::error::Result;
use clap::Parser;

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::from_cli(&cli)?;
    let mut db = Database::open(&config.db_path)?;
    db.init()?;

    match cli.command {
        Command::Scan(args) => {
            let mut result = codex::scan::scan_all(&mut db, &config, args.rescan)?;
            let claude_result = claude::scan::scan_all(&mut db, &config, args.rescan)?;
            result.files_scanned += claude_result.files_scanned;
            result.events_inserted += claude_result.events_inserted;
            result.errors += claude_result.errors;
            result.events.extend(claude_result.events);
            if args.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!(
                    "scanned {} files, discovered {} new skill invocations, {} errors",
                    result.files_scanned, result.events_inserted, result.errors
                );
            }
        }
        Command::Watch(args) => {
            watch::run(&mut db, &config, args.poll_interval, args.debounce)?;
        }
        Command::Stats(args) => {
            stats::print_stats(&db, args.group_by, args.since, args.json)?;
        }
        Command::Doctor(args) => {
            codex::doctor::run(&db, &config, args.json)?;
        }
    }

    Ok(())
}
