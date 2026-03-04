//! Claude Code adapter implementation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use claude_code::{
    ClaudeAgentOptions, ClaudeSdkClient, Error as ClaudeError, InputPrompt, Message,
};
use tokio::sync::Mutex;

use crate::error::{ExecutorError, Result};
use crate::executor::{AgentCapabilities, AgentExecutor, AvailabilityStatus, SpawnConfig};
use crate::session::AgentSession;
use crate::types::{ExecutorType, PermissionPolicy};

const DEFAULT_SESSION_ID: &str = "default";

/// Executor adapter backed by `claude_code::ClaudeSdkClient`.
#[derive(Clone)]
pub struct ClaudeCodeExecutor {
    base_options: ClaudeAgentOptions,
    sessions: Arc<Mutex<HashMap<String, ClaudeSdkClient>>>,
}

impl ClaudeCodeExecutor {
    /// Creates an executor with default Claude SDK options.
    pub fn new() -> Self {
        Self::with_options(ClaudeAgentOptions::default())
    }

    /// Creates an executor with pre-configured Claude SDK options.
    pub fn with_options(options: ClaudeAgentOptions) -> Self {
        Self {
            base_options: options,
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn build_options(
        &self,
        working_dir: &Path,
        config: &SpawnConfig,
        resume_session: Option<&str>,
        continue_conversation: bool,
        fork_session: bool,
    ) -> ClaudeAgentOptions {
        let mut options = self.base_options.clone();
        options.cwd = Some(working_dir.to_path_buf());

        if config.model.is_some() {
            options.model = config.model.clone();
        }
        if config.reasoning.is_some() {
            options.effort = config.reasoning.clone();
        }
        if let Some(mode) = map_permission_policy(config.permission_policy) {
            options.permission_mode = Some(mode);
        }

        options.env.extend(config.env.iter().cloned());
        options.continue_conversation = continue_conversation;
        options.resume = resume_session.map(str::to_owned);
        options.fork_session = fork_session;
        options
    }

    async fn store_client(&self, session_id: String, client: ClaudeSdkClient) -> Result<()> {
        let old_client = {
            let mut sessions = self.sessions.lock().await;
            sessions.insert(session_id, client)
        };

        if let Some(mut old_client) = old_client {
            old_client.disconnect().await.map_err(map_claude_error)?;
        }

        Ok(())
    }

    fn to_agent_session(&self, session_id: String, working_dir: &Path) -> AgentSession {
        AgentSession {
            session_id,
            executor_type: ExecutorType::ClaudeCode,
            working_dir: working_dir.to_path_buf(),
        }
    }
}

impl Default for ClaudeCodeExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AgentExecutor for ClaudeCodeExecutor {
    fn executor_type(&self) -> ExecutorType {
        ExecutorType::ClaudeCode
    }

    async fn spawn(
        &self,
        working_dir: &Path,
        prompt: &str,
        config: &SpawnConfig,
    ) -> Result<AgentSession> {
        let options = self.build_options(working_dir, config, None, false, false);
        let mut client = ClaudeSdkClient::new(Some(options), None);

        client.connect(None).await.map_err(map_claude_error)?;
        client
            .query(InputPrompt::Text(prompt.to_owned()), DEFAULT_SESSION_ID)
            .await
            .map_err(map_claude_error)?;

        let messages = client.receive_response().await.map_err(map_claude_error)?;
        let session_id =
            extract_session_id(&messages).unwrap_or_else(|| DEFAULT_SESSION_ID.to_string());

        self.store_client(session_id.clone(), client).await?;
        Ok(self.to_agent_session(session_id, working_dir))
    }

    async fn resume(
        &self,
        working_dir: &Path,
        prompt: &str,
        session_id: &str,
        reset_to: Option<&str>,
        config: &SpawnConfig,
    ) -> Result<AgentSession> {
        let options = self.build_options(working_dir, config, Some(session_id), true, false);
        let mut client = ClaudeSdkClient::new(Some(options), None);

        client.connect(None).await.map_err(map_claude_error)?;

        if let Some(reset_to) = reset_to {
            client
                .rewind_files(reset_to)
                .await
                .map_err(map_claude_error)?;
        }

        client
            .query(InputPrompt::Text(prompt.to_owned()), session_id)
            .await
            .map_err(map_claude_error)?;

        let messages = client.receive_response().await.map_err(map_claude_error)?;
        let resumed_session_id =
            extract_session_id(&messages).unwrap_or_else(|| session_id.to_string());

        self.store_client(resumed_session_id.clone(), client)
            .await?;
        Ok(self.to_agent_session(resumed_session_id, working_dir))
    }

    fn capabilities(&self) -> AgentCapabilities {
        AgentCapabilities {
            session_fork: true,
            context_usage: true,
            mcp_support: true,
            structured_output: true,
        }
    }

    fn availability(&self) -> AvailabilityStatus {
        match resolve_cli_path(&self.base_options) {
            Ok(path) => AvailabilityStatus {
                available: true,
                reason: Some(format!("Claude CLI found at {}", path.display())),
            },
            Err(reason) => AvailabilityStatus {
                available: false,
                reason: Some(reason),
            },
        }
    }
}

fn map_permission_policy(policy: Option<PermissionPolicy>) -> Option<claude_code::PermissionMode> {
    match policy {
        Some(PermissionPolicy::Bypass) => Some(claude_code::PermissionMode::BypassPermissions),
        Some(PermissionPolicy::Prompt) => Some(claude_code::PermissionMode::Default),
        Some(PermissionPolicy::Deny) => Some(claude_code::PermissionMode::Plan),
        None => None,
    }
}

fn extract_session_id(messages: &[Message]) -> Option<String> {
    messages.iter().rev().find_map(|message| match message {
        Message::Result(result) => Some(result.session_id.clone()),
        Message::StreamEvent(event) => Some(event.session_id.clone()),
        _ => None,
    })
}

fn map_claude_error(error: ClaudeError) -> ExecutorError {
    match error {
        ClaudeError::CLINotFound(err) => ExecutorError::Unavailable(err.to_string()),
        ClaudeError::CLIConnection(err) => ExecutorError::SpawnFailed(err.to_string()),
        ClaudeError::Io(err) => ExecutorError::Io(err),
        ClaudeError::Process(err) => ExecutorError::ExecutionFailed(err.to_string()),
        ClaudeError::CLIJSONDecode(err) => ExecutorError::ExecutionFailed(err.to_string()),
        ClaudeError::MessageParse(err) => ExecutorError::ExecutionFailed(err.to_string()),
        ClaudeError::Json(err) => ExecutorError::Serialization(err),
        ClaudeError::ClaudeSDK(err) => ExecutorError::InvalidConfig(err.to_string()),
        ClaudeError::Other(msg) => ExecutorError::Other(msg),
    }
}

fn resolve_cli_path(options: &ClaudeAgentOptions) -> std::result::Result<PathBuf, String> {
    if let Some(cli_path) = &options.cli_path {
        if cli_path.exists() {
            return Ok(cli_path.clone());
        }
        return Err(format!(
            "Configured Claude CLI path does not exist: {}",
            cli_path.display()
        ));
    }

    if let Ok(path) = which::which("claude") {
        return Ok(path);
    }

    if let Ok(path) = std::env::var("CLAUDE_CODE_BUNDLED_CLI") {
        let bundled = PathBuf::from(path);
        if bundled.exists() {
            return Ok(bundled);
        }
    }

    Err(
        "Claude Code CLI not found. Install with `npm install -g @anthropic-ai/claude-code`."
            .to_string(),
    )
}
