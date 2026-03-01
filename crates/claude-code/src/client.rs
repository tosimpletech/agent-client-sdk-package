use std::collections::HashMap;
use std::time::Duration;

use serde_json::Value;

use crate::errors::{CLIConnectionError, Error, Result};
use crate::query::Query;
use crate::transport::Transport;
use crate::transport::subprocess_cli::{Prompt as TransportPrompt, SubprocessCliTransport};
use crate::types::{ClaudeAgentOptions, McpServerConfig, McpServersOption, Message};

#[derive(Debug, Clone, PartialEq)]
pub enum InputPrompt {
    Text(String),
    Messages(Vec<Value>),
}

pub struct ClaudeSdkClient {
    options: ClaudeAgentOptions,
    custom_transport: Option<Box<dyn Transport>>,
    query: Option<Query>,
}

impl ClaudeSdkClient {
    pub fn new(options: Option<ClaudeAgentOptions>, transport: Option<Box<dyn Transport>>) -> Self {
        Self {
            options: options.unwrap_or_default(),
            custom_transport: transport,
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
            query.stream_input(messages).await?;
        }

        self.query = Some(query);
        Ok(())
    }

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

    pub async fn receive_message(&mut self) -> Result<Option<Message>> {
        let query = self.query.as_mut().ok_or_else(|| {
            Error::CLIConnection(CLIConnectionError::new(
                "Not connected. Call connect() first.",
            ))
        })?;
        query.receive_next_message().await
    }

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

    pub async fn interrupt(&mut self) -> Result<()> {
        let query = self.query.as_mut().ok_or_else(|| {
            Error::CLIConnection(CLIConnectionError::new(
                "Not connected. Call connect() first.",
            ))
        })?;
        query.interrupt().await
    }

    pub async fn set_permission_mode(&mut self, mode: &str) -> Result<()> {
        let query = self.query.as_mut().ok_or_else(|| {
            Error::CLIConnection(CLIConnectionError::new(
                "Not connected. Call connect() first.",
            ))
        })?;
        query.set_permission_mode(mode).await
    }

    pub async fn set_model(&mut self, model: Option<&str>) -> Result<()> {
        let query = self.query.as_mut().ok_or_else(|| {
            Error::CLIConnection(CLIConnectionError::new(
                "Not connected. Call connect() first.",
            ))
        })?;
        query.set_model(model).await
    }

    pub async fn rewind_files(&mut self, user_message_id: &str) -> Result<()> {
        let query = self.query.as_mut().ok_or_else(|| {
            Error::CLIConnection(CLIConnectionError::new(
                "Not connected. Call connect() first.",
            ))
        })?;
        query.rewind_files(user_message_id).await
    }

    pub async fn get_mcp_status(&mut self) -> Result<Value> {
        let query = self.query.as_mut().ok_or_else(|| {
            Error::CLIConnection(CLIConnectionError::new(
                "Not connected. Call connect() first.",
            ))
        })?;
        query.get_mcp_status().await
    }

    pub fn get_server_info(&self) -> Result<Option<Value>> {
        let query = self.query.as_ref().ok_or_else(|| {
            Error::CLIConnection(CLIConnectionError::new(
                "Not connected. Call connect() first.",
            ))
        })?;
        Ok(query.initialization_result())
    }

    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(query) = &mut self.query {
            query.close().await?;
        }
        self.query = None;
        Ok(())
    }
}
