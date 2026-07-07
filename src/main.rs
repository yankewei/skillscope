mod api;
mod claude;
mod cli;
mod client;
mod codex;
mod config;
mod dashboard;
mod db;
mod error;
mod events;
mod jsonl_cursor;
mod paths;
mod server;
mod stats;
mod tags;
mod watch;

use crate::api::ScanRequest;
use crate::cli::{
    Cli, Command, DaemonArgs, DaemonCommand, DashboardArgs, DEFAULT_DAEMON_ADDR,
    DEFAULT_SERVICE_URL,
};
use crate::client::ServiceClient;
use crate::config::Config;
use crate::error::{Result, SkillScopeError};
use clap::Parser;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::from_cli(&cli)?;
    let service_url = cli.service_url.clone();
    let command = cli.command.clone();

    match command {
        Command::Daemon(args) => {
            run_daemon_command(&cli, config, args)?;
        }
        Command::Dashboard(args) => {
            run_dashboard_command(&cli, config, args)?;
        }
        Command::Scan(args) => {
            let client = ServiceClient::new(service_url);
            let result = client.scan(&ScanRequest {
                rescan: args.rescan,
            })?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!(
                    "scanned {} files, discovered {} new skill invocations, {} errors",
                    result.files_scanned, result.events_inserted, result.errors
                );
            }
        }
        Command::Stats(args) => {
            let client = ServiceClient::new(service_url);
            match args.group_by {
                cli::GroupBy::Skill => {
                    let stats = client.skill_stats(args.since.as_deref())?;
                    stats::print_skill_stats_rows(stats, args.json)?;
                }
                cli::GroupBy::InvocationType => {
                    let stats = client.invocation_type_stats(args.since.as_deref())?;
                    stats::print_invocation_type_stats_rows(stats, args.json)?;
                }
            }
        }
        Command::Doctor(args) => {
            let client = ServiceClient::new(service_url);
            let report = client.doctor()?;
            codex::doctor::print_report(report, args.json)?;
        }
    }

    Ok(())
}

fn run_daemon_command(cli: &Cli, config: Config, args: DaemonArgs) -> Result<()> {
    let service_url = effective_service_url(cli, &args);
    match daemon_command(&args) {
        DaemonCommand::Run => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(server::run(
                config,
                args.addr,
                args.poll_interval,
                args.debounce,
            ))?;
        }
        DaemonCommand::Start => start_daemon(cli, &config, &args, &service_url)?,
        DaemonCommand::Status => {
            let client = ServiceClient::new(service_url.clone());
            client.health()?;
            println!("skillscope daemon is running at {service_url}");
        }
        DaemonCommand::Stop => {
            let client = ServiceClient::new(service_url);
            client.shutdown()?;
            println!("skillscope daemon is stopping");
        }
    }
    Ok(())
}

fn run_dashboard_command(cli: &Cli, config: Config, args: DashboardArgs) -> Result<()> {
    let service_url = effective_service_url_for_addr(cli, args.addr);
    let client = ServiceClient::new(service_url.clone());
    if client.health().is_err() {
        let daemon_args = DaemonArgs {
            addr: args.addr,
            poll_interval: args.poll_interval,
            debounce: args.debounce,
            command: Some(DaemonCommand::Start),
        };
        start_daemon(cli, &config, &daemon_args, &service_url)?;
    }

    let url = format!("{}/dashboard", service_url.trim_end_matches('/'));
    match open_url(&url) {
        Ok(()) => println!("opened SkillScope dashboard at {url}"),
        Err(err) => {
            eprintln!("could not open browser automatically: {err}");
            println!("SkillScope dashboard: {url}");
        }
    }
    Ok(())
}

fn daemon_command(args: &DaemonArgs) -> DaemonCommand {
    args.command.clone().unwrap_or(DaemonCommand::Start)
}

fn effective_service_url(cli: &Cli, args: &DaemonArgs) -> String {
    effective_service_url_for_addr(cli, args.addr)
}

fn effective_service_url_for_addr(cli: &Cli, addr: std::net::SocketAddr) -> String {
    let default_addr = default_daemon_addr();
    if cli.service_url == DEFAULT_SERVICE_URL && addr != default_addr {
        format!("http://{addr}")
    } else {
        cli.service_url.clone()
    }
}

fn default_daemon_addr() -> std::net::SocketAddr {
    DEFAULT_DAEMON_ADDR
        .parse()
        .expect("default daemon address is valid")
}

fn start_daemon(cli: &Cli, config: &Config, args: &DaemonArgs, service_url: &str) -> Result<()> {
    let client = ServiceClient::new(service_url.to_string());
    if client.health().is_ok() {
        println!("skillscope daemon is already running at {}", service_url);
        return Ok(());
    }

    let exe = std::env::current_exe()?;
    let log_path = daemon_log_path(&config.db_path);
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let err_log = log.try_clone()?;

    let mut command = ProcessCommand::new(exe);
    add_global_args(&mut command, cli);
    add_daemon_run_args(&mut command, args);
    command
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err_log))
        .stdin(Stdio::null());
    detach_daemon_process(&mut command);

    command.spawn()?;
    wait_for_daemon(&client, Duration::from_secs(5))?;
    println!(
        "skillscope daemon started at {} (log: {})",
        service_url,
        log_path.display()
    );
    Ok(())
}

