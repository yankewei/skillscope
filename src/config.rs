use crate::cli::Cli;
use crate::error::{Result, SkillScopeError};
use crate::paths::expand_tilde;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub codex_home: PathBuf,
    pub claude_home: PathBuf,
    pub agents_home: PathBuf,
    pub db_path: PathBuf,
}

impl Config {
    pub fn from_cli(cli: &Cli) -> Result<Self> {
        let home = dirs::home_dir().ok_or_else(|| {
            SkillScopeError::InvalidPath("could not resolve home directory".into())
        })?;
        let data_dir = dirs::data_local_dir().unwrap_or_else(|| home.join(".local/share"));

        Ok(Self {
            codex_home: cli
                .codex_home
                .as_ref()
                .map(|path| expand_tilde(path))
                .unwrap_or_else(|| home.join(".codex")),
            claude_home: cli
                .claude_home
                .as_ref()
                .map(|path| expand_tilde(path))
                .unwrap_or_else(|| home.join(".claude")),
            agents_home: cli
                .agents_home
                .as_ref()
                .map(|path| expand_tilde(path))
                .unwrap_or_else(|| home.join(".agents")),
            db_path: cli
                .db
                .as_ref()
                .map(|path| expand_tilde(path))
                .unwrap_or_else(|| data_dir.join("skillscope").join("skillscope.sqlite")),
        })
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.codex_home.join("sessions")
    }

    pub fn claude_projects_dir(&self) -> PathBuf {
        self.claude_home.join("projects")
    }
}
