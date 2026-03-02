//! Core query/session management for Claude Code communication.
//!
//! This module provides the [`Query`] struct, which handles the low-level
//! communication protocol with the Claude Code CLI process, including:
//!
//! - Session initialization and handshake
//! - Control request/response protocol (permissions, hooks, MCP)
//! - Message queuing and parsing
//! - Lifecycle management (interrupt, model change, rewind)
//!
//! Most users should use [`ClaudeSdkClient`](crate::ClaudeSdkClient) or
//! [`query()`](crate::query) instead of interacting with this module directly.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use futures::{Stream, StreamExt};
use serde_json::{Map, Value, json};
use tracing::debug;

use crate::errors::{Error, Result};
use crate::message_parser::parse_message;
use crate::sdk_mcp::McpSdkServer;
use crate::transport::Transport;
use crate::types::{
    AgentDefinition, CanUseToolCallback, HookCallback, HookMatcher, Message, PermissionResult,
    ToolPermissionContext,
};

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

/// Low-level query session handler for Claude Code CLI communication.
///
/// Manages the bidirectional JSON stream protocol between the SDK and the CLI,
/// handling control messages (permissions, hooks, MCP) transparently while
/// exposing content messages to the caller.
///
/// This struct is used internally by [`ClaudeSdkClient`](crate::ClaudeSdkClient)
/// and [`InternalClient`](crate::internal_client::InternalClient). Direct usage is
/// possible but not recommended for most use cases.
pub struct Query {
    transport: Box<dyn Transport>,
    is_streaming_mode: bool,
    can_use_tool: Option<CanUseToolCallback>,
    hooks: HashMap<String, Vec<HookMatcher>>,
    sdk_mcp_servers: HashMap<String, std::sync::Arc<McpSdkServer>>,
    agents: Option<HashMap<String, AgentDefinition>>,
    request_counter: usize,
    next_callback_id: usize,
    hook_callbacks: HashMap<String, HookCallback>,
    queued_messages: VecDeque<Value>,
    initialized: bool,
    initialization_result: Option<Value>,
    initialize_timeout: Duration,
    /// When true, stdin close is deferred until the first result message is received.
    /// This is needed when hooks or SDK MCP servers are present, because the CLI may
    /// send control requests that require writing responses back over stdin.
    pending_stdin_close: bool,
    /// Timeout for waiting for the first result before force-closing stdin.
    stream_close_timeout: Duration,
}

