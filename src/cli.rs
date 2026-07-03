use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(name = "skillscope")]
#[command(about = "Local Skill invocation analytics for agent session logs")]
pub struct Cli {
    #[arg(long, global = true)]
    pub codex_home: Option<PathBuf>,

    #[arg(long, global = true)]
    pub claude_home: Option<PathBuf>,

    #[arg(long, global = true)]
    pub agents_home: Option<PathBuf>,

    #[arg(long, global = true)]
    pub db: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Scan(ScanArgs),
    Watch(WatchArgs),
    Stats(StatsArgs),
    Doctor(DoctorArgs),
}

#[derive(Debug, Args)]
pub struct ScanArgs {
    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub rescan: bool,
}

#[derive(Debug, Args)]
pub struct WatchArgs {
    #[arg(long, value_parser = parse_duration, default_value = "30s")]
    pub poll_interval: Duration,

    #[arg(long, value_parser = parse_duration, default_value = "300ms")]
    pub debounce: Duration,
}

#[derive(Debug, Args)]
pub struct StatsArgs {
    #[arg(long, value_enum, default_value = "skill")]
    pub group_by: GroupBy,

    #[arg(long)]
    pub since: Option<String>,

    #[arg(long)]
    pub json: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum GroupBy {
    Skill,
    InvocationType,
}

#[derive(Debug, Args)]
pub struct DoctorArgs {
    #[arg(long)]
    pub json: bool,
}

fn parse_duration(value: &str) -> std::result::Result<Duration, String> {
    if let Some(ms) = value.strip_suffix("ms") {
        return ms
            .parse::<u64>()
            .map(Duration::from_millis)
            .map_err(|err| err.to_string());
    }
    if let Some(seconds) = value.strip_suffix('s') {
        return seconds
            .parse::<u64>()
            .map(Duration::from_secs)
            .map_err(|err| err.to_string());
    }
    value
        .parse::<u64>()
        .map(Duration::from_secs)
        .map_err(|err| err.to_string())
}
