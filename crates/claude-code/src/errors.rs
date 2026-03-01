use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error, Clone)]
#[error("{message}")]
pub struct ClaudeSDKError {
    pub message: String,
}

impl ClaudeSDKError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug, Error, Clone)]
#[error("{message}")]
pub struct CLIConnectionError {
    pub message: String,
}

impl CLIConnectionError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug, Error, Clone)]
#[error("{message}")]
pub struct CLINotFoundError {
    pub message: String,
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

#[derive(Debug, Error, Clone)]
#[error("{message}")]
pub struct ProcessError {
    pub message: String,
    pub exit_code: Option<i32>,
    pub stderr: Option<String>,
}

impl ProcessError {
    pub fn new(message: impl Into<String>, exit_code: Option<i32>, stderr: Option<String>) -> Self {
        let base = message.into();
        let mut message = base.clone();
        if let Some(code) = exit_code {
            message = format!("{message} (exit code: {code})");
        }
        if let Some(stderr_text) = &stderr {
            message = format!("{message}\nError output: {stderr_text}");
        }

        Self {
            message,
            exit_code,
            stderr,
        }
    }
}

#[derive(Debug, Error, Clone)]
#[error("Failed to decode JSON: {preview}...")]
pub struct CLIJSONDecodeError {
    pub line: String,
    pub original_error: String,
    pub preview: String,
}

impl CLIJSONDecodeError {
    pub fn new(line: impl Into<String>, original_error: impl Into<String>) -> Self {
        let line = line.into();
        let preview: String = line.chars().take(100).collect();
        Self {
            line,
            original_error: original_error.into(),
            preview,
        }
    }
}

#[derive(Debug, Error, Clone)]
#[error("{message}")]
pub struct MessageParseError {
    pub message: String,
    pub data: Option<Value>,
}

impl MessageParseError {
    pub fn new(message: impl Into<String>, data: Option<Value>) -> Self {
        Self {
            message: message.into(),
            data,
        }
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    ClaudeSDK(#[from] ClaudeSDKError),
    #[error(transparent)]
    CLIConnection(#[from] CLIConnectionError),
    #[error(transparent)]
    CLINotFound(#[from] CLINotFoundError),
    #[error(transparent)]
    Process(#[from] ProcessError),
    #[error(transparent)]
    CLIJSONDecode(#[from] CLIJSONDecodeError),
    #[error(transparent)]
    MessageParse(#[from] MessageParseError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
