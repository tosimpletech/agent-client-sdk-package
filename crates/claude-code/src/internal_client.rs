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
        if options.can_use_tool.is_some() && matches!(prompt, InputPrompt::Text(_)) {
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

        let mut configured_options = options.clone();
        if configured_options.can_use_tool.is_some() {
            configured_options.permission_prompt_tool_name = Some("stdio".to_string());
        }

        let transport_prompt = match &prompt {
            InputPrompt::Text(text) => TransportPrompt::Text(text.clone()),
            InputPrompt::Messages(_) => TransportPrompt::Messages,
        };

        let mut chosen_transport: Box<dyn Transport> = if let Some(transport) = transport {
            transport
        } else {
            Box::new(SubprocessCliTransport::new(
                transport_prompt,
                configured_options.clone(),
            )?)
        };
        chosen_transport.connect().await?;

        let mut query = Query::new(
            chosen_transport,
            true,
            configured_options.can_use_tool.clone(),
            configured_options.hooks.clone(),
            Some(Self::extract_sdk_mcp_servers(&configured_options)),
            configured_options.agents.clone(),
            Duration::from_secs(60),
        );
        query.start().await?;
        query.initialize().await?;

        match prompt {
            InputPrompt::Text(text) => {
                query.send_user_message(&text, "").await?;
                query.end_input().await?;
            }
            InputPrompt::Messages(messages) => {
                query.stream_input(messages).await?;
            }
        }

        let mut messages = Vec::new();
        while let Some(message) = query.receive_next_message().await? {
            messages.push(message);
        }
        query.close().await?;
        Ok(messages)
    }
}
