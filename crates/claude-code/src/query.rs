//! Core query/session management for Claude Code communication.
//!
//! This module provides the [`Query`] struct, which handles the low-level
//! communication protocol with the Claude Code CLI process, including:
//!
//! - Session initialization and handshake
//! - Control request/response protocol (permissions, hooks, MCP)
//! - Background message reading and routing via tokio tasks
//! - Lifecycle management (interrupt, model change, rewind)
//!
//! Most users should use [`ClaudeSdkClient`](crate::ClaudeSdkClient) or
//! [`query()`](crate::query_fn::query) instead of interacting with this module directly.

use std::collections::HashMap;
use std::future::Future;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use futures::{FutureExt, Stream, StreamExt};
use serde_json::{Map, Value, json};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::errors::{Error, Result};
use crate::message_parser::parse_message;
use crate::sdk_mcp::McpSdkServer;
use crate::transport::{TransportCloseHandle, TransportReader, TransportWriter};
use crate::types::{
    AgentDefinition, CanUseToolCallback, HookCallback, HookMatcher, Message, PermissionResult,
    ToolPermissionContext,
};

/// Channel buffer size for SDK messages (matches Python SDK's buffer=100).
const MESSAGE_CHANNEL_BUFFER: usize = 100;

/// Converts hook callback output keys from Rust-safe names to CLI protocol names.
///
/// Specifically maps `async_` → `async` and `continue_` → `continue`, since
/// those are reserved words in Rust.
fn convert_hook_output_for_cli(output: Value) -> Value {
    let Some(obj) = output.as_object() else {
        return output;
    };

    let mut converted = Map::new();
    for (key, value) in obj {
        match key.as_str() {
            "async_" => {
                converted.insert("async".to_string(), value.clone());
            }
            "continue_" => {
                converted.insert("continue".to_string(), value.clone());
            }
            _ => {
                converted.insert(key.clone(), value.clone());
            }
        }
    }
    Value::Object(converted)
}

