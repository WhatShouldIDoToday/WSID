use thiserror::Error;

#[derive(Debug, Error)]
pub enum WsidError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("invalid task ID '{0}': expected 2 letters (excluding I and O) followed by 4 digits (e.g. AB1234)")]
    InvalidTaskId(String),

    #[error("task not found: {0}")]
    TaskNotFound(String),

    #[error("validation error: {0}")]
    Validation(String),

    #[error("failed to generate a unique short task ID after {0} attempts")]
    IdCollisionExhausted(usize),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, WsidError>;
