//! Internal client for executing single-query sessions.
//!
//! This module provides [`InternalClient`], a stateless helper that manages
//! the full lifecycle of a single query: connect → initialize → send → receive → close.
//!
//! This is used internally by the [`query()`](crate::query_fn::query) convenience function.
//! Most users should use [`query()`](crate::query_fn::query) or [`ClaudeSdkClient`](crate::ClaudeSdkClient)
//! directly.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures::stream::BoxStream;
use futures::{Stream, StreamExt};
use serde_json::Value;
use serde_json::json;

use crate::client::InputPrompt;
use crate::errors::{Error, Result};
use crate::query::{Query, build_hooks_config};
use crate::sdk_mcp::McpSdkServer;
use crate::transport::Transport;
use crate::transport::subprocess_cli::{Prompt as TransportPrompt, SubprocessCliTransport};
use crate::types::{ClaudeAgentOptions, McpServerConfig, McpServersOption, Message};

/// Stateless internal client for executing single-query sessions.
///
/// Unlike [`ClaudeSdkClient`](crate::ClaudeSdkClient), this client does not maintain
/// state between queries. Each call to [`process_query()`](Self::process_query) creates
/// a fresh connection, executes the query, and tears down the connection.
pub struct InternalClient;

impl Default for InternalClient {
    fn default() -> Self {
        Self::new()
    }
}

impl InternalClient {
    /// Creates a new `InternalClient`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use claude_code::internal_client::InternalClient;
    ///
    /// let _client = InternalClient::new();
    /// ```
    pub fn new() -> Self {
        Self
    }

    /// Extracts SDK MCP server instances from the options for in-process routing.
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

    fn configure_options(
        options: ClaudeAgentOptions,
        is_text_prompt: bool,
    ) -> Result<ClaudeAgentOptions> {
        if options.can_use_tool.is_some() && is_text_prompt {
            return Err(Error::Other(
                "can_use_tool callback requires streaming mode. Please provide prompt as messages."
                    .to_string(),
            ));
        }

        if options.can_use_tool.is_some() && options.permission_prompt_tool_name.is_some() {
            return Err(Error::Other(
                "can_use_tool callback cannot be used with permission_prompt_tool_name."
                    .to_string(),
            ));
        }

        let mut configured_options = options;
        if configured_options.can_use_tool.is_some() {
            configured_options.permission_prompt_tool_name = Some("stdio".to_string());
        }
        Ok(configured_options)
    }

    async fn initialize_query(
        &self,
        transport_prompt: TransportPrompt,
        options: ClaudeAgentOptions,
        transport: Option<Box<dyn Transport>>,
    ) -> Result<Query> {
        let mut chosen_transport: Box<dyn Transport> = if let Some(transport) = transport {
            transport
        } else {
            Box::new(SubprocessCliTransport::new(
                transport_prompt,
                options.clone(),
            )?)
        };
        chosen_transport.connect().await?;

        let hooks = options.hooks.clone().unwrap_or_default();
        let sdk_mcp_servers = Self::extract_sdk_mcp_servers(&options);
        let (hooks_config, hook_callbacks) = build_hooks_config(&hooks);

        let (reader, writer, close_handle) = chosen_transport.into_split()?;

        let mut query = Query::start(
            reader,
            writer,
            close_handle,
            true,
            options.can_use_tool.clone(),
            hook_callbacks,
            sdk_mcp_servers,
            options.agents.clone(),
            Duration::from_secs(60),
        );
        query.initialize(hooks_config).await?;
        Ok(query)
    }

    async fn send_prompt(query: &Query, prompt: InputPrompt) -> Result<()> {
        match prompt {
            InputPrompt::Text(text) => {
                query
                    .stream_input(vec![json!({
                        "type": "user",
                        "message": {"role": "user", "content": text},
                        "parent_tool_use_id": Value::Null,
                        "session_id": ""
                    })])
                    .await?;
            }
            InputPrompt::Messages(messages) => {
                query.stream_input(messages).await?;
            }
        }
        Ok(())
    }

    async fn collect_messages(mut query: Query) -> Result<Vec<Message>> {
        let mut messages = Vec::new();
        let read_result: Result<()> = async {
            while let Some(message) = query.receive_next_message().await? {
                messages.push(message);
            }
            Ok(())
        }
        .await;
        let close_result = query.close().await;

        match (read_result, close_result) {
            (Err(err), _) => Err(err),
            (Ok(()), Err(err)) => Err(err),
            (Ok(()), Ok(())) => Ok(messages),
        }
    }

    fn into_message_stream(mut query: Query) -> BoxStream<'static, Result<Message>> {
        let rx = query.take_message_receiver();

