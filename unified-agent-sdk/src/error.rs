//! Error types

use std::fmt;

use thiserror::Error;

/// Result type used by the unified executor APIs.
pub type Result<T> = std::result::Result<T, ExecutorError>;

/// Error type for executor lifecycle, transport, and normalization failures.
#[derive(Error, Debug)]
pub enum ExecutorError {
    /// Underlying I/O error while interacting with local files or subprocess pipes.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Executor process or SDK client could not be created.
    #[error("Process spawn failed: {0}")]
    SpawnFailed(String),

    /// Executor process ran but the request failed during execution.
    #[error("Process execution failed: {0}")]
    ExecutionFailed(String),

    /// Requested session identifier is unknown to the executor backend.
    #[error("Session not found: {0}")]
    SessionNotFound(String),

    /// User-provided or resolved configuration is invalid.
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    /// Executor binary or required runtime dependency is unavailable.
    #[error("Executor unavailable: {0}")]
    Unavailable(String),

    /// JSON serialization or deserialization failed.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Miscellaneous error not covered by the typed variants above.
    #[error("{0}")]
    Other(String),
}

impl ExecutorError {
    /// Returns a stable machine-readable error category.
    pub fn error_type(&self) -> &'static str {
        match self {
            ExecutorError::Io(_) => "io",
            ExecutorError::SpawnFailed(_) => "spawn_failed",
            ExecutorError::ExecutionFailed(_) => "execution_failed",
            ExecutorError::SessionNotFound(_) => "session_not_found",
            ExecutorError::InvalidConfig(_) => "invalid_config",
            ExecutorError::Unavailable(_) => "unavailable",
            ExecutorError::Serialization(_) => "serialization",
            ExecutorError::Other(_) => "other",
        }
    }

    /// Builds [`ExecutorError::SpawnFailed`] with contextual operation details.
    pub fn spawn_failed(context: impl AsRef<str>, error: impl fmt::Display) -> Self {
        Self::SpawnFailed(format_with_context(context.as_ref(), error))
    }

    /// Builds [`ExecutorError::ExecutionFailed`] with contextual operation details.
    pub fn execution_failed(context: impl AsRef<str>, error: impl fmt::Display) -> Self {
        Self::ExecutionFailed(format_with_context(context.as_ref(), error))
    }

    /// Builds [`ExecutorError::InvalidConfig`] with contextual operation details.
    pub fn invalid_config(context: impl AsRef<str>, error: impl fmt::Display) -> Self {
        Self::InvalidConfig(format_with_context(context.as_ref(), error))
    }

    /// Builds [`ExecutorError::Unavailable`] with contextual operation details.
    pub fn unavailable(context: impl AsRef<str>, error: impl fmt::Display) -> Self {
        Self::Unavailable(format_with_context(context.as_ref(), error))
    }

    /// Builds [`ExecutorError::Other`] with contextual operation details.
    pub fn other(context: impl AsRef<str>, error: impl fmt::Display) -> Self {
        Self::Other(format_with_context(context.as_ref(), error))
    }
}

fn format_with_context(context: &str, error: impl fmt::Display) -> String {
    if context.is_empty() {
        error.to_string()
    } else {
        format!("{context}: {error}")
    }
}
