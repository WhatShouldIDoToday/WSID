use thiserror::Error;

#[derive(Debug, Error)]
pub enum WsidError {
    #[error("database error: {0}")]
    Database(String),

    #[error("this database uses schema version {found}, but this WSID supports up to version {supported}; upgrade WSID to open it")]
    DatabaseTooNew { found: i64, supported: i64 },

    #[error("invalid task ID '{0}': expected 2 letters (excluding I and O) followed by 4 digits (e.g. AB1234)")]
    InvalidTaskId(String),

    #[error("task not found: {0}")]
    TaskNotFound(String),

    #[error("cycle detected: linking {source_id} -{relation}-> {target_id} would create a circular dependency")]
    CycleDetected { source_id: String, target_id: String, relation: String },

    #[error("validation error: {0}")]
    Validation(String),

    #[error("validation error: {}", crate::draft::summarize_issues(.0))]
    DraftRejected(Vec<crate::draft::FieldIssue>),

    #[error("failed to generate a unique short task ID after {0} attempts")]
    IdCollisionExhausted(usize),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<rusqlite::Error> for WsidError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Database(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, WsidError>;
