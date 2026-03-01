use std::collections::HashMap;
use std::time::Duration;

use crate::client::InputPrompt;
use crate::errors::{Error, Result};
use crate::query::Query;
use crate::transport::Transport;
use crate::transport::subprocess_cli::{Prompt as TransportPrompt, SubprocessCliTransport};
use crate::types::{ClaudeAgentOptions, McpServerConfig, McpServersOption, Message};

pub struct InternalClient;

impl Default for InternalClient {
    fn default() -> Self {
        Self::new()
    }
}

impl InternalClient {
    pub fn new() -> Self {
        Self
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