impl Query {
    /// Creates a new `Query` bound to a transport.
    ///
    /// # Arguments
    ///
    /// * `transport` — The transport layer for CLI communication.
    /// * `is_streaming_mode` — Whether to use the streaming protocol (always `true` currently).
    /// * `can_use_tool` — Optional permission callback for tool approval.
    /// * `hooks` — Optional hook matchers keyed by event name.
    /// * `sdk_mcp_servers` — Optional in-process MCP servers.
    /// * `agents` — Optional subagent definitions.
    /// * `initialize_timeout` — Timeout for the initialization handshake.
    pub fn new(
        transport: Box<dyn Transport>,
        is_streaming_mode: bool,
        can_use_tool: Option<CanUseToolCallback>,
        hooks: Option<HashMap<String, Vec<HookMatcher>>>,
        sdk_mcp_servers: Option<HashMap<String, std::sync::Arc<McpSdkServer>>>,
        agents: Option<HashMap<String, AgentDefinition>>,
        initialize_timeout: Duration,
    ) -> Self {
        let stream_close_timeout_ms: u64 = std::env::var("CLAUDE_CODE_STREAM_CLOSE_TIMEOUT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60_000);
        let stream_close_timeout =
            Duration::from_millis(stream_close_timeout_ms).max(Duration::from_secs(60));

        Self {
            transport,
            is_streaming_mode,
            can_use_tool,
            hooks: hooks.unwrap_or_default(),
            sdk_mcp_servers: sdk_mcp_servers.unwrap_or_default(),
            agents,
            request_counter: 0,
            next_callback_id: 0,
            hook_callbacks: HashMap::new(),
            queued_messages: VecDeque::new(),
            initialized: false,
            initialization_result: None,
            initialize_timeout,
            pending_stdin_close: false,
            stream_close_timeout,
        }
    }

    /// Starts the query session (currently a no-op, reserved for future use).
    pub async fn start(&mut self) -> Result<()> {
        Ok(())
    }

    /// Sends the initialization handshake to the CLI.
    ///
    /// Registers hook callbacks and agent definitions with the CLI process,
    /// and waits for the initialization response.
    ///
    /// Returns the initialization response payload, or `None` if not in streaming mode.
    pub async fn initialize(&mut self) -> Result<Option<Value>> {
        if !self.is_streaming_mode {
            return Ok(None);
        }

        let mut hooks_config = Map::new();
        for (event, matchers) in &self.hooks {
            if matchers.is_empty() {
                continue;
            }
            let mut event_matchers = Vec::new();
            for matcher in matchers {
                let mut callback_ids = Vec::new();
                for callback in &matcher.hooks {
                    let callback_id = format!("hook_{}", self.next_callback_id);
                    self.next_callback_id += 1;
                    self.hook_callbacks
                        .insert(callback_id.clone(), callback.clone());
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
    pub fn initialization_result(&self) -> Option<Value> {
        self.initialization_result.clone()
    }

    /// Sends a control response back to the CLI for a pending request.
    async fn send_control_response(
        &mut self,
        request_id: &str,
        subtype: &str,
        payload: Value,
    ) -> Result<()> {
        let response = match subtype {
            "success" => json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": request_id,
                    "response": payload
                }
            }),
            "error" => json!({
                "type": "control_response",
                "response": {
                    "subtype": "error",
                    "request_id": request_id,
                    "error": payload.as_str().unwrap_or("Unknown error")
                }
            }),
            _ => {
                return Err(Error::Other(format!(
                    "Unsupported control response subtype: {subtype}"
                )));
            }
        };

        self.transport.write(&(response.to_string() + "\n")).await
    }

    /// Handles an incoming control request from the CLI.
    ///
    /// Control requests include:
    /// - `can_use_tool` — Tool permission approval via [`CanUseToolCallback`]
    /// - `hook_callback` — Hook function invocation
    /// - `mcp_message` — In-process MCP server message routing
    ///
    /// The response is automatically sent back to the CLI.
    pub async fn handle_control_request(&mut self, request: Value) -> Result<()> {
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
            "can_use_tool" => {
                let callback = self.can_use_tool.clone().ok_or_else(|| {
                    Error::Other("canUseTool callback is not provided".to_string())
                })?;
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
                let context = ToolPermissionContext { suggestions };

                let callback_result = callback(tool_name, input.clone(), context).await?;
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
            "hook_callback" => {
                let callback_id = request_data
                    .get("callback_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        Error::Other("Missing callback_id in hook_callback".to_string())
                    })?;
                let callback = self
                    .hook_callbacks
                    .get(callback_id)
                    .cloned()
                    .ok_or_else(|| {
                        Error::Other(format!("No hook callback found for ID: {callback_id}"))
                    })?;
                let input = request_data.get("input").cloned().unwrap_or(Value::Null);
                let tool_use_id = request_data
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string);
                let output = callback(input, tool_use_id, Default::default()).await?;
                Ok(convert_hook_output_for_cli(output))
            }
            "mcp_message" => {
                let server_name = request_data
                    .get("server_name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        Error::Other("Missing server_name in mcp_message".to_string())
                    })?;
                let message = request_data
                    .get("message")
                    .cloned()
                    .ok_or_else(|| Error::Other("Missing message in mcp_message".to_string()))?;
                let response = self.handle_sdk_mcp_request(server_name, &message).await;
                Ok(json!({ "mcp_response": response }))
            }
            _ => Err(Error::Other(format!(
                "Unsupported control request subtype: {subtype}"
            ))),
        };

