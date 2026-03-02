//! Transport layer abstraction for CLI communication.
//!
//! This module defines the [`Transport`] trait, which abstracts the communication
//! channel between the SDK and the Claude Code CLI process. The default implementation
//! is [`SubprocessCliTransport`](subprocess_cli::SubprocessCliTransport), which spawns
//! the CLI as a child process and communicates via stdin/stdout.
//!
//! Custom transports can be implemented for testing or alternative communication
//! mechanisms by implementing the [`Transport`] trait.
//!
//! # Split I/O
//!
//! For concurrent read/write scenarios, the transport can be split into independent
//! reader and writer halves via [`Transport::into_split()`]. Implementations can use
//! [`split_with_adapter()`] for a simple lock-based fallback via [`SplitAdapter`].

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::errors::Result;

pub mod subprocess_cli;

/// The result of splitting a transport into independent reader, writer, and close handle.
pub type TransportSplitResult = Result<(
    Box<dyn TransportReader>,
    Box<dyn TransportWriter>,
    Box<dyn TransportCloseHandle>,
)>;

/// Async reader half of a split transport.
///
/// Reads JSON messages from the CLI's output stream independently of writes.
#[async_trait]
pub trait TransportReader: Send {
    /// Reads the next JSON message from the CLI's output stream.
    ///
    /// Returns `Ok(None)` when the stream is exhausted (EOF).
    async fn read_next_message(&mut self) -> Result<Option<Value>>;
}

/// Async writer half of a split transport.
///
/// Writes data to the CLI's input stream independently of reads.
#[async_trait]
pub trait TransportWriter: Send {
    /// Writes a string (typically a JSON line) to the CLI's input stream.
    async fn write(&mut self, data: &str) -> Result<()>;

    /// Signals that no more input will be sent (closes the input stream).
    async fn end_input(&mut self) -> Result<()>;
}

/// Async transport trait for bidirectional communication with the Claude Code CLI.
///
/// Implementations handle the lifecycle of the communication channel: connecting,
/// writing JSON messages, reading responses, and closing the channel.
///
/// All methods are async and the trait requires `Send` for use in async runtimes.
///
/// # Splitting
///
/// After [`connect()`](Transport::connect), call [`into_split()`](Transport::into_split)
/// to obtain independent reader and writer halves for concurrent I/O. Use
/// [`split_with_adapter()`] for a lock-based fallback, or provide a native split
/// for true concurrent I/O.
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

    /// Splits the transport into independent reader and writer halves.
    ///
    /// Consumes the transport. The returned halves can be used concurrently
    /// from different tasks.
    ///
    /// For a simple implementation, delegate to [`split_with_adapter(self)`](split_with_adapter)
    /// which wraps `self` in a [`SplitAdapter`] that serializes access via a mutex.
    /// Transports that can provide true concurrent I/O (like
    /// [`SubprocessCliTransport`](subprocess_cli::SubprocessCliTransport)) should
    /// provide a native split instead.
    ///
    /// # Returns
    ///
    /// A tuple of `(reader, writer, close_handle)`. The `close_handle` should
    /// be used to close the transport and clean up resources when done.
    fn into_split(self: Box<Self>) -> TransportSplitResult;
}

/// Handle for closing a transport after it has been split.
///
/// Holds shared ownership of the transport and provides async cleanup.
#[async_trait]
pub trait TransportCloseHandle: Send + Sync {
    /// Closes the transport and cleans up all resources.
    async fn close(&self) -> Result<()>;
}

/// Factory for creating fresh [`Transport`] instances.
///
/// Used by [`ClaudeSdkClient`](crate::ClaudeSdkClient) to produce a new transport
/// on each [`connect()`](crate::ClaudeSdkClient::connect) call, enabling reconnect
/// after disconnect without consuming the factory.
///
/// # Example
///
/// ```rust,ignore
/// use claude_code::{Transport, TransportFactory, Result};
///
/// struct MyTransportFactory { /* config */ }
///
/// impl TransportFactory for MyTransportFactory {
///     fn create_transport(&self) -> Result<Box<dyn Transport>> {
///         Ok(Box::new(MyTransport::new()))
///     }
/// }
/// ```
pub trait TransportFactory: Send + Sync {
    /// Creates a new transport instance for a new connection session.
    fn create_transport(&self) -> Result<Box<dyn Transport>>;
}

