//! Error types

use thiserror::Error;

pub type Result<T> = std::result::Result<T, ExecutorError>;

#[derive(Error, Debug)]
pub enum ExecutorError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Process spawn failed: {0}")]
    SpawnFailed(String),

    #[error("Process execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("Executor unavailable: {0}")]
    Unavailable(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}