fn panic_payload_to_string(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(msg) = payload.downcast_ref::<&str>() {
        (*msg).to_string()
    } else if let Some(msg) = payload.downcast_ref::<String>() {
        msg.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

fn callback_panic_error(callback_type: &str, payload: Box<dyn std::any::Any + Send>) -> Error {
    let panic_message = panic_payload_to_string(payload);
    warn!(
        callback_type,
        panic_message, "Caught panic in callback invocation"
    );
    Error::Other(format!(
        "{callback_type} callback panicked: {panic_message}"
    ))
}

async fn await_callback_with_panic_isolation<T, F>(
    callback_type: &str,
    callback_future: F,
) -> Result<T>
where
    F: Future<Output = Result<T>>,
{
    match AssertUnwindSafe(callback_future).catch_unwind().await {
        Ok(result) => result,
        Err(payload) => Err(callback_panic_error(callback_type, payload)),
    }
}

/// Tracks pending control request senders and early-arrival response buffers.
///
/// Both maps are behind a single mutex to ensure atomicity: a response that
/// arrives before the sender is registered gets buffered, and a later
/// `send_control_request` drains the buffer under the same lock.
struct PendingControlsState {
    senders: HashMap<String, oneshot::Sender<Result<Value>>>,
    buffered: HashMap<String, Result<Value>>,
}

/// Shared state accessible by both the background reader task and the main task.
struct QuerySharedState {
    can_use_tool: Option<CanUseToolCallback>,
    hook_callbacks: Mutex<HashMap<String, HookCallback>>,
    sdk_mcp_servers: HashMap<String, Arc<McpSdkServer>>,
    /// Pending control request/response matching.
    pending_controls: Mutex<PendingControlsState>,
    /// Shared writer for responding to control requests and sending messages.
    writer: Arc<Mutex<Box<dyn TransportWriter>>>,
    /// Whether stdin close is deferred until first result.
    pending_stdin_close: AtomicBool,
    /// Timeout for deferred stdin close.
    stream_close_timeout: Duration,
    /// Whether the background reader task has terminated.
    reader_terminated: AtomicBool,
    /// Reason for reader task termination, if known.
    reader_termination_reason: Mutex<Option<String>>,
}

/// Low-level query session handler for Claude Code CLI communication.
///
/// Manages the bidirectional JSON stream protocol between the SDK and the CLI.
/// On startup, a background tokio task
/// is spawned to continuously read messages from the transport and route them:
///
/// - **Control responses** are delivered to the waiting control-request caller via oneshot channels.
/// - **Control requests** (permissions, hooks, MCP) are handled by the background task.
/// - **SDK messages** (user, assistant, system, result) are parsed and delivered via an mpsc channel.
///
/// This architecture mirrors the Python SDK's task-group model and enables
/// concurrent send and receive operations.
pub struct Query {
    /// Shared state for background task and main task.
    state: Option<Arc<QuerySharedState>>,

    /// Receiver end of the SDK message channel.
    message_rx: Option<mpsc::Receiver<Result<Message>>>,

    /// Handle for the background reader task.
    reader_task: Option<JoinHandle<()>>,

    /// Handle for closing the split transport.
    close_handle: Option<Box<dyn TransportCloseHandle>>,

    /// Monotonically increasing request ID counter.
    request_counter: Arc<AtomicUsize>,

    /// Whether the query is in streaming mode.
    is_streaming_mode: bool,

    /// Agent definitions to register during initialization.
    agents: Option<HashMap<String, AgentDefinition>>,

    /// Whether initialization has completed.
    initialized: bool,

    /// The initialization response from the CLI.
    initialization_result: Option<Value>,

    /// Timeout for the initialization handshake.
    initialize_timeout: Duration,

    /// Whether hooks or SDK MCP servers are present (for deferred stdin close).
    has_hooks_or_mcp: bool,
}

impl Query {
    /// Creates a new `Query` and starts the background reader task.
    ///
    /// This is the primary constructor. It splits the given reader and writer,
    /// registers callbacks, and spawns the background task.
    ///
    /// # Arguments
    ///
    /// * `reader` — The transport reader half.
    /// * `writer` — The transport writer half (wrapped in `Arc<Mutex<>>` for sharing).
    /// * `close_handle` — Handle for closing the transport.
    /// * `is_streaming_mode` — Whether to use the streaming protocol.
    /// * `can_use_tool` — Optional permission callback for tool approval.
    /// * `hook_callbacks` — Hook callbacks keyed by callback ID.
    /// * `sdk_mcp_servers` — In-process MCP servers.
    /// * `agents` — Optional subagent definitions.
    /// * `initialize_timeout` — Timeout for the initialization handshake.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn start(
        reader: Box<dyn TransportReader>,
        writer: Box<dyn TransportWriter>,
        close_handle: Box<dyn TransportCloseHandle>,
        is_streaming_mode: bool,
        can_use_tool: Option<CanUseToolCallback>,
        hook_callbacks: HashMap<String, HookCallback>,
        sdk_mcp_servers: HashMap<String, Arc<McpSdkServer>>,
        agents: Option<HashMap<String, AgentDefinition>>,
        initialize_timeout: Duration,
    ) -> Self {
        let stream_close_timeout_ms: u64 = std::env::var("CLAUDE_CODE_STREAM_CLOSE_TIMEOUT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60_000);
        let stream_close_timeout =
            Duration::from_millis(stream_close_timeout_ms).max(Duration::from_secs(60));

        let has_hooks_or_mcp = !hook_callbacks.is_empty() || !sdk_mcp_servers.is_empty();
        let writer = Arc::new(Mutex::new(writer));

        let state = Arc::new(QuerySharedState {
            can_use_tool,
            hook_callbacks: Mutex::new(hook_callbacks),
            sdk_mcp_servers,
            pending_controls: Mutex::new(PendingControlsState {
                senders: HashMap::new(),
                buffered: HashMap::new(),
            }),
            writer: writer.clone(),
            pending_stdin_close: AtomicBool::new(false),
            stream_close_timeout,
            reader_terminated: AtomicBool::new(false),
            reader_termination_reason: Mutex::new(None),
        });

        let (message_tx, message_rx) = mpsc::channel(MESSAGE_CHANNEL_BUFFER);

        let reader_state = state.clone();
        let reader_task = tokio::spawn(async move {
            background_reader_task(reader, reader_state, message_tx).await;
        });

        Self {
            state: Some(state),
            message_rx: Some(message_rx),
            reader_task: Some(reader_task),
            close_handle: Some(close_handle),
            request_counter: Arc::new(AtomicUsize::new(0)),
            is_streaming_mode,
            agents,
            initialized: false,
            initialization_result: None,
            initialize_timeout,
            has_hooks_or_mcp,
        }
    }

    /// Sends the initialization handshake to the CLI.
    ///
    /// Registers hook callbacks and agent definitions with the CLI process,
    /// and waits for the initialization response.
    ///
    /// Returns the initialization response payload, or `None` if not in streaming mode.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use claude_code::Query;
    /// use serde_json::Map;
    /// use serde_json::Value;
    ///
    /// # async fn demo(query: &mut Query) -> claude_code::Result<()> {
    /// let _ = query.initialize(Map::<String, Value>::new()).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn initialize(&mut self, hooks_config: Map<String, Value>) -> Result<Option<Value>> {
        if !self.is_streaming_mode {
            return Ok(None);
        }

        let mut request = Map::new();
        request.insert(
            "subtype".to_string(),
            Value::String("initialize".to_string()),
        );
        request.insert(
            "hooks".to_string(),
            if hooks_config.is_empty() {
                Value::Null
            } else {
                Value::Object(hooks_config)
            },
        );

        if let Some(agents) = &self.agents {
            request.insert(
                "agents".to_string(),
                serde_json::to_value(agents).unwrap_or(Value::Null),
            );
        }

        let response = self
            .send_control_request(Value::Object(request), self.initialize_timeout)
            .await?;
        self.initialized = true;
        self.initialization_result = Some(response.clone());
        Ok(Some(response))
    }

    /// Returns the initialization result from the CLI handshake.
    ///
    /// Returns `None` if [`initialize()`](Self::initialize) has not been called yet.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use claude_code::Query;
    ///
    /// fn demo(query: &Query) {
    ///     let _info = query.initialization_result();
    /// }
    /// ```
    pub fn initialization_result(&self) -> Option<Value> {
        self.initialization_result.clone()
    }

    /// Sends a control request to the CLI and waits for the matching response.
    ///
    /// The request is written via the shared writer. The background reader task
    /// delivers the matching control response via a oneshot channel.
    async fn send_control_request(&self, request: Value, timeout: Duration) -> Result<Value> {
        if !self.is_streaming_mode {
            return Err(Error::Other(
                "Control requests require streaming mode".to_string(),
            ));
        }

        let state = self
            .state
            .as_ref()
            .ok_or_else(|| Error::Other("Query not started or already closed.".to_string()))?;

        let request_id = format!(
            "req_{}",
            self.request_counter.fetch_add(1, Ordering::SeqCst) + 1
        );

        // Write the control request first (so it's always observable).
        let control_request = json!({
            "type": "control_request",
            "request_id": request_id,
            "request": request,
        });
        state
            .writer
            .lock()
            .await
            .write(&(control_request.to_string() + "\n"))
            .await?;

        // Register a oneshot channel for the response, checking the buffer first.
        // Both operations are under a single lock to avoid a race where a response
        // arrives between checking the buffer and registering the sender.
        let (tx, rx) = oneshot::channel();
        {
            let mut controls = state.pending_controls.lock().await;
            if let Some(result) = controls.buffered.remove(&request_id) {
                return result;
            }
            controls.senders.insert(request_id.clone(), tx);
        }
        if state.reader_terminated.load(Ordering::SeqCst) {
            state
                .pending_controls
                .lock()
                .await
                .senders
                .remove(&request_id);
            let reason = reader_termination_reason(state).await;
            return Err(Error::Other(format!(
                "Background reader task terminated: {reason}"
            )));
        }

        // Wait for the response with timeout.
        let result = tokio::time::timeout(timeout, rx).await;
        match result {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => {
                // Channel closed — background task died
                Err(Error::Other(
                    "Background reader task terminated while waiting for control response"
                        .to_string(),
                ))
            }
            Err(_) => {
                // Timeout
                let subtype = request
                    .get("subtype")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                state
                    .pending_controls
                    .lock()
                    .await
                    .senders
                    .remove(&request_id);
                Err(Error::Other(format!("Control request timeout: {subtype}")))
            }
        }
    }

    /// Sends a user text message to the CLI.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use claude_code::Query;
    ///
    /// # async fn demo(query: &Query) -> claude_code::Result<()> {
    /// query.send_user_message("hello", "default").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_user_message(&self, prompt: &str, session_id: &str) -> Result<()> {
        let message = json!({
            "type": "user",
            "message": {"role": "user", "content": prompt},
            "parent_tool_use_id": Value::Null,
            "session_id": session_id
        });
        self.write_message(&message).await
    }

    /// Sends a raw JSON message to the CLI without any transformation.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use claude_code::Query;
    /// use serde_json::json;
    ///
    /// # async fn demo(query: &Query) -> claude_code::Result<()> {
    /// query.send_raw_message(json!({"type":"user","message":{"role":"user","content":"hi"}})).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_raw_message(&self, message: Value) -> Result<()> {
        self.write_message(&message).await
    }

    /// Writes a JSON message to the shared writer.
    async fn write_message(&self, message: &Value) -> Result<()> {
        let state = self
            .state
            .as_ref()
            .ok_or_else(|| Error::Other("Query not started or already closed.".to_string()))?;
        state
            .writer
            .lock()
            .await
            .write(&(message.to_string() + "\n"))
            .await
    }

    /// Sends multiple input messages to the CLI without closing stdin.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use claude_code::Query;
    /// use serde_json::json;
    ///
    /// # async fn demo(query: &Query) -> claude_code::Result<()> {
    /// query
    ///     .send_input_messages(vec![json!({"type":"user","message":{"role":"user","content":"hello"}})])
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_input_messages(&self, messages: Vec<Value>) -> Result<()> {
        for message in messages {
            self.send_raw_message(message).await?;
        }
        Ok(())
    }

    /// Sends streamed input messages to the CLI without closing stdin.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use claude_code::Query;
    /// use futures::stream;
    /// use serde_json::json;
    ///
    /// # async fn demo(query: &Query) -> claude_code::Result<()> {
    /// query
    ///     .send_input_from_stream(stream::iter(vec![json!({"type":"user","message":{"role":"user","content":"hello"}})]))
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_input_from_stream<S>(&self, mut messages: S) -> Result<()>
    where
        S: Stream<Item = Value> + Unpin,
    {
        while let Some(message) = messages.next().await {
            self.send_raw_message(message).await?;
        }
        Ok(())
    }

    /// Spawns a background task that streams input messages to the CLI.
    ///
    /// This is useful for long-lived or unbounded input streams where the caller
    /// should continue processing messages concurrently.
    ///
    /// The returned task completes when the input stream ends or a write error
    /// occurs. It does not close stdin.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use claude_code::Query;
    /// use futures::stream;
    /// use serde_json::json;
    ///
    /// # async fn demo(query: &Query) -> claude_code::Result<()> {
    /// let handle = query.spawn_input_from_stream(stream::iter(vec![
    ///     json!({"type":"user","message":{"role":"user","content":"hello"}}),
    /// ]))?;
    /// handle.await??;
    /// # Ok(())
    /// # }
    /// ```
    pub fn spawn_input_from_stream<S>(&self, mut messages: S) -> Result<JoinHandle<Result<()>>>
    where
        S: Stream<Item = Value> + Send + Unpin + 'static,
    {
        let state = self
            .state
            .as_ref()
            .cloned()
            .ok_or_else(|| Error::Other("Query not started or already closed.".to_string()))?;

        Ok(tokio::spawn(async move {
            while let Some(message) = messages.next().await {
                state
                    .writer
                    .lock()
                    .await
                    .write(&(message.to_string() + "\n"))
                    .await?;
            }
            Ok(())
        }))
    }

    /// Streams multiple messages to the CLI and closes the input stream.
    ///
    /// If SDK MCP servers or hooks are present, stdin close is deferred until
    /// the first result message is received (or a timeout expires).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use claude_code::Query;
    /// use serde_json::json;
    ///
    /// # async fn demo(query: &Query) -> claude_code::Result<()> {
    /// query
    ///     .stream_input(vec![json!({"type":"user","message":{"role":"user","content":"hello"}})])
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn stream_input(&self, messages: Vec<Value>) -> Result<()> {
        self.send_input_messages(messages).await?;
        self.finalize_stream_input().await
    }

    /// Streams messages from an async stream source and closes the input stream.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use claude_code::Query;
    /// use futures::stream;
    /// use serde_json::json;
    ///
    /// # async fn demo(query: &Query) -> claude_code::Result<()> {
    /// query
    ///     .stream_input_from_stream(stream::iter(vec![json!({"type":"user","message":{"role":"user","content":"hello"}})]))
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn stream_input_from_stream<S>(&self, mut messages: S) -> Result<()>
    where
        S: Stream<Item = Value> + Unpin,
    {
        self.send_input_from_stream(&mut messages).await?;
        self.finalize_stream_input().await
    }

    async fn finalize_stream_input(&self) -> Result<()> {
        let state = self
            .state
            .as_ref()
            .ok_or_else(|| Error::Other("Query not started or already closed.".to_string()))?;

        if self.has_hooks_or_mcp {
            debug!(
                has_hooks_or_mcp = self.has_hooks_or_mcp,
                "Deferring stdin close until first result"
            );
            state.pending_stdin_close.store(true, Ordering::SeqCst);
        } else {
            state.writer.lock().await.end_input().await?;
        }
        Ok(())
    }

    /// Closes the input stream without sending any messages.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use claude_code::Query;
    ///
    /// # async fn demo(query: &Query) -> claude_code::Result<()> {
    /// query.end_input().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn end_input(&self) -> Result<()> {
        let state = self
            .state
            .as_ref()
            .ok_or_else(|| Error::Other("Query not started or already closed.".to_string()))?;
        state.writer.lock().await.end_input().await
    }

    /// Receives the next content message from the CLI.
    ///
    /// Messages are delivered by the background reader task via an mpsc channel.
    /// Control messages are handled transparently by the background task.
    ///
    /// Returns `None` when the stream is exhausted (no more messages).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use claude_code::Query;
    ///
    /// # async fn demo(query: &mut Query) -> claude_code::Result<()> {
    /// while let Some(message) = query.receive_next_message().await? {
    ///     println!("{message:?}");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn receive_next_message(&mut self) -> Result<Option<Message>> {
        let rx = self
            .message_rx
            .as_mut()
            .ok_or_else(|| Error::Other("Query not started or already closed.".to_string()))?;

        match rx.recv().await {
            Some(Ok(message)) => Ok(Some(message)),
            Some(Err(err)) => Err(err),
            None => Ok(None),
        }
    }

    /// Queries the status of connected MCP servers via the CLI.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use claude_code::Query;
    ///
    /// # async fn demo(query: &Query) -> claude_code::Result<()> {
    /// let _status = query.get_mcp_status().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_mcp_status(&self) -> Result<Value> {
        self.send_control_request(json!({ "subtype": "mcp_status" }), Duration::from_secs(60))
            .await
    }

    /// Sends an interrupt signal to the CLI to stop the current operation.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use claude_code::Query;
    ///
    /// # async fn demo(query: &Query) -> claude_code::Result<()> {
    /// query.interrupt().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn interrupt(&self) -> Result<()> {
        self.send_control_request(json!({ "subtype": "interrupt" }), Duration::from_secs(60))
            .await?;
        Ok(())
    }

    /// Changes the permission mode via a control request.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use claude_code::Query;
    ///
    /// # async fn demo(query: &Query) -> claude_code::Result<()> {
    /// query.set_permission_mode("plan").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_permission_mode(&self, mode: &str) -> Result<()> {
        self.send_control_request(
            json!({ "subtype": "set_permission_mode", "mode": mode }),
            Duration::from_secs(60),
        )
        .await?;
        Ok(())
    }

    /// Changes the model used by the CLI via a control request.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use claude_code::Query;
    ///
    /// # async fn demo(query: &Query) -> claude_code::Result<()> {
    /// query.set_model(Some("sonnet")).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_model(&self, model: Option<&str>) -> Result<()> {
        self.send_control_request(
            json!({ "subtype": "set_model", "model": model }),
            Duration::from_secs(60),
        )
        .await?;
        Ok(())
    }

    /// Rewinds file changes to a specific user message checkpoint.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use claude_code::Query;
    ///
    /// # async fn demo(query: &Query) -> claude_code::Result<()> {
    /// query.rewind_files("user-msg-1").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn rewind_files(&self, user_message_id: &str) -> Result<()> {
        self.send_control_request(
            json!({ "subtype": "rewind_files", "user_message_id": user_message_id }),
            Duration::from_secs(60),
        )
        .await?;
        Ok(())
    }

    /// Closes the query session.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use claude_code::Query;
    ///
    /// # async fn demo(query: Query) -> claude_code::Result<()> {
    /// query.close().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn close(mut self) -> Result<()> {
        self.shutdown().await
    }

    /// Internal shutdown logic.
    async fn shutdown(&mut self) -> Result<()> {
        self.message_rx.take();
        self.state.take();

        if let Some(task) = self.reader_task.take() {
            task.abort();
            let _ = task.await;
        }

        if let Some(close_handle) = self.close_handle.take() {
            close_handle.close().await?;
        }

        Ok(())
    }

    /// Takes the message receiver for stream construction.
    pub(crate) fn take_message_receiver(&mut self) -> Option<mpsc::Receiver<Result<Message>>> {
        self.message_rx.take()
    }
}

