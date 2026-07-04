use thiserror::Error;

pub type Result<T> = std::result::Result<T, SkillScopeError>;

#[derive(Debug, Error)]
pub enum SkillScopeError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("notify error: {0}")]
    Notify(#[from] notify::Error),

    #[error("walkdir error: {0}")]
    Walkdir(#[from] walkdir::Error),

    #[error("invalid path: {0}")]
    InvalidPath(String),

    #[error("service unavailable: {0}")]
    ServiceUnavailable(String),

    #[error("service error: {0}")]
    Service(String),
}
