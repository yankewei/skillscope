use clap::{Args, Parser, Subcommand, ValueEnum};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

pub const DEFAULT_SERVICE_URL: &str = "http://127.0.0.1:3766";
pub const DEFAULT_DAEMON_ADDR: &str = "127.0.0.1:3766";

#[derive(Clone, Debug, Parser)]
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

    #[arg(long, global = true, default_value = DEFAULT_SERVICE_URL)]
    pub service_url: String,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Clone, Debug, Subcommand)]
pub enum Command {
    Daemon(DaemonArgs),
    Dashboard(DashboardArgs),
    Scan(ScanArgs),
    Stats(StatsArgs),
    Doctor(DoctorArgs),
}

#[derive(Clone, Debug, Args)]
pub struct DashboardArgs {
    #[arg(long, default_value = DEFAULT_DAEMON_ADDR)]
    pub addr: SocketAddr,

    #[arg(long, value_parser = parse_duration, default_value = "30s")]
    pub poll_interval: Duration,

    #[arg(long, value_parser = parse_duration, default_value = "300ms")]
    pub debounce: Duration,
}

#[derive(Clone, Debug, Args)]
pub struct DaemonArgs {
    #[arg(long, default_value = DEFAULT_DAEMON_ADDR)]
    pub addr: SocketAddr,

    #[arg(long, value_parser = parse_duration, default_value = "30s")]
    pub poll_interval: Duration,

    #[arg(long, value_parser = parse_duration, default_value = "300ms")]
    pub debounce: Duration,

    #[command(subcommand)]
    pub command: Option<DaemonCommand>,
}

#[derive(Clone, Debug, Subcommand)]
pub enum DaemonCommand {
    Run,
    Start,
    Status,
    Stop,
}

#[derive(Clone, Debug, Args)]
pub struct ScanArgs {
    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub rescan: bool,
}

#[derive(Clone, Debug, Args)]
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

#[derive(Clone, Debug, Args)]
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
