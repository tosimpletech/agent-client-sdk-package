//! Internal client for executing single-query sessions.
//!
//! This module provides [`InternalClient`], a stateless helper that manages
//! the full lifecycle of a single query: connect → initialize → send → receive → close.
//!
//! This is used internally by the [`query()`](crate::query) convenience function.
//! Most users should use [`query()`](crate::query) or [`ClaudeSdkClient`](crate::ClaudeSdkClient)
//! directly.

use std::collections::HashMap;
use std::time::Duration;

use futures::stream::LocalBoxStream;
use futures::{Stream, StreamExt};
use serde_json::Value;
use serde_json::json;

use crate::client::InputPrompt;
use crate::errors::{Error, Result};
use crate::query::Query;
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
    pub fn new() -> Self {
        Self
    }

    /// Extracts SDK MCP server instances from the options for in-process routing.
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

        let mut query = Query::new(
            chosen_transport,
            true,
            options.can_use_tool.clone(),
            options.hooks.clone(),
            Some(Self::extract_sdk_mcp_servers(&options)),
            options.agents.clone(),
            Duration::from_secs(60),
        );
        query.start().await?;
        query.initialize().await?;
        Ok(query)
    }

    async fn send_prompt(query: &mut Query, prompt: InputPrompt) -> Result<()> {
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
        while let Some(message) = query.receive_next_message().await? {
            messages.push(message);
        }
        query.close().await?;
        Ok(messages)
    }

    fn into_message_stream(query: Query) -> LocalBoxStream<'static, Result<Message>> {
        struct QueryStreamState {
            query: Query,
            done: bool,
        }

        futures::stream::try_unfold(
            QueryStreamState { query, done: false },
            |mut state| async move {
                if state.done {
                    return Ok(None);
                }

                match state.query.receive_next_message().await {
                    Ok(Some(message)) => Ok(Some((message, state))),
                    Ok(None) => {
                        state.done = true;
                        state.query.close().await?;
                        Ok(None)
                    }
                    Err(err) => {
                        let _ = state.query.close().await;
                        Err(err)
                    }
                }
            },
        )
        .boxed_local()
    }

    /// Executes a complete query lifecycle: connect, send, receive all messages, and close.
    ///
    /// # Arguments
    ///
    /// * `prompt` — The input prompt (text or structured messages).
    /// * `options` — Configuration options for this query.
    /// * `transport` — Optional custom transport. If `None`, uses the default subprocess transport.
    ///
    /// # Returns
    ///
    /// A `Vec<Message>` containing all messages from the interaction.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - `can_use_tool` is set with a `Text` prompt
    /// - `can_use_tool` is set alongside `permission_prompt_tool_name`
    /// - The CLI process fails to start or communicate
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

        let mut query = self
            .initialize_query(transport_prompt, configured_options, transport)
            .await?;
        Self::send_prompt(&mut query, prompt).await?;
        Self::collect_messages(query).await
    }

    /// Executes a one-shot query where input messages are provided as a stream.
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
        let mut query = self
            .initialize_query(TransportPrompt::Messages, configured_options, transport)
            .await?;
        query.stream_input_from_stream(prompt).await?;
        Self::collect_messages(query).await
    }

    /// Executes a one-shot query and returns a streaming response interface.
    pub async fn process_query_as_stream(
        &self,
        prompt: InputPrompt,
        options: ClaudeAgentOptions,
        transport: Option<Box<dyn Transport>>,
    ) -> Result<LocalBoxStream<'static, Result<Message>>> {
        let configured_options =
            Self::configure_options(options, matches!(prompt, InputPrompt::Text(_)))?;
        let transport_prompt = match &prompt {
            InputPrompt::Text(text) => TransportPrompt::Text(text.clone()),
            InputPrompt::Messages(_) => TransportPrompt::Messages,
        };
        let mut query = self
            .initialize_query(transport_prompt, configured_options, transport)
            .await?;
        Self::send_prompt(&mut query, prompt).await?;
        Ok(Self::into_message_stream(query))
    }

    /// Executes a one-shot streamed-input query and returns a streaming response interface.
    pub async fn process_query_from_stream_as_stream<S>(
        &self,
        prompt: S,
        options: ClaudeAgentOptions,
        transport: Option<Box<dyn Transport>>,
    ) -> Result<LocalBoxStream<'static, Result<Message>>>
    where
        S: Stream<Item = Value> + Unpin,
    {
        let configured_options = Self::configure_options(options, false)?;
        let mut query = self
            .initialize_query(TransportPrompt::Messages, configured_options, transport)
            .await?;
        query.stream_input_from_stream(prompt).await?;
        Ok(Self::into_message_stream(query))
    }
}