impl Drop for Query {
    fn drop(&mut self) {
        if let Some(task) = self.reader_task.take() {
            task.abort();
        }

        if let Some(close_handle) = self.close_handle.take() {
            // Spawn a detached task to perform async cleanup.
            // If no runtime is available, fall back to a temporary current-thread
            // runtime for best-effort synchronous cleanup.
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    let _ = close_handle.close().await;
                });
            } else if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                let _ = runtime.block_on(async move { close_handle.close().await });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Background Reader Task
// ---------------------------------------------------------------------------

/// Background task that continuously reads from the transport reader and routes
/// messages to their appropriate destinations.
async fn background_reader_task(
    mut reader: Box<dyn TransportReader>,
    state: Arc<QuerySharedState>,
    message_tx: mpsc::Sender<Result<Message>>,
) {
    loop {
        // Handle deferred stdin close timeout.
        let read_result = if state.pending_stdin_close.load(Ordering::SeqCst) {
            let timeout_dur = state.stream_close_timeout;
            match tokio::time::timeout(timeout_dur, reader.read_next_message()).await {
                Ok(result) => result,
                Err(_) => {
                    debug!("Timed out waiting for first result, closing input stream");
                    try_close_deferred_stdin(&state).await;
                    continue;
                }
            }
        } else {
            reader.read_next_message().await
        };

        let raw = match read_result {
            Ok(Some(raw)) => raw,
            Ok(None) => {
                try_close_deferred_stdin(&state).await;
                break;
            }
            Err(err) => {
                mark_reader_terminated(&state, err.to_string()).await;
                let _ = message_tx.send(Err(err)).await;
                break;
            }
        };

        let msg_type = raw.get("type").and_then(Value::as_str).unwrap_or_default();

        if msg_type == "control_response" {
            handle_control_response(&state, &raw).await;
            continue;
        }

        if msg_type == "control_request" {
            if let Err(err) = handle_control_request(&state, raw).await {
                debug!("Error handling control request: {err}");
            }
            continue;
        }

        if msg_type == "control_cancel_request" {
            continue;
        }

        // Parse and forward SDK messages.
        match parse_message(&raw) {
            Ok(Some(msg)) => {
                if matches!(msg, Message::Result(_))
                    && state.pending_stdin_close.load(Ordering::SeqCst)
                {
                    debug!("Received first result, closing input stream");
                    try_close_deferred_stdin(&state).await;
                }

                if message_tx.send(Ok(msg)).await.is_err() {
                    break;
                }
            }
            Ok(None) => {}
            Err(err) => {
                if message_tx
                    .send(Err(Error::MessageParse(err)))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
    }
}

/// Marks reader termination and fails all pending control requests immediately.
async fn mark_reader_terminated(state: &QuerySharedState, reason: String) {
    state.reader_terminated.store(true, Ordering::SeqCst);
    let stored_reason = {
        let mut termination_reason = state.reader_termination_reason.lock().await;
        if termination_reason.is_none() {
            *termination_reason = Some(reason);
        }
        termination_reason
            .clone()
            .unwrap_or_else(|| "Unknown reason".to_string())
    };

    let mut controls = state.pending_controls.lock().await;
    for (_, sender) in controls.senders.drain() {
        let _ = sender.send(Err(Error::Other(format!(
            "Background reader task terminated: {stored_reason}"
        ))));
    }
}

/// Returns the recorded reader termination reason or a generic fallback.
async fn reader_termination_reason(state: &QuerySharedState) -> String {
    state
        .reader_termination_reason
        .lock()
        .await
        .clone()
        .unwrap_or_else(|| "Unknown reason".to_string())
}

/// Closes deferred stdin via the shared writer.
async fn try_close_deferred_stdin(state: &QuerySharedState) {
    if state
        .pending_stdin_close
        .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        if let Err(e) = state.writer.lock().await.end_input().await {
            debug!("Error closing deferred stdin: {e}");
        }
    }
}

/// Routes a control response to the waiting oneshot sender, or buffers it.
///
/// If no sender is registered for this response's `request_id`, the parsed
/// result is stored in the buffer for later retrieval by `send_control_request`.
async fn handle_control_response(state: &QuerySharedState, raw: &Value) {
    let Some(response) = raw.get("response").and_then(Value::as_object) else {
        return;
    };
    let response_request_id = response
        .get("request_id")
        .and_then(Value::as_str)
        .unwrap_or_default();

    let subtype = response
        .get("subtype")
        .and_then(Value::as_str)
        .unwrap_or_default();

    let result: Result<Value> = if subtype == "error" {
        let error = response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("Unknown error");
        Err(Error::Other(error.to_string()))
    } else {
        Ok(response
            .get("response")
            .cloned()
            .unwrap_or_else(|| json!({})))
    };

    let mut controls = state.pending_controls.lock().await;
    if let Some(sender) = controls.senders.remove(response_request_id) {
        let _ = sender.send(result);
    } else {
        // Response arrived before sender was registered — buffer it.
        controls
            .buffered
            .insert(response_request_id.to_string(), result);
    }
}

async fn handle_can_use_tool_request(
    state: &QuerySharedState,
    request_data: &Map<String, Value>,
) -> Result<Value> {
    let callback = state
        .can_use_tool
        .clone()
        .ok_or_else(|| Error::Other("canUseTool callback is not provided".to_string()))?;
    let tool_name = request_data
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let input = request_data
        .get("input")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let suggestions = request_data
        .get("permission_suggestions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| serde_json::from_value(value).ok())
        .collect();
    let blocked_path = request_data
        .get("blocked_path")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let context = ToolPermissionContext {
        suggestions,
        blocked_path,
        signal: None,
    };

    let callback_future = panic::catch_unwind(AssertUnwindSafe(|| {
        callback(tool_name, input.clone(), context)
    }))
    .map_err(|payload| callback_panic_error("can_use_tool", payload))?;
    let callback_result =
        await_callback_with_panic_isolation("can_use_tool", callback_future).await?;
    let output = match callback_result {
        PermissionResult::Allow(allow) => {
            let mut obj = Map::new();
            obj.insert("behavior".to_string(), Value::String("allow".to_string()));
            obj.insert(
                "updatedInput".to_string(),
                allow.updated_input.unwrap_or(input),
            );
            if let Some(updated_permissions) = allow.updated_permissions {
                let permissions_json: Vec<Value> = updated_permissions
                    .into_iter()
                    .map(|permission| permission.to_cli_dict())
                    .collect();
                obj.insert(
                    "updatedPermissions".to_string(),
                    Value::Array(permissions_json),
                );
            }
            Value::Object(obj)
        }
        PermissionResult::Deny(deny) => {
            let mut obj = Map::new();
            obj.insert("behavior".to_string(), Value::String("deny".to_string()));
            obj.insert("message".to_string(), Value::String(deny.message));
            if deny.interrupt {
                obj.insert("interrupt".to_string(), Value::Bool(true));
            }
            Value::Object(obj)
        }
    };
    Ok(output)
}

async fn handle_hook_callback_request(
    state: &QuerySharedState,
    request_data: &Map<String, Value>,
) -> Result<Value> {
    let callback_id = request_data
        .get("callback_id")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Other("Missing callback_id in hook_callback".to_string()))?;
    let callback = state
        .hook_callbacks
        .lock()
        .await
        .get(callback_id)
        .cloned()
        .ok_or_else(|| Error::Other(format!("No hook callback found for ID: {callback_id}")))?;
    let input = request_data.get("input").cloned().unwrap_or(Value::Null);
    let tool_use_id = request_data
        .get("tool_use_id")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let callback_future = panic::catch_unwind(AssertUnwindSafe(|| {
        callback(input, tool_use_id, Default::default())
    }))
    .map_err(|payload| callback_panic_error("hook", payload))?;
    let output = await_callback_with_panic_isolation("hook", callback_future).await?;
    Ok(convert_hook_output_for_cli(output))
}

async fn handle_mcp_message_request(
    state: &QuerySharedState,
    request_data: &Map<String, Value>,
) -> Result<Value> {
    let server_name = request_data
        .get("server_name")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Other("Missing server_name in mcp_message".to_string()))?;
    let message = request_data
        .get("message")
        .cloned()
        .ok_or_else(|| Error::Other("Missing message in mcp_message".to_string()))?;
    let response = handle_sdk_mcp_request(&state.sdk_mcp_servers, server_name, &message).await;
    Ok(json!({ "mcp_response": response }))
}

