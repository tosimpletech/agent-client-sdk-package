//! Claude Code adapter implementation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use chrono::Utc;
use claude_code::{
    ClaudeAgentOptions, ClaudeSdkClient, Error as ClaudeError, InputPrompt, Message, Prompt,
    SubprocessCliTransport,
};
use tokio::sync::Mutex;

use crate::error::{ExecutorError, Result};
use crate::executor::{AgentCapabilities, AgentExecutor, AvailabilityStatus, SpawnConfig};
use crate::session::AgentSession;
use crate::types::{ExecutorType, PermissionPolicy};

const DEFAULT_SESSION_ID: &str = "default";
const MAX_TRACKED_SESSIONS: usize = 64;
static FALLBACK_SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);

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
            sessions.remove(&session_id)
        };

        if let Some(mut old_client) = old_client
            && let Err(error) = old_client.disconnect().await
        {
            let mut sessions = self.sessions.lock().await;
            sessions.entry(session_id).or_insert(old_client);
            return Err(map_claude_error(
                "failed to disconnect replaced claude session",
                error,
            ));
        }

        let mut sessions = self.sessions.lock().await;
        let current_session_id = session_id.clone();
        sessions.insert(current_session_id.clone(), client);
        let evicted_client = if sessions.len() > MAX_TRACKED_SESSIONS {
            let evicted_session_id = sessions
                .keys()
                .find(|session_id| *session_id != &current_session_id)
                .cloned()
                .unwrap_or_else(|| DEFAULT_SESSION_ID.to_string());
            sessions.remove(&evicted_session_id)
        } else {
            None
        };
        drop(sessions);

        if let Some(mut evicted_client) = evicted_client {
            let _ = evicted_client.disconnect().await;
        }

        Ok(())
    }

    fn to_agent_session(&self, session_id: String, working_dir: &Path) -> AgentSession {
        AgentSession {
            session_id,
            executor_type: ExecutorType::ClaudeCode,
            working_dir: working_dir.to_path_buf(),
            created_at: Utc::now(),
            last_message_id: None,
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

        client
            .connect(None)
            .await
            .map_err(|error| map_claude_error("failed to connect claude sdk client", error))?;
        client
            .query(InputPrompt::Text(prompt.to_owned()), DEFAULT_SESSION_ID)
            .await
            .map_err(|error| {
                map_claude_error("failed to send prompt to claude sdk client", error)
            })?;

        let messages = client.receive_response().await.map_err(|error| {
            map_claude_error("failed to receive response from claude sdk client", error)
        })?;
        ensure_query_succeeded(&messages, "failed to execute prompt in claude sdk session")?;
        let session_id = extract_session_id(&messages).unwrap_or_else(unique_fallback_session_id);

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

        client
            .connect(None)
            .await
            .map_err(|error| map_claude_error("failed to connect claude sdk client", error))?;

        if let Some(reset_to) = reset_to {
            client.rewind_files(reset_to).await.map_err(|error| {
                map_claude_error("failed to rewind claude session files", error)
            })?;
        }

        client
            .query(InputPrompt::Text(prompt.to_owned()), session_id)
            .await
            .map_err(|error| {
                map_claude_error("failed to send prompt to resumed claude session", error)
            })?;

        let messages = client.receive_response().await.map_err(|error| {
            map_claude_error(
                "failed to receive response from resumed claude session",
                error,
            )
        })?;
        ensure_query_succeeded(
            &messages,
            "failed to execute prompt in resumed claude session",
        )?;
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

fn unique_fallback_session_id() -> String {
    let sequence = FALLBACK_SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
    let timestamp_nanos = Utc::now().timestamp_nanos_opt().unwrap_or_default();
    format!("{DEFAULT_SESSION_ID}-{timestamp_nanos}-{sequence}")
}

fn ensure_query_succeeded(messages: &[Message], context: &str) -> Result<()> {
    let Some(result) = messages.iter().rev().find_map(|message| match message {
        Message::Result(result) => Some(result),
        _ => None,
    }) else {
        return Ok(());
    };

    if result.is_error {
        let detail = result
            .result
            .as_deref()
            .unwrap_or("claude query returned an error result");
        return Err(ExecutorError::execution_failed(context, detail));
    }

    Ok(())
}

fn map_claude_error(context: &str, error: ClaudeError) -> ExecutorError {
    match error {
        ClaudeError::CLINotFound(err) => ExecutorError::unavailable(context, err),
        ClaudeError::CLIConnection(err) => ExecutorError::spawn_failed(context, err),
        ClaudeError::Io(err) => {
            ExecutorError::Io(std::io::Error::new(err.kind(), format!("{context}: {err}")))
        }
        ClaudeError::Process(err) => ExecutorError::execution_failed(context, err),
        ClaudeError::CLIJSONDecode(err) => ExecutorError::execution_failed(context, err),
        ClaudeError::MessageParse(err) => ExecutorError::execution_failed(context, err),
        ClaudeError::Json(err) => ExecutorError::Serialization(err),
        ClaudeError::ClaudeSDK(err) => ExecutorError::invalid_config(context, err),
        ClaudeError::Other(msg) => ExecutorError::other(context, msg),
    }
}

fn resolve_cli_path(options: &ClaudeAgentOptions) -> std::result::Result<PathBuf, String> {
    if let Some(cli_path) = &options.cli_path {
        return validate_cli_path(cli_path, "configured Claude CLI path");
    }

    let transport = SubprocessCliTransport::new(Prompt::Messages, options.clone())
        .map_err(|error| error.to_string())?;
    let resolved = PathBuf::from(&transport.cli_path);
    validate_cli_path(&resolved, "Claude CLI path resolved by SDK")
}

fn validate_cli_path(path: &Path, label: &str) -> std::result::Result<PathBuf, String> {
    if !path.exists() {
        return Err(format!("{label} does not exist: {}", path.display()));
    }
    if !path.is_file() {
        return Err(format!(
            "{label} must point to an executable file: {}",
            path.display()
        ));
    }
    if !is_executable_file(path) {
        return Err(format!("{label} is not executable: {}", path.display()));
    }
    Ok(path.to_path_buf())
}

fn is_executable_file(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use claude_code::ResultMessage;
    use std::fs::{self, File};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn result_message(is_error: bool, result: Option<&str>) -> Message {
        Message::Result(ResultMessage {
            subtype: if is_error {
                "error".to_string()
            } else {
                "success".to_string()
            },
            duration_ms: 1,
            duration_api_ms: 1,
            is_error,
            num_turns: 1,
            session_id: "session-1".to_string(),
            total_cost_usd: None,
            usage: None,
            result: result.map(str::to_string),
            structured_output: None,
        })
    }

    #[test]
    fn fallback_session_ids_are_unique() {
        let first = unique_fallback_session_id();
        let second = unique_fallback_session_id();

        assert_ne!(first, second);
        assert!(first.starts_with(DEFAULT_SESSION_ID));
        assert!(second.starts_with(DEFAULT_SESSION_ID));
    }

    #[test]
    fn query_success_check_rejects_error_result_messages() {
        let messages = vec![result_message(true, Some("permission denied"))];

        let error = ensure_query_succeeded(&messages, "spawn failed")
            .expect_err("error result message should fail");
        assert!(error.to_string().contains("permission denied"));
    }

    #[test]
    fn query_success_check_accepts_success_result_messages() {
        let messages = vec![result_message(false, Some("ok"))];
        assert!(ensure_query_succeeded(&messages, "spawn failed").is_ok());
    }

    #[test]
    fn resolve_cli_path_rejects_directory_override() {
        let temp_dir = new_temp_path("claude-cli-dir");
        fs::create_dir_all(&temp_dir).expect("directory should be created");

        let options = ClaudeAgentOptions {
            cli_path: Some(temp_dir.clone()),
            ..ClaudeAgentOptions::default()
        };

        let result = resolve_cli_path(&options);
        assert!(result.is_err());
        assert!(
            result
                .expect_err("directory should be rejected")
                .contains("must point to an executable file")
        );

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn resolve_cli_path_accepts_executable_file_override() {
        let temp_file = new_temp_path("claude-cli-file");
        File::create(&temp_file).expect("file should be created");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&temp_file)
                .expect("metadata should exist")
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&temp_file, permissions).expect("permissions should be set");
        }

        let options = ClaudeAgentOptions {
            cli_path: Some(temp_file.clone()),
            ..ClaudeAgentOptions::default()
        };

        let resolved = resolve_cli_path(&options).expect("file should be accepted");
        assert_eq!(resolved, temp_file);

        let _ = fs::remove_file(temp_file);
    }

    fn new_temp_path(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time should move forward")
                .as_nanos()
        ))
    }
}
