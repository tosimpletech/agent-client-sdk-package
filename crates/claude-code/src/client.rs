//! Session-based client for multi-turn interactions with Claude Code.
//!
//! This module provides [`ClaudeSdkClient`], which maintains a persistent session
//! for multi-turn conversations. Use this when you need to send follow-up queries,
//! interrupt operations, or manage the session lifecycle manually.
//!
//! For one-off queries without session management, see [`query()`](crate::query).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures::{Stream, StreamExt};
use serde_json::Value;

use crate::errors::{CLIConnectionError, Error, Result};
use crate::query::{Query, build_hooks_config};
use crate::sdk_mcp::McpSdkServer;
use crate::transport::subprocess_cli::{Prompt as TransportPrompt, SubprocessCliTransport};
use crate::transport::{Transport, TransportFactory};
use crate::types::{ClaudeAgentOptions, McpServerConfig, McpServersOption, Message};

/// Input prompt for a query — either plain text or structured messages.
///
/// # Variants
///
/// - `Text` — A simple text prompt string.
/// - `Messages` — A list of structured JSON messages for fine-grained control
///   over the conversation input. Required when using [`can_use_tool`](crate::ClaudeAgentOptions::can_use_tool)
///   callbacks.
#[derive(Debug, Clone, PartialEq)]
pub enum InputPrompt {
    Text(String),
    Messages(Vec<Value>),
}

/// Session-based client for multi-turn Claude Code interactions.
///
/// `ClaudeSdkClient` maintains a connection to the Claude Code CLI subprocess,
/// allowing multiple queries within the same conversation context. The session
/// preserves conversation history across calls.
///
/// # Lifecycle
///
/// 1. Create a client with [`new()`](Self::new)
/// 2. Call [`connect()`](Self::connect) to start the session
/// 3. Send queries with [`query()`](Self::query) and receive responses with
///    [`receive_message()`](Self::receive_message) or [`receive_response()`](Self::receive_response)
/// 4. Call [`disconnect()`](Self::disconnect) when done
///
/// # Concurrency
///
/// After connection, [`query()`](Self::query), [`interrupt()`](Self::interrupt),
/// and control methods take `&self`, allowing concurrent operations from different
/// tasks. Only [`connect()`](Self::connect), [`disconnect()`](Self::disconnect),
/// and [`receive_message()`](Self::receive_message) require `&mut self`.
///
/// # Example
///
/// ```rust,no_run
/// # use claude_code::{ClaudeSdkClient, InputPrompt, Message};
/// # async fn example() -> claude_code::Result<()> {
/// let mut client = ClaudeSdkClient::new(None, None);
/// client.connect(None).await?;
///
/// client.query(InputPrompt::Text("Hello!".into()), "session-1").await?;
/// let messages = client.receive_response().await?;
///
/// client.disconnect().await?;
/// # Ok(())
/// # }
/// ```
pub struct ClaudeSdkClient {
    options: ClaudeAgentOptions,
    transport_factory: Option<Box<dyn TransportFactory>>,
    query: Option<Query>,
}

/// Adapter that wraps a single pre-built transport instance as a one-shot factory.
struct SingleUseTransportFactory(std::sync::Mutex<Option<Box<dyn Transport>>>);

impl TransportFactory for SingleUseTransportFactory {
    fn create_transport(&self) -> Result<Box<dyn Transport>> {
        self.0
            .lock()
            .map_err(|_| Error::Other("Transport factory lock poisoned".to_string()))?
            .take()
            .ok_or_else(|| {
                Error::Other(
                    "Single-use transport already consumed. Use a TransportFactory for reconnect support."
                        .to_string(),
                )
            })
    }
}

impl ClaudeSdkClient {
    /// Creates a new `ClaudeSdkClient` with optional configuration and transport factory.
    ///
    /// # Arguments
    ///
    /// * `options` — Optional [`ClaudeAgentOptions`] for configuring the session.
    ///   If `None`, defaults are used.
    /// * `transport_factory` — Optional [`TransportFactory`] for creating transport
    ///   instances on each [`connect()`](Self::connect) call. If `None`, the default
    ///   [`SubprocessCliTransport`] is used. Using a factory enables reconnect after
    ///   disconnect with the same client instance.
    pub fn new(
        options: Option<ClaudeAgentOptions>,
        transport_factory: Option<Box<dyn TransportFactory>>,
    ) -> Self {
        Self {
            options: options.unwrap_or_default(),
            transport_factory,
            query: None,
        }
    }

    /// Creates a new `ClaudeSdkClient` with a single-use custom transport.
    ///
    /// The transport is consumed on the first [`connect()`](Self::connect). Subsequent
    /// `connect()` calls after [`disconnect()`](Self::disconnect) will return an error.
    /// For reconnect support with custom transports, use [`new()`](Self::new) with a
    /// [`TransportFactory`].
    pub fn new_with_transport(
        options: Option<ClaudeAgentOptions>,
        transport: Box<dyn Transport>,
    ) -> Self {
        Self {
            options: options.unwrap_or_default(),
            transport_factory: Some(Box::new(SingleUseTransportFactory(std::sync::Mutex::new(
                Some(transport),
            )))),
            query: None,
        }
    }