/// Handles an incoming control request from the CLI within the background task.
async fn handle_control_request(state: &QuerySharedState, request: Value) -> Result<()> {
    let Some(request_obj) = request.as_object() else {
        return Err(Error::Other("Invalid control request format".to_string()));
    };
    let request_id = request_obj
        .get("request_id")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Other("Missing request_id in control request".to_string()))?
        .to_string();
    let request_data = request_obj
        .get("request")
        .and_then(Value::as_object)
        .ok_or_else(|| Error::Other("Missing request payload".to_string()))?;
    let subtype = request_data
        .get("subtype")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Other("Missing request subtype".to_string()))?;

    let result: Result<Value> = match subtype {
        "can_use_tool" => handle_can_use_tool_request(state, request_data).await,
        "hook_callback" => handle_hook_callback_request(state, request_data).await,
        "mcp_message" => handle_mcp_message_request(state, request_data).await,
        _ => Err(Error::Other(format!(
            "Unsupported control request subtype: {subtype}"
        ))),
    };

    let response_json = match result {
        Ok(payload) => json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": request_id,
                "response": payload
            }
        }),
        Err(err) => json!({
            "type": "control_response",
            "response": {
                "subtype": "error",
                "request_id": request_id,
                "error": err.to_string()
            }
        }),
    };

    state
        .writer
        .lock()
        .await
        .write(&(response_json.to_string() + "\n"))
        .await
}

