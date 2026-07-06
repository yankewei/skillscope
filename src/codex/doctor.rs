use crate::claude::scan::project_files;
use crate::codex::registry::SkillRegistry;
use crate::codex::scan::session_files;
use crate::config::Config;
use crate::db::Database;
use crate::error::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct DoctorReport {
    pub codex_home: String,
    pub codex_home_exists: bool,
    pub sessions_dir: String,
    pub sessions_dir_exists: bool,
    pub session_files: usize,
    pub claude_home: String,
    pub claude_home_exists: bool,
    pub claude_projects_dir: String,
    pub claude_projects_dir_exists: bool,
    pub claude_project_files: usize,
    pub parsed_files: u64,
    pub skills_found: usize,
    pub registry_diagnostics: Vec<String>,
    pub latest_parse_error: Option<String>,
}

pub fn report(db: &Database, config: &Config) -> Result<DoctorReport> {
    let registry = SkillRegistry::scan(config)?;
    let session_files = session_files(config)?;
    let claude_project_files = project_files(config)?;
    Ok(DoctorReport {
        codex_home: config.codex_home.to_string_lossy().into_owned(),
        codex_home_exists: config.codex_home.exists(),
        sessions_dir: config.sessions_dir().to_string_lossy().into_owned(),
        sessions_dir_exists: config.sessions_dir().exists(),
        session_files: session_files.len(),
        claude_home: config.claude_home.to_string_lossy().into_owned(),
        claude_home_exists: config.claude_home.exists(),
        claude_projects_dir: config.claude_projects_dir().to_string_lossy().into_owned(),
        claude_projects_dir_exists: config.claude_projects_dir().exists(),
        claude_project_files: claude_project_files.len(),
        parsed_files: db.parsed_file_count()?,
        skills_found: registry.len(),
        registry_diagnostics: registry.diagnostics().to_vec(),
        latest_parse_error: db.latest_parse_error()?,
    })
}

pub fn print_report(report: DoctorReport, json: bool) -> Result<()> {
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
    println!(
        "claude_home: {} ({})",
        report.claude_home,
        exists(report.claude_home_exists)
    );
    println!(
        "claude_projects_dir: {} ({})",
        report.claude_projects_dir,
        exists(report.claude_projects_dir_exists)
    );
    println!("claude_project_files: {}", report.claude_project_files);
    println!("parsed_files: {}", report.parsed_files);
    println!("skills_found: {}", report.skills_found);
    for diagnostic in report.registry_diagnostics {
        println!("registry_diagnostic: {diagnostic}");
    }
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