/// Splits a transport using a lock-based adapter.
///
/// This is a convenience function for implementing [`Transport::into_split()`]
/// when a transport doesn't have a natural way to split its I/O. All operations
/// are serialized via a mutex.
///
/// # Example
///
/// ```rust
/// use claude_code::transport::subprocess_cli::{Prompt, SubprocessCliTransport};
/// use claude_code::transport::split_with_adapter;
///
/// let transport = SubprocessCliTransport::new(Prompt::Messages, Default::default()).unwrap();
/// let (_reader, _writer, _close) = split_with_adapter(Box::new(transport)).unwrap();
/// ```
pub fn split_with_adapter(transport: Box<dyn Transport>) -> TransportSplitResult {
    let adapter = SplitAdapter::new(transport);
    Ok((
        Box::new(adapter.reader()),
        Box::new(adapter.writer()),
        Box::new(adapter),
    ))
}

/// Lock-based split adapter for backward-compatible transport splitting.
///
/// Wraps a `Box<dyn Transport>` in `Arc<Mutex<>>` and provides reader/writer
/// halves that serialize access. This is the fallback for transports that
/// choose not to provide a native split.
pub struct SplitAdapter {
    inner: Arc<Mutex<Box<dyn Transport>>>,
}

impl SplitAdapter {
    /// Creates a new `SplitAdapter` wrapping the given transport.
    ///
    /// # Example
    ///
    /// ```rust
    /// use claude_code::transport::subprocess_cli::{Prompt, SubprocessCliTransport};
    /// use claude_code::transport::SplitAdapter;
    ///
    /// let transport = SubprocessCliTransport::new(Prompt::Messages, Default::default()).unwrap();
    /// let _adapter = SplitAdapter::new(Box::new(transport));
    /// ```
    pub fn new(transport: Box<dyn Transport>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(transport)),
        }
    }

    /// Returns a reader half backed by the shared transport.
    ///
    /// # Example
    ///
    /// ```rust
    /// use claude_code::transport::subprocess_cli::{Prompt, SubprocessCliTransport};
    /// use claude_code::transport::SplitAdapter;
    ///
    /// let transport = SubprocessCliTransport::new(Prompt::Messages, Default::default()).unwrap();
    /// let adapter = SplitAdapter::new(Box::new(transport));
    /// let _reader = adapter.reader();
    /// ```
    pub fn reader(&self) -> SplitAdapterReader {
        SplitAdapterReader {
            inner: self.inner.clone(),
        }
    }

    /// Returns a writer half backed by the shared transport.
    ///
    /// # Example
    ///
    /// ```rust
    /// use claude_code::transport::subprocess_cli::{Prompt, SubprocessCliTransport};
    /// use claude_code::transport::SplitAdapter;
    ///
    /// let transport = SubprocessCliTransport::new(Prompt::Messages, Default::default()).unwrap();
    /// let adapter = SplitAdapter::new(Box::new(transport));
    /// let _writer = adapter.writer();
    /// ```
    pub fn writer(&self) -> SplitAdapterWriter {
        SplitAdapterWriter {
            inner: self.inner.clone(),
        }
    }
}

#[async_trait]
impl TransportCloseHandle for SplitAdapter {
    async fn close(&self) -> Result<()> {
        self.inner.lock().await.close().await
    }
}

/// Reader half of a [`SplitAdapter`].
pub struct SplitAdapterReader {
    inner: Arc<Mutex<Box<dyn Transport>>>,
}

#[async_trait]
impl TransportReader for SplitAdapterReader {
    async fn read_next_message(&mut self) -> Result<Option<Value>> {
        self.inner.lock().await.read_next_message().await
    }
}

/// Writer half of a [`SplitAdapter`].
pub struct SplitAdapterWriter {
    inner: Arc<Mutex<Box<dyn Transport>>>,
}

#[async_trait]
impl TransportWriter for SplitAdapterWriter {
    async fn write(&mut self, data: &str) -> Result<()> {
        self.inner.lock().await.write(data).await
    }

    async fn end_input(&mut self) -> Result<()> {
        self.inner.lock().await.end_input().await
    }
}