/// Routes an MCP message to the appropriate in-process SDK MCP server.
///
/// Implements JSON-RPC message routing for in-process SDK MCP servers.
/// Handles `initialize`, `tools/list`, `tools/call`, and `notifications/initialized` methods.
///
/// # Example
///
/// ```rust,no_run
/// use claude_code::{create_sdk_mcp_server, tool};
/// use claude_code::query::handle_sdk_mcp_request;
/// use serde_json::{json, Value};
/// use std::collections::HashMap;
/// use std::sync::Arc;
///
/// # async fn example() {
///     let config = create_sdk_mcp_server(
///     "tools",
///     "1.0.0",
///     vec![tool("echo", "Echo", json!({"type":"object"}), |_args: Value| async move {
///         Ok(json!({"content": []}))
///     })],
///     );
///
///     let mut servers = HashMap::new();
///     servers.insert(config.name.clone(), Arc::clone(&config.instance));
///
///     let response = handle_sdk_mcp_request(
///     &servers,
///     "tools",
///     &json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
///     )
///     .await;
///
///     assert_eq!(response["jsonrpc"], "2.0");
/// # }
/// ```
pub async fn handle_sdk_mcp_request(
    sdk_mcp_servers: &HashMap<String, Arc<McpSdkServer>>,
    server_name: &str,
    message: &Value,
) -> Value {
    let Some(server) = sdk_mcp_servers.get(server_name) else {
        return json!({
            "jsonrpc": "2.0",
            "id": message.get("id").cloned().unwrap_or(Value::Null),
            "error": {
                "code": -32601,
                "message": format!("Server '{server_name}' not found")
            }
        });
    };

    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));

    match method {
        "initialize" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {
                    "name": server.name,
                    "version": server.version
                }
            }
        }),
        "tools/list" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": server.list_tools_json()
            }
        }),
        "tools/call" => {
            let tool_name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let result = server.call_tool_json(tool_name, arguments).await;
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result
            })
        }
        "notifications/initialized" => json!({
            "jsonrpc": "2.0",
            "result": {}
        }),
        _ => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": format!("Method '{method}' not found")
            }
        }),
    }
}