    fn initialize_timeout() -> Duration {
        let timeout_ms = std::env::var("CLAUDE_CODE_STREAM_CLOSE_TIMEOUT")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(60_000);
        Duration::from_secs_f64((timeout_ms as f64 / 1000.0).max(60.0))
    }

    fn extract_sdk_mcp_servers(options: &ClaudeAgentOptions) -> HashMap<String, Arc<McpSdkServer>> {
        let mut servers = HashMap::new();
        if let McpServersOption::Servers(configs) = &options.mcp_servers {
            for (name, config) in configs {
                if let McpServerConfig::Sdk(sdk_config) = config {
                    servers.insert(name.clone(), sdk_config.instance.clone());
                }
            }
        }
        servers
    }

    /// Establishes a connection to the Claude Code CLI and starts the session.
    ///
    /// If an existing connection exists, it is disconnected first.
    ///
    /// # Arguments
    ///
    /// * `prompt` — Optional initial prompt to send upon connection. When using
    ///   `can_use_tool`, this must be [`InputPrompt::Messages`], not `Text`.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The CLI executable is not found
    /// - `can_use_tool` is set with a `Text` prompt (requires `Messages`)
    /// - `can_use_tool` is set alongside `permission_prompt_tool_name`
    /// - The subprocess fails to start
    pub async fn connect(&mut self, prompt: Option<InputPrompt>) -> Result<()> {
        if self.query.is_some() {
            self.disconnect().await?;
        }

        if self.options.can_use_tool.is_some() {
            if matches!(prompt, Some(InputPrompt::Text(_))) {
                return Err(Error::Other(
                    "can_use_tool callback requires streaming mode. Please provide prompt as messages."
                        .to_string(),
                ));
            }
            if self.options.permission_prompt_tool_name.is_some() {
                return Err(Error::Other(
                    "can_use_tool callback cannot be used with permission_prompt_tool_name."
                        .to_string(),
                ));
            }
        }

        let mut configured_options = self.options.clone();
        if configured_options.can_use_tool.is_some() {
            configured_options.permission_prompt_tool_name = Some("stdio".to_string());
        }

        let transport_prompt = match &prompt {
            Some(InputPrompt::Text(text)) => TransportPrompt::Text(text.clone()),
            _ => TransportPrompt::Messages,
        };

        let mut transport: Box<dyn Transport> = if let Some(factory) = &self.transport_factory {
            factory.create_transport()?
        } else {
            Box::new(SubprocessCliTransport::new(
                transport_prompt,
                configured_options.clone(),
            )?)
        };
        transport.connect().await?;

        let hooks = configured_options.hooks.clone().unwrap_or_default();
        let sdk_mcp_servers = Self::extract_sdk_mcp_servers(&configured_options);
        let (hooks_config, hook_callbacks) = build_hooks_config(&hooks);

        let (reader, writer, close_handle) = transport.into_split()?;

        let mut query = Query::start(
            reader,
            writer,
            close_handle,
            true,
            configured_options.can_use_tool.clone(),
            hook_callbacks,
            sdk_mcp_servers,
            configured_options.agents.clone(),
            Self::initialize_timeout(),
        );
        query.initialize(hooks_config).await?;

        if let Some(InputPrompt::Messages(messages)) = prompt {
            query.send_input_messages(messages).await?;
        }

        self.query = Some(query);
        Ok(())
    }

    /// Establishes a connection and sends initial prompt messages from a stream.
    ///
    /// This is a Rust-idiomatic equivalent of Python SDK `connect(AsyncIterable)`.
    /// Unlike one-off query streaming helpers, this keeps stdin open so the session
    /// can continue with follow-up [`query()`](Self::query) calls.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`connect()`](Self::connect), plus any write
    /// errors while streaming the initial messages.
    pub async fn connect_with_messages<S>(&mut self, prompt: S) -> Result<()>
    where
        S: Stream<Item = Value> + Unpin,
    {
        self.connect(None).await?;

        let query = self.query.as_ref().ok_or_else(|| {
            Error::CLIConnection(CLIConnectionError::new(
                "Not connected. Call connect() first.",
            ))
        })?;

        query.send_input_from_stream(prompt).await
    }

    /// Sends a query within the current session.
    ///
    /// The session must be connected first via [`connect()`](Self::connect).
    /// After sending, use [`receive_message()`](Self::receive_message) or
    /// [`receive_response()`](Self::receive_response) to get the response.
    ///
    /// # Arguments
    ///
    /// * `prompt` — The prompt to send (text or structured messages).
    /// * `session_id` — Session identifier for the query.
    ///
    /// # Errors
    ///
    /// Returns [`CLIConnectionError`] if not connected.
    pub async fn query(&self, prompt: InputPrompt, session_id: &str) -> Result<()> {
        let query = self.query.as_ref().ok_or_else(|| {
            Error::CLIConnection(CLIConnectionError::new(
                "Not connected. Call connect() first.",
            ))
        })?;

        match prompt {
            InputPrompt::Text(text) => {
                query.send_user_message(&text, session_id).await?;
            }
            InputPrompt::Messages(messages) => {
                for mut message in messages {
                    if let Value::Object(ref mut obj) = message
                        && !obj.contains_key("session_id")
                    {
                        obj.insert(
                            "session_id".to_string(),
                            Value::String(session_id.to_string()),
                        );
                    }
                    query.send_raw_message(message).await?;
                }
            }
        }

        Ok(())
    }

