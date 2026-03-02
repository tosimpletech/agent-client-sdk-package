//! Session-based client for multi-turn interactions with Claude Code.
//!
//! This module provides [`ClaudeSdkClient`], which maintains a persistent session
//! for multi-turn conversations. Use this when you need to send follow-up queries,
//! interrupt operations, or manage the session lifecycle manually.
//!
//! For one-off queries without session management, see [`query()`](crate::query).

use std::collections::HashMap;
use std::time::Duration;

use futures::{Stream, StreamExt};
use serde_json::Value;

use crate::errors::{CLIConnectionError, Error, Result};
use crate::query::Query;
use crate::transport::Transport;
use crate::transport::subprocess_cli::{Prompt as TransportPrompt, SubprocessCliTransport};
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
    custom_transport: Option<Box<dyn Transport>>,
    has_custom_transport: bool,
    query: Option<Query>,
}

impl ClaudeSdkClient {
    /// Creates a new `ClaudeSdkClient` with optional configuration and transport.
    ///
    /// # Arguments
    ///
    /// * `options` — Optional [`ClaudeAgentOptions`] for configuring the session.
    ///   If `None`, defaults are used.
    /// * `transport` — Optional custom [`Transport`] implementation. If `None`,
    ///   the default [`SubprocessCliTransport`] is used.
    pub fn new(options: Option<ClaudeAgentOptions>, transport: Option<Box<dyn Transport>>) -> Self {
        let has_custom_transport = transport.is_some();
        Self {
            options: options.unwrap_or_default(),
            custom_transport: transport,
            has_custom_transport,
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

    fn extract_sdk_mcp_servers(
        options: &ClaudeAgentOptions,
    ) -> HashMap<String, std::sync::Arc<crate::sdk_mcp::McpSdkServer>> {
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

        let mut transport: Box<dyn Transport> = if let Some(custom) = self.custom_transport.take() {
            custom
        } else {
            Box::new(SubprocessCliTransport::new(
                transport_prompt,
                configured_options.clone(),
            )?)
        };
        transport.connect().await?;

        let mut query = Query::new(
            transport,
            true,
            configured_options.can_use_tool.clone(),
            configured_options.hooks.clone(),
            Some(Self::extract_sdk_mcp_servers(&configured_options)),
            configured_options.agents.clone(),
            Self::initialize_timeout(),
        );
        query.start().await?;
        query.initialize().await?;

        if let Some(InputPrompt::Messages(messages)) = prompt {
            query.send_input_messages(messages).await?;
        }

        self.query = Some(query);
        Ok(())
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
    /// * `session_id` — Session identifier for the query. Used to associate
    ///   messages with a specific conversation thread.
    ///
    /// # Errors
    ///
    /// Returns [`CLIConnectionError`] if not connected.
    pub async fn query(&mut self, prompt: InputPrompt, session_id: &str) -> Result<()> {
        let query = self.query.as_mut().ok_or_else(|| {
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
    /// Each streamed message is sent as it arrives. If a message object does not
    /// include `session_id`, this method injects the provided `session_id`.
    ///
    /// # Errors
    ///
    /// Returns [`CLIConnectionError`] if not connected.
    pub async fn query_stream<S>(&mut self, prompt: S, session_id: &str) -> Result<()>
    where
        S: Stream<Item = Value> + Unpin,
    {
        let query = self.query.as_mut().ok_or_else(|| {
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
    /// Returns `None` when no more messages are available (the stream has ended).
    /// Use in a loop to process messages one at a time, or use
    /// [`receive_response()`](Self::receive_response) to collect all messages at once.
    ///
    /// This method automatically handles control requests (permission prompts, hooks,
    /// MCP calls) internally, only returning content messages to the caller.
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
    /// Collects messages into a `Vec`, stopping when a result message is encountered.
    /// The result message is included as the last element.
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
    /// Sends an interrupt signal to the CLI, causing it to stop the current
    /// generation and return a result.
    ///
    /// # Errors
    ///
    /// Returns [`CLIConnectionError`] if not connected.
    pub async fn interrupt(&mut self) -> Result<()> {
        let query = self.query.as_mut().ok_or_else(|| {
            Error::CLIConnection(CLIConnectionError::new(
                "Not connected. Call connect() first.",
            ))
        })?;
        query.interrupt().await
    }

    /// Changes the permission mode for the current session.
    ///
    /// # Arguments
    ///
    /// * `mode` — The permission mode string (e.g., `"default"`, `"acceptEdits"`,
    ///   `"plan"`, `"bypassPermissions"`).
    ///
    /// # Errors
    ///
    /// Returns [`CLIConnectionError`] if not connected.
    pub async fn set_permission_mode(&mut self, mode: &str) -> Result<()> {
        let query = self.query.as_mut().ok_or_else(|| {
            Error::CLIConnection(CLIConnectionError::new(
                "Not connected. Call connect() first.",
            ))
        })?;
        query.set_permission_mode(mode).await
    }

    /// Changes the model used for the current session.
    ///
    /// # Arguments
    ///
    /// * `model` — The model identifier to switch to, or `None` to use the default.
    ///
    /// # Errors
    ///
    /// Returns [`CLIConnectionError`] if not connected.
    pub async fn set_model(&mut self, model: Option<&str>) -> Result<()> {
        let query = self.query.as_mut().ok_or_else(|| {
            Error::CLIConnection(CLIConnectionError::new(
                "Not connected. Call connect() first.",
            ))
        })?;
        query.set_model(model).await
    }

    /// Rewinds file changes to a specific user message checkpoint.
    ///
    /// Requires `enable_file_checkpointing` to be set in [`ClaudeAgentOptions`].
    ///
    /// # Arguments
    ///
    /// * `user_message_id` — The UUID of the user message to rewind to.
    ///
    /// # Errors
    ///
    /// Returns [`CLIConnectionError`] if not connected.
    pub async fn rewind_files(&mut self, user_message_id: &str) -> Result<()> {
        let query = self.query.as_mut().ok_or_else(|| {
            Error::CLIConnection(CLIConnectionError::new(
                "Not connected. Call connect() first.",
            ))
        })?;
        query.rewind_files(user_message_id).await
    }

    /// Queries the status of connected MCP servers.
    ///
    /// Returns a JSON value with the current status of all configured MCP servers.
    ///
    /// # Errors
    ///
    /// Returns [`CLIConnectionError`] if not connected.
    pub async fn get_mcp_status(&mut self) -> Result<Value> {
        let query = self.query.as_mut().ok_or_else(|| {
            Error::CLIConnection(CLIConnectionError::new(
                "Not connected. Call connect() first.",
            ))
        })?;
        query.get_mcp_status().await
    }

    /// Returns the server initialization response, if available.
    ///
    /// The initialization result contains server capabilities and configuration
    /// details returned during the handshake.
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
    /// If a custom transport was provided, it is recovered and can be reused on
    /// the next [`connect()`](Self::connect) call.
    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(query) = self.query.take() {
            let transport = query.close_and_take_transport().await?;
            // Recover the custom transport so it can be reused on reconnect.
            if self.has_custom_transport {
                self.custom_transport = Some(transport);
            }
        }
        Ok(())
    }
}