fn add_global_args(command: &mut ProcessCommand, cli: &Cli) {
    if let Some(path) = &cli.codex_home {
        command.arg("--codex-home").arg(path);
    }
    if let Some(path) = &cli.claude_home {
        command.arg("--claude-home").arg(path);
    }
    if let Some(path) = &cli.agents_home {
        command.arg("--agents-home").arg(path);
    }
    if let Some(path) = &cli.db {
        command.arg("--db").arg(path);
    }
    command.arg("--service-url").arg(&cli.service_url);
}

fn add_daemon_run_args(command: &mut ProcessCommand, args: &DaemonArgs) {
    command
        .arg("daemon")
        .arg("--addr")
        .arg(args.addr.to_string())
        .arg("--poll-interval")
        .arg(format_duration(args.poll_interval))
        .arg("--debounce")
        .arg(format_duration(args.debounce))
        .arg("run");
}

fn detach_daemon_process(command: &mut ProcessCommand) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
}

fn wait_for_daemon(client: &ServiceClient, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        if client.health().is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(SkillScopeError::Service(
                "daemon did not become ready in time".to_string(),
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn open_url(url: &str) -> Result<()> {
    let status = open_url_command(url).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(SkillScopeError::Service(format!(
            "browser opener exited with {status}"
        )))
    }
}

fn open_url_command(url: &str) -> ProcessCommand {
    #[cfg(target_os = "macos")]
    {
        let mut command = ProcessCommand::new("open");
        command.arg(url);
        command
    }

    #[cfg(target_os = "windows")]
    {
        let mut command = ProcessCommand::new("cmd");
        command.args(["/C", "start", "", url]);
        command
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut command = ProcessCommand::new("xdg-open");
        command.arg(url);
        command
    }
}

fn daemon_log_path(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .map(|parent| parent.join("skillscope-daemon.log"))
        .unwrap_or_else(|| PathBuf::from("skillscope-daemon.log"))
}

fn format_duration(duration: Duration) -> String {
    if duration.subsec_millis() == 0 {
        format!("{}s", duration.as_secs())
    } else {
        format!("{}ms", duration.as_millis())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn daemon_args(addr: &str, command: Option<DaemonCommand>) -> DaemonArgs {
        DaemonArgs {
            addr: addr.parse().unwrap(),
            poll_interval: Duration::from_secs(30),
            debounce: Duration::from_millis(300),
            command,
        }
    }

    fn cli_with_service_url(service_url: &str, args: DaemonArgs) -> Cli {
        Cli {
            codex_home: None,
            claude_home: None,
            agents_home: None,
            db: None,
            service_url: service_url.to_string(),
            command: Command::Daemon(args),
        }
    }

    fn cli_with_dashboard_service_url(service_url: &str, args: DashboardArgs) -> Cli {
        Cli {
            codex_home: None,
            claude_home: None,
            agents_home: None,
            db: None,
            service_url: service_url.to_string(),
            command: Command::Dashboard(args),
        }
    }

    #[test]
    fn bare_daemon_defaults_to_background_start() {
        let args = daemon_args(DEFAULT_DAEMON_ADDR, None);

        assert!(matches!(daemon_command(&args), DaemonCommand::Start));
    }

    #[test]
    fn daemon_addr_sets_effective_service_url_when_default_url_is_used() {
        let args = daemon_args("127.0.0.1:4000", Some(DaemonCommand::Status));
        let cli = cli_with_service_url(DEFAULT_SERVICE_URL, args.clone());

        assert_eq!(effective_service_url(&cli, &args), "http://127.0.0.1:4000");
    }

    #[test]
    fn explicit_service_url_is_preserved_for_daemon_commands() {
        let args = daemon_args("127.0.0.1:4000", Some(DaemonCommand::Status));
        let cli = cli_with_service_url("http://127.0.0.1:5000", args.clone());

        assert_eq!(effective_service_url(&cli, &args), "http://127.0.0.1:5000");
    }

    #[test]
    fn dashboard_addr_sets_effective_service_url_when_default_url_is_used() {
        let args = DashboardArgs {
            addr: "127.0.0.1:4000".parse().unwrap(),
            poll_interval: Duration::from_secs(30),
            debounce: Duration::from_millis(300),
        };
        let cli = cli_with_dashboard_service_url(DEFAULT_SERVICE_URL, args.clone());

        assert_eq!(
            effective_service_url_for_addr(&cli, args.addr),
            "http://127.0.0.1:4000"
        );
    }

    #[test]
    fn dashboard_opener_targets_platform_browser_command() {
        let command = open_url_command("http://127.0.0.1:3766/dashboard");
        let program = command.get_program().to_string_lossy();

        #[cfg(target_os = "macos")]
        assert_eq!(program, "open");
        #[cfg(target_os = "windows")]
        assert_eq!(program, "cmd");
        #[cfg(all(unix, not(target_os = "macos")))]
        assert_eq!(program, "xdg-open");
    }

    #[test]
    fn start_spawns_foreground_run_subcommand() {
        let args = daemon_args(DEFAULT_DAEMON_ADDR, Some(DaemonCommand::Start));
        let mut command = ProcessCommand::new("skillscope");

        add_daemon_run_args(&mut command, &args);

        let actual = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            actual,
            vec![
                "daemon",
                "--addr",
                DEFAULT_DAEMON_ADDR,
                "--poll-interval",
                "30s",
                "--debounce",
                "300ms",
                "run"
            ]
        );
    }
}