// ---------------------------------------------------------------------------
// Helper: Build hooks config and callbacks
// ---------------------------------------------------------------------------

/// Builds the hooks configuration for the initialization handshake and extracts
/// hook callbacks for the background task.
pub(crate) fn build_hooks_config(
    hooks: &HashMap<String, Vec<HookMatcher>>,
) -> (Map<String, Value>, HashMap<String, HookCallback>) {
    let mut hooks_config = Map::new();
    let mut hook_callbacks = HashMap::new();
    let mut next_callback_id: usize = 0;

    for (event, matchers) in hooks {
        if matchers.is_empty() {
            continue;
        }
        let mut event_matchers = Vec::new();
        for matcher in matchers {
            let mut callback_ids = Vec::new();
            for callback in &matcher.hooks {
                let callback_id = format!("hook_{}", next_callback_id);
                next_callback_id += 1;
                hook_callbacks.insert(callback_id.clone(), callback.clone());
                callback_ids.push(callback_id);
            }

            let mut matcher_obj = Map::new();
            matcher_obj.insert(
                "matcher".to_string(),
                matcher
                    .matcher
                    .as_ref()
                    .map(|m| Value::String(m.clone()))
                    .unwrap_or(Value::Null),
            );
            matcher_obj.insert("hookCallbackIds".to_string(), json!(callback_ids));
            if let Some(timeout) = matcher.timeout {
                matcher_obj.insert("timeout".to_string(), json!(timeout));
            }
            event_matchers.push(Value::Object(matcher_obj));
        }
        hooks_config.insert(event.clone(), Value::Array(event_matchers));
    }

    (hooks_config, hook_callbacks)
}
