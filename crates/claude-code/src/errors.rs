//! Error types for the Claude Code SDK.
//!
//! This module defines all error types that can occur during SDK operations,
//! including connection failures, process errors, JSON parsing errors, and
//! message parsing errors.

use serde_json::Value;
use thiserror::Error;

/// General SDK error for validation and logic failures.
///
/// Used for errors that don't fit into more specific categories, such as
/// invalid configuration or callback errors.
#[derive(Debug, Error, Clone)]
#[error("{message}")]
pub struct ClaudeSDKError {
    /// Human-readable error message.
    pub message: String,
}

impl ClaudeSDKError {
    /// Creates a new `ClaudeSDKError` with the given message.
    ///
    /// # Example
    ///
    /// ```rust
    /// use claude_code::ClaudeSDKError;
    ///
    /// let err = ClaudeSDKError::new("invalid configuration");
    /// assert_eq!(err.to_string(), "invalid configuration");
    /// ```
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Error establishing or maintaining a connection to the Claude Code CLI process.
///
/// Raised when the transport layer cannot connect, the process terminates
/// unexpectedly, or stdin/stdout communication fails.
#[derive(Debug, Error, Clone)]
#[error("{message}")]
pub struct CLIConnectionError {
    /// Human-readable connection error message.
    pub message: String,
}

impl CLIConnectionError {
    /// Creates a new `CLIConnectionError` with the given message.
    ///
    /// # Example
    ///
    /// ```rust
    /// use claude_code::CLIConnectionError;
    ///
    /// let err = CLIConnectionError::new("connection dropped");
    /// assert!(err.to_string().contains("connection dropped"));
    /// ```
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Error when the Claude Code CLI executable cannot be found.
///
/// This error is raised if the CLI is not installed or not in the expected
/// locations. Install Claude Code with `npm install -g @anthropic-ai/claude-code`,
/// or provide a custom path via [`ClaudeAgentOptions::cli_path`](crate::ClaudeAgentOptions::cli_path).
#[derive(Debug, Error, Clone)]
#[error("{message}")]
pub struct CLINotFoundError {
    /// Human-readable not-found message.
    pub message: String,
    /// The path that was searched, if a specific path was configured.
    pub cli_path: Option<String>,
}

impl CLINotFoundError {
    /// Creates a new `CLINotFoundError` with the given message and optional path.
    ///
    /// # Example
    ///
    /// ```rust
    /// use claude_code::CLINotFoundError;
    ///
    /// let err = CLINotFoundError::new("Claude Code not found", Some("/usr/local/bin/claude".to_string()));
    /// assert!(err.to_string().contains("/usr/local/bin/claude"));
    /// ```
    pub fn new(message: impl Into<String>, cli_path: Option<String>) -> Self {
        let base = message.into();
        let message = match &cli_path {
            Some(path) => format!("{base}: {path}"),
            None => base,
        };
        Self { message, cli_path }
    }
}

/// Error from the Claude Code CLI subprocess execution.
///
/// Contains the exit code and stderr output for debugging.
#[derive(Debug, Error, Clone)]
#[error("{message}")]
pub struct ProcessError {
    /// Human-readable process error message (may include code/stderr summary).
    pub message: String,
    /// The process exit code, if available.
    pub exit_code: Option<i32>,
    /// The stderr output from the process, if captured.
    pub stderr: Option<String>,
}

impl ProcessError {
    /// Creates a new `ProcessError` with the given message, exit code, and stderr.
    ///
    /// # Example
    ///
    /// ```rust
    /// use claude_code::ProcessError;
    ///
    /// let err = ProcessError::new("Command failed", Some(1), Some("permission denied".to_string()));
    /// assert!(err.to_string().contains("exit code: 1"));
    /// ```
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

/// Error decoding JSON from the CLI stdout stream.
///
/// Raised when the CLI outputs invalid JSON or when a JSON message exceeds
/// the configured buffer size.
#[derive(Debug, Error, Clone)]
#[error("Failed to decode JSON: {preview}...")]
pub struct CLIJSONDecodeError {
    /// The raw line that failed to parse.
    pub line: String,
    /// The original parsing error description.
    pub original_error: String,
    /// A preview of the raw line (first 100 characters).
    pub preview: String,
}

impl CLIJSONDecodeError {
    /// Creates a new `CLIJSONDecodeError` with the raw line and error description.
    ///
    /// # Example
    ///
    /// ```rust
    /// use claude_code::CLIJSONDecodeError;
    ///
    /// let err = CLIJSONDecodeError::new("{bad json}", "expected value");
    /// assert!(err.preview.contains("{bad json}"));
    /// ```
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

/// Error parsing a JSON message into a typed [`Message`](crate::Message).
///
/// Raised when a message from the CLI is missing required fields or
/// has unexpected structure.
#[derive(Debug, Error, Clone)]
#[error("{message}")]
pub struct MessageParseError {
    /// Human-readable parse failure message.
    pub message: String,
    /// The raw JSON data that failed to parse, if available.
    pub data: Option<Value>,
}

impl MessageParseError {
    /// Creates a new `MessageParseError` with the given message and optional raw data.
    ///
    /// # Example
    ///
    /// ```rust
    /// use claude_code::MessageParseError;
    ///
    /// let err = MessageParseError::new("missing field", None);
    /// assert_eq!(err.to_string(), "missing field");
    /// ```
    pub fn new(message: impl Into<String>, data: Option<Value>) -> Self {
        Self {
            message: message.into(),
            data,
        }
    }
}

/// Unified error type for all SDK operations.
///
/// This enum wraps all specific error types into a single type, enabling
/// use of the `?` operator throughout the SDK.
#[derive(Debug, Error)]
pub enum Error {
    /// General SDK error.
    #[error(transparent)]
    ClaudeSDK(#[from] ClaudeSDKError),
    /// Connection error with the CLI process.
    #[error(transparent)]
    CLIConnection(#[from] CLIConnectionError),
    /// CLI executable not found.
    #[error(transparent)]
    CLINotFound(#[from] CLINotFoundError),
    /// CLI process execution error.
    #[error(transparent)]
    Process(#[from] ProcessError),
    /// JSON decoding error from CLI output.
    #[error(transparent)]
    CLIJSONDecode(#[from] CLIJSONDecodeError),
    /// Message parsing error.
    #[error(transparent)]
    MessageParse(#[from] MessageParseError),
    /// Standard I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// JSON serialization/deserialization error.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// Other errors not covered by specific variants.
    #[error("{0}")]
    Other(String),
}

/// A specialized `Result` type for SDK operations.
pub type Result<T> = std::result::Result<T, Error>;
