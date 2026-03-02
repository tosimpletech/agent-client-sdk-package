use thiserror::Error;

/// A specialized `Result` type for Codex SDK operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Unified error type for all Codex SDK operations.
#[derive(Debug, Error)]
pub enum Error {
    #[error("Codex CLI not found: {0}")]
    CliNotFound(String),

    #[error("Failed to spawn Codex CLI: {0}")]
    Spawn(String),

    #[error("Codex process exited with {detail}: {stderr}")]
    Process {
        detail: String,
        stderr: String,
        code: Option<i32>,
    },

    #[error("Failed to parse JSON event: {0}")]
    JsonParse(String),

    #[error("Thread run failed: {0}")]
    ThreadRun(String),

    #[error("Invalid output schema: {0}")]
    InvalidOutputSchema(String),

    #[error("Invalid config override: {0}")]
    InvalidConfig(String),

    #[error("Turn cancelled")]
    Cancelled,

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