        if let Some(rx) = rx {
            // Use the channel receiver directly — this is Send.
            let close_handle_query = query;
            futures::stream::unfold(
                (rx, Some(close_handle_query)),
                |(mut rx, query)| async move {
                    match rx.recv().await {
                        Some(msg) => Some((msg, (rx, query))),
                        None => {
                            // Channel closed — close the query.
                            if let Some(q) = query {
                                let _ = q.close().await;
                            }
                            None
                        }
                    }
                },
            )
            .boxed()
        } else {
            // Fallback: empty stream.
            futures::stream::empty().boxed()
        }
    }

    /// Executes a complete query lifecycle: connect, send, receive all messages, and close.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use claude_code::internal_client::InternalClient;
    /// use claude_code::{InputPrompt, ClaudeAgentOptions};
    ///
    /// # async fn example() -> claude_code::Result<()> {
    /// let client = InternalClient::new();
    /// let _messages = client
    ///     .process_query(
    ///         InputPrompt::Text("hello".to_string()),
    ///         ClaudeAgentOptions::default(),
    ///         None,
    ///     )
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn process_query(
        &self,
        prompt: InputPrompt,
        options: ClaudeAgentOptions,
        transport: Option<Box<dyn Transport>>,
    ) -> Result<Vec<Message>> {
        let configured_options =
            Self::configure_options(options, matches!(prompt, InputPrompt::Text(_)))?;

        let transport_prompt = match &prompt {
            InputPrompt::Text(text) => TransportPrompt::Text(text.clone()),
            InputPrompt::Messages(_) => TransportPrompt::Messages,
        };

        let query = self
            .initialize_query(transport_prompt, configured_options, transport)
            .await?;
        Self::send_prompt(&query, prompt).await?;
        Self::collect_messages(query).await
    }

    /// Executes a one-shot query where input messages are provided as a stream.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use claude_code::internal_client::InternalClient;
    /// use claude_code::ClaudeAgentOptions;
    /// use futures::stream;
    /// use serde_json::json;
    ///
    /// # async fn example() -> claude_code::Result<()> {
    /// let client = InternalClient::new();
    /// let _messages = client
    ///     .process_query_from_stream(
    ///         stream::iter(vec![json!({"type":"user","message":{"role":"user","content":"hello"}})]),
    ///         ClaudeAgentOptions::default(),
    ///         None,
    ///     )
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn process_query_from_stream<S>(
        &self,
        prompt: S,
        options: ClaudeAgentOptions,
        transport: Option<Box<dyn Transport>>,
    ) -> Result<Vec<Message>>
    where
        S: Stream<Item = Value> + Unpin,
    {
        let configured_options = Self::configure_options(options, false)?;
        let query = self
            .initialize_query(TransportPrompt::Messages, configured_options, transport)
            .await?;
        query.stream_input_from_stream(prompt).await?;
        Self::collect_messages(query).await
    }

    /// Executes a one-shot query and returns a streaming response interface.
    ///
    /// The returned stream is `Send` and can be consumed from any tokio task.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use claude_code::internal_client::InternalClient;
    /// use claude_code::{InputPrompt, ClaudeAgentOptions};
    /// use futures::StreamExt;
    ///
    /// # async fn example() -> claude_code::Result<()> {
    /// let client = InternalClient::new();
    /// let mut stream = client
    ///     .process_query_as_stream(
    ///         InputPrompt::Text("hello".to_string()),
    ///         ClaudeAgentOptions::default(),
    ///         None,
    ///     )
    ///     .await?;
    ///
    /// let _ = stream.next().await;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn process_query_as_stream(
        &self,
        prompt: InputPrompt,
        options: ClaudeAgentOptions,
        transport: Option<Box<dyn Transport>>,
    ) -> Result<BoxStream<'static, Result<Message>>> {
        let configured_options =
            Self::configure_options(options, matches!(prompt, InputPrompt::Text(_)))?;
        let transport_prompt = match &prompt {
            InputPrompt::Text(text) => TransportPrompt::Text(text.clone()),
            InputPrompt::Messages(_) => TransportPrompt::Messages,
        };
        let query = self
            .initialize_query(transport_prompt, configured_options, transport)
            .await?;
        Self::send_prompt(&query, prompt).await?;
        Ok(Self::into_message_stream(query))
    }

    /// Executes a one-shot streamed-input query and returns a streaming response interface.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use claude_code::internal_client::InternalClient;
    /// use claude_code::ClaudeAgentOptions;
    /// use futures::{stream, StreamExt};
    /// use serde_json::json;
    ///
    /// # async fn example() -> claude_code::Result<()> {
    /// let client = InternalClient::new();
    /// let mut stream = client
    ///     .process_query_from_stream_as_stream(
    ///         stream::iter(vec![json!({"type":"user","message":{"role":"user","content":"hello"}})]),
    ///         ClaudeAgentOptions::default(),
    ///         None,
    ///     )
    ///     .await?;
    ///
    /// let _ = stream.next().await;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn process_query_from_stream_as_stream<S>(
        &self,
        prompt: S,
        options: ClaudeAgentOptions,
        transport: Option<Box<dyn Transport>>,
    ) -> Result<BoxStream<'static, Result<Message>>>
    where
        S: Stream<Item = Value> + Unpin,
    {
        let configured_options = Self::configure_options(options, false)?;
        let query = self
            .initialize_query(TransportPrompt::Messages, configured_options, transport)
            .await?;
        query.stream_input_from_stream(prompt).await?;
        Ok(Self::into_message_stream(query))
    }
}
