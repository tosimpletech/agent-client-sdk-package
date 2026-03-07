use thiserror::Error;

/// General SDK error for validation and logic failures.
#[derive(Debug, Error, Clone)]
#[error("{message}")]
pub struct OpencodeSDKError {
    /// Human-readable error message.
    pub message: String,
}

impl OpencodeSDKError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Error when the OpenCode CLI executable cannot be found.
#[derive(Debug, Error, Clone)]
#[error("{message}")]
pub struct CLINotFoundError {
    /// Human-readable not-found message.
    pub message: String,
    /// The path that was searched, if a specific path was configured.
    pub cli_path: Option<String>,
}

impl CLINotFoundError {
    pub fn new(message: impl Into<String>, cli_path: Option<String>) -> Self {
        let base = message.into();
        let message = match &cli_path {
            Some(path) => format!("{base}: {path}"),
            None => base,
        };
        Self { message, cli_path }
    }
}

/// Error from OpenCode CLI subprocess execution.
#[derive(Debug, Error, Clone)]
#[error("{message}")]
pub struct ProcessError {
    /// Human-readable process error message.
    pub message: String,
    /// Process exit code when available.
    pub exit_code: Option<i32>,
    /// Captured output snippet from stdout/stderr.
    pub output: Option<String>,
}

impl ProcessError {
    pub fn new(message: impl Into<String>, exit_code: Option<i32>, output: Option<String>) -> Self {
        let base = message.into();
        let mut message = base;
        if let Some(code) = exit_code {
            message = format!("{message} (exit code: {code})");
        }
        if let Some(content) = &output {
            if !content.trim().is_empty() {
                message = format!("{message}\nOutput: {content}");
            }
        }

        Self {
            message,
            exit_code,
            output,
        }
    }
}

/// Error returned by OpenCode HTTP API.
#[derive(Debug, Error, Clone)]
#[error("OpenCode API error: status {status}, body: {body}")]
pub struct ApiError {
    /// HTTP status code.
    pub status: u16,
    /// Raw body text (JSON or plain text).
    pub body: String,
}

/// Unified error type for OpenCode SDK operations.
#[derive(Debug, Error)]
pub enum Error {
    /// General SDK-level validation and logic errors.
    #[error(transparent)]
    OpencodeSDK(#[from] OpencodeSDKError),
    /// OpenCode CLI executable not found.
    #[error(transparent)]
    CLINotFound(#[from] CLINotFoundError),
    /// OpenCode CLI process failure.
    #[error(transparent)]
    Process(#[from] ProcessError),
    /// HTTP API response error.
    #[error(transparent)]
    Api(#[from] ApiError),
    /// Standard I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// JSON serialization/deserialization error.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// HTTP client error.
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    /// Header parsing/encoding error.
    #[error(transparent)]
    InvalidHeaderValue(#[from] reqwest::header::InvalidHeaderValue),
    /// Header name parsing error.
    #[error(transparent)]
    InvalidHeaderName(#[from] reqwest::header::InvalidHeaderName),
    /// Timeout while waiting for server startup.
    #[error("Timeout waiting for server to start after {timeout_ms}ms")]
    ServerStartupTimeout { timeout_ms: u64 },
    /// Missing required path parameter.
    #[error("Missing required path parameter: {0}")]
    MissingPathParameter(String),
    /// Catch-all string error.
    #[error("{0}")]
    Other(String),
}

/// Specialized `Result` for OpenCode SDK.
pub type Result<T> = std::result::Result<T, Error>;