    /// Streams JSON message prompts within the current session.
    ///
    /// # Errors
    ///
    /// Returns [`CLIConnectionError`] if not connected.
    pub async fn query_stream<S>(&self, prompt: S, session_id: &str) -> Result<()>
    where
        S: Stream<Item = Value> + Unpin,
    {
        let query = self.query.as_ref().ok_or_else(|| {
            Error::CLIConnection(CLIConnectionError::new(
                "Not connected. Call connect() first.",
            ))
        })?;

        let session_id = session_id.to_string();
        let mapped = prompt.map(move |mut message| {
            if let Value::Object(ref mut obj) = message
                && !obj.contains_key("session_id")
            {
                obj.insert("session_id".to_string(), Value::String(session_id.clone()));
            }
            message
        });
        query.send_input_from_stream(mapped).await
    }

    /// Receives a single message from the current query.
    ///
    /// Returns `None` when no more messages are available.
    ///
    /// # Errors
    ///
    /// Returns [`CLIConnectionError`] if not connected.
    pub async fn receive_message(&mut self) -> Result<Option<Message>> {
        let query = self.query.as_mut().ok_or_else(|| {
            Error::CLIConnection(CLIConnectionError::new(
                "Not connected. Call connect() first.",
            ))
        })?;
        query.receive_next_message().await
    }

    /// Receives all messages for the current query until a [`Message::Result`] is received.
    ///
    /// # Errors
    ///
    /// Returns [`CLIConnectionError`] if not connected.
    pub async fn receive_response(&mut self) -> Result<Vec<Message>> {
        let mut messages = Vec::new();
        while let Some(message) = self.receive_message().await? {
            let is_result = matches!(message, Message::Result(_));
            messages.push(message);
            if is_result {
                break;
            }
        }
        Ok(messages)
    }

    /// Interrupts the current operation.
    ///
    /// # Errors
    ///
    /// Returns [`CLIConnectionError`] if not connected.
    pub async fn interrupt(&self) -> Result<()> {
        let query = self.query.as_ref().ok_or_else(|| {
            Error::CLIConnection(CLIConnectionError::new(
                "Not connected. Call connect() first.",
            ))
        })?;
        query.interrupt().await
    }

    /// Changes the permission mode for the current session.
    ///
    /// # Errors
    ///
    /// Returns [`CLIConnectionError`] if not connected.
    pub async fn set_permission_mode(&self, mode: &str) -> Result<()> {
        let query = self.query.as_ref().ok_or_else(|| {
            Error::CLIConnection(CLIConnectionError::new(
                "Not connected. Call connect() first.",
            ))
        })?;
        query.set_permission_mode(mode).await
    }

    /// Changes the model used for the current session.
    ///
    /// # Errors
    ///
    /// Returns [`CLIConnectionError`] if not connected.
    pub async fn set_model(&self, model: Option<&str>) -> Result<()> {
        let query = self.query.as_ref().ok_or_else(|| {
            Error::CLIConnection(CLIConnectionError::new(
                "Not connected. Call connect() first.",
            ))
        })?;
        query.set_model(model).await
    }

    /// Rewinds file changes to a specific user message checkpoint.
    ///
    /// # Errors
    ///
    /// Returns [`CLIConnectionError`] if not connected.
    pub async fn rewind_files(&self, user_message_id: &str) -> Result<()> {
        let query = self.query.as_ref().ok_or_else(|| {
            Error::CLIConnection(CLIConnectionError::new(
                "Not connected. Call connect() first.",
            ))
        })?;
        query.rewind_files(user_message_id).await
    }

    /// Queries the status of connected MCP servers.
    ///
    /// # Errors
    ///
    /// Returns [`CLIConnectionError`] if not connected.
    pub async fn get_mcp_status(&self) -> Result<Value> {
        let query = self.query.as_ref().ok_or_else(|| {
            Error::CLIConnection(CLIConnectionError::new(
                "Not connected. Call connect() first.",
            ))
        })?;
        query.get_mcp_status().await
    }

    /// Returns the server initialization response, if available.
    ///
    /// # Errors
    ///
    /// Returns [`CLIConnectionError`] if not connected.
    pub fn get_server_info(&self) -> Result<Option<Value>> {
        let query = self.query.as_ref().ok_or_else(|| {
            Error::CLIConnection(CLIConnectionError::new(
                "Not connected. Call connect() first.",
            ))
        })?;
        Ok(query.initialization_result())
    }

    /// Disconnects from the Claude Code CLI and closes the session.
    ///
    /// After disconnecting, the client can be reconnected with [`connect()`](Self::connect).
    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(query) = self.query.take() {
            query.close().await?;
        }
        Ok(())
    }
}