        match result {
            Ok(payload) => {
                self.send_control_response(&request_id, "success", payload)
                    .await
            }
            Err(err) => {
                self.send_control_response(&request_id, "error", Value::String(err.to_string()))
                    .await
            }
        }
    }

    /// Sends a control request to the CLI and waits for the matching response.
    ///
    /// While waiting, incoming control requests from the CLI are handled
    /// automatically, and content messages are queued for later retrieval.
    ///
    /// Each individual read from the transport is wrapped in a timeout, so the
    /// method cannot hang indefinitely even if the CLI stops producing output.
    async fn send_control_request(&mut self, request: Value, timeout: Duration) -> Result<Value> {
        if !self.is_streaming_mode {
            return Err(Error::Other(
                "Control requests require streaming mode".to_string(),
            ));
        }

        self.request_counter += 1;
        let request_id = format!("req_{}", self.request_counter);

        let control_request = json!({
            "type": "control_request",
            "request_id": request_id,
            "request": request,
        });
        self.transport
            .write(&(control_request.to_string() + "\n"))
            .await?;

        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                let subtype = request
                    .get("subtype")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                return Err(Error::Other(format!("Control request timeout: {subtype}")));
            }

            let read_result =
                tokio::time::timeout(remaining, self.transport.read_next_message()).await;
            let message = match read_result {
                Ok(Ok(Some(msg))) => msg,
                Ok(Ok(None)) => {
                    return Err(Error::Other(
                        "Transport closed while waiting for control response".to_string(),
                    ));
                }
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    let subtype = request
                        .get("subtype")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    return Err(Error::Other(format!("Control request timeout: {subtype}")));
                }
            };

            let msg_type = message
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();

            if msg_type == "control_response" {
                let Some(response) = message.get("response").and_then(Value::as_object) else {
                    continue;
                };
                let response_request_id = response
                    .get("request_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if response_request_id != request_id {
                    continue;
                }

                let subtype = response
                    .get("subtype")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if subtype == "error" {
                    let error = response
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("Unknown error");
                    return Err(Error::Other(error.to_string()));
                }

                return Ok(response
                    .get("response")
                    .cloned()
                    .unwrap_or_else(|| json!({})));
            }

            if msg_type == "control_request" {
                self.handle_control_request(message).await?;
                continue;
            }

            self.queued_messages.push_back(message);
        }
    }

    /// Routes an MCP message to the appropriate in-process SDK MCP server.
    ///
    /// Handles the JSON-RPC protocol for `initialize`, `tools/list`, `tools/call`,
    /// and `notifications/initialized` methods.
    pub async fn handle_sdk_mcp_request(&self, server_name: &str, message: &Value) -> Value {
        let Some(server) = self.sdk_mcp_servers.get(server_name) else {
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

    /// Sends a user text message to the CLI.
    ///
    /// # Arguments
    ///
    /// * `prompt` — The text content of the user message.
    /// * `session_id` — The session identifier.
    pub async fn send_user_message(&mut self, prompt: &str, session_id: &str) -> Result<()> {
        let message = json!({
            "type": "user",
            "message": {"role": "user", "content": prompt},
            "parent_tool_use_id": Value::Null,
            "session_id": session_id
        });
        self.transport.write(&(message.to_string() + "\n")).await
    }

    /// Sends a raw JSON message to the CLI without any transformation.
    pub async fn send_raw_message(&mut self, message: Value) -> Result<()> {
        self.transport.write(&(message.to_string() + "\n")).await
    }

    /// Streams multiple messages to the CLI and closes the input stream.
    ///
    /// Used for batch-style interactions where all input is provided upfront.
    ///
    /// If SDK MCP servers or hooks are present, stdin close is deferred until
    /// the first result message is received (or a timeout expires). This allows
    /// the CLI to send control requests (hook callbacks, MCP messages) that
    /// require responses to be written back over stdin.
    pub async fn stream_input(&mut self, messages: Vec<Value>) -> Result<()> {
        for message in messages {
            self.send_raw_message(message).await?;
        }

        self.finalize_stream_input().await
    }

    /// Streams messages from an async stream source and closes the input stream.
    ///
    /// This is a Rust-idiomatic equivalent of Python's `AsyncIterable` prompt mode.
    /// Messages are written as they arrive from the stream.
    pub async fn stream_input_from_stream<S>(&mut self, mut messages: S) -> Result<()>
    where
        S: Stream<Item = Value> + Unpin,
    {
        while let Some(message) = messages.next().await {
            self.send_raw_message(message).await?;
        }

        self.finalize_stream_input().await
    }

    async fn finalize_stream_input(&mut self) -> Result<()> {
        let has_hooks = !self.hooks.is_empty();
        let has_sdk_mcp = !self.sdk_mcp_servers.is_empty();

        if has_sdk_mcp || has_hooks {
            debug!(
                sdk_mcp_servers = self.sdk_mcp_servers.len(),
                has_hooks, "Deferring stdin close until first result"
            );
            self.pending_stdin_close = true;
        } else {
            self.transport.end_input().await?;
        }
        Ok(())
    }

    /// Closes the input stream without sending any messages.
    pub async fn end_input(&mut self) -> Result<()> {
        self.transport.end_input().await
    }

    /// Receives the next content message from the CLI.
    ///
    /// Control messages (permission requests, hook callbacks, MCP messages)
    /// are handled automatically and transparently. Only content messages
    /// (user, assistant, system, result, stream_event) are returned.
    ///
    /// If stdin close was deferred (due to hooks/SDK MCP), the input stream
    /// is closed when the first result message is received or on timeout.
    ///
    /// Returns `None` when the stream is exhausted (no more messages).
    pub async fn receive_next_message(&mut self) -> Result<Option<Message>> {
        let deadline = if self.pending_stdin_close {
            Some(Instant::now() + self.stream_close_timeout)
        } else {
            None
        };

        loop {
            let raw = if let Some(message) = self.queued_messages.pop_front() {
                Some(message)
            } else if let Some(deadline) = deadline {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    // Timeout waiting for result — close stdin now
                    debug!("Timed out waiting for first result, closing input stream");
                    self.try_close_deferred_stdin().await;
                    self.transport.read_next_message().await?
                } else {
                    match tokio::time::timeout(remaining, self.transport.read_next_message()).await
                    {
                        Ok(result) => result?,
                        Err(_) => {
                            debug!("Timed out waiting for first result, closing input stream");
                            self.try_close_deferred_stdin().await;
                            continue;
                        }
                    }
                }
            } else {
                self.transport.read_next_message().await?
            };

            let Some(raw) = raw else {
                // Stream ended — ensure stdin is closed
                self.try_close_deferred_stdin().await;
                return Ok(None);
            };

            let msg_type = raw.get("type").and_then(Value::as_str).unwrap_or_default();
            if msg_type == "control_request" {
                self.handle_control_request(raw).await?;
                continue;
            }
            if msg_type == "control_response" || msg_type == "control_cancel_request" {
                continue;
            }

            let parsed = parse_message(&raw)?;
            if let Some(ref msg) = parsed {
                // Close stdin when the first result message arrives
                if matches!(msg, Message::Result(_)) && self.pending_stdin_close {
                    debug!("Received first result, closing input stream");
                    self.try_close_deferred_stdin().await;
                }
                return Ok(parsed);
            }
        }
    }

    /// Closes stdin if a deferred close is pending; resets the flag.
    async fn try_close_deferred_stdin(&mut self) {
        if self.pending_stdin_close {
            self.pending_stdin_close = false;
            if let Err(e) = self.transport.end_input().await {
                debug!("Error closing deferred stdin: {e}");
            }
        }
    }

    /// Queries the status of connected MCP servers via the CLI.
    pub async fn get_mcp_status(&mut self) -> Result<Value> {
        self.send_control_request(json!({ "subtype": "mcp_status" }), Duration::from_secs(60))
            .await
    }

    /// Sends an interrupt signal to the CLI to stop the current operation.
    pub async fn interrupt(&mut self) -> Result<()> {
        self.send_control_request(json!({ "subtype": "interrupt" }), Duration::from_secs(60))
            .await?;
        Ok(())
    }

    /// Changes the permission mode via a control request.
    pub async fn set_permission_mode(&mut self, mode: &str) -> Result<()> {
        self.send_control_request(
            json!({ "subtype": "set_permission_mode", "mode": mode }),
            Duration::from_secs(60),
        )
        .await?;
        Ok(())
    }

    /// Changes the model used by the CLI via a control request.
    pub async fn set_model(&mut self, model: Option<&str>) -> Result<()> {
        self.send_control_request(
            json!({ "subtype": "set_model", "model": model }),
            Duration::from_secs(60),
        )
        .await?;
        Ok(())
    }

    /// Rewinds file changes to a specific user message checkpoint.
    pub async fn rewind_files(&mut self, user_message_id: &str) -> Result<()> {
        self.send_control_request(
            json!({ "subtype": "rewind_files", "user_message_id": user_message_id }),
            Duration::from_secs(60),
        )
        .await?;
        Ok(())
    }

    /// Closes the transport and ends the query session.
    pub async fn close(&mut self) -> Result<()> {
        self.transport.close().await
    }

    /// Closes the query session and returns the underlying transport.
    ///
    /// This allows the transport to be reused for subsequent connections
    /// (e.g., when reconnecting a [`ClaudeSdkClient`](crate::ClaudeSdkClient)
    /// with a custom transport).
    pub async fn close_and_take_transport(mut self) -> Result<Box<dyn Transport>> {
        self.transport.close().await?;
        Ok(self.transport)
    }
}
