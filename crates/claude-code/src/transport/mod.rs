//! Transport layer abstraction for CLI communication.
//!
//! This module defines the [`Transport`] trait, which abstracts the communication
//! channel between the SDK and the Claude Code CLI process. The default implementation
//! is [`SubprocessCliTransport`](subprocess_cli::SubprocessCliTransport), which spawns
//! the CLI as a child process and communicates via stdin/stdout.
//!
//! Custom transports can be implemented for testing or alternative communication
//! mechanisms by implementing the [`Transport`] trait.

use async_trait::async_trait;
use serde_json::Value;

use crate::errors::Result;

pub mod subprocess_cli;

/// Async transport trait for bidirectional communication with the Claude Code CLI.
///
/// Implementations handle the lifecycle of the communication channel: connecting,
/// writing JSON messages, reading responses, and closing the channel.
///
/// All methods are async and the trait requires `Send` for use in async runtimes.
#[async_trait]
pub trait Transport: Send {
    /// Establishes the transport connection (e.g., spawns the subprocess).
    async fn connect(&mut self) -> Result<()>;

    /// Writes a string (typically a JSON line) to the CLI's input stream.
    async fn write(&mut self, data: &str) -> Result<()>;

    /// Signals that no more input will be sent (closes the input stream).
    async fn end_input(&mut self) -> Result<()>;

    /// Reads the next JSON message from the CLI's output stream.
    ///
    /// Returns `Ok(None)` when the stream is exhausted (EOF).
    async fn read_next_message(&mut self) -> Result<Option<Value>>;

    /// Closes the transport connection and cleans up resources.
    async fn close(&mut self) -> Result<()>;

    /// Returns `true` if the transport is connected and ready for I/O.
    fn is_ready(&self) -> bool;
}
