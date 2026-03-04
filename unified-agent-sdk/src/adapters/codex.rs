//! Codex adapter for unified executor abstraction.

use std::path::Path;

use async_trait::async_trait;
use codex::{ApprovalMode, Codex, CodexOptions, ModelReasoningEffort, Thread, ThreadOptions};

use crate::{
    error::{ExecutorError, Result},
    executor::{AgentCapabilities, AgentExecutor, AvailabilityStatus, SpawnConfig},
    session::AgentSession,
    types::{ExecutorType, PermissionPolicy},
};

/// Adapter that maps [`codex::Codex`] to the unified [`AgentExecutor`] interface.
#[derive(Debug, Clone, Default)]
pub struct CodexExecutor {
    options: CodexOptions,
}

impl CodexExecutor {
    /// Create a new executor with optional base Codex options.
    pub fn new(options: Option<CodexOptions>) -> Self {
        Self {
            options: options.unwrap_or_default(),
        }
    }

    fn build_client(&self, config: &SpawnConfig) -> Result<Codex> {
        let mut options = self.options.clone();
        let mut env = options.env.take().unwrap_or_default();

        for (key, value) in &config.env {
            env.insert(key.clone(), value.clone());
        }
        options.env = (!env.is_empty()).then_some(env);

        Codex::new(Some(options)).map_err(|error| {
            ExecutorError::spawn_failed("failed to initialize codex client", error)
        })
    }

    fn build_thread_options(
        &self,
        working_dir: &Path,
        config: &SpawnConfig,
    ) -> Result<ThreadOptions> {
        Ok(ThreadOptions {
            model: config.model.clone(),
            model_reasoning_effort: parse_reasoning_effort(config.reasoning.as_deref())?,
            approval_policy: map_permission_policy(config.permission_policy),
            working_directory: Some(working_dir.to_string_lossy().to_string()),
            ..ThreadOptions::default()
        })
    }

    fn wrap_thread(
        thread: &Thread,
        working_dir: &Path,
        fallback_session_id: Option<&str>,
    ) -> Result<AgentSession> {
        let session_id = thread
            .id()
            .or_else(|| fallback_session_id.map(ToOwned::to_owned))
            .ok_or_else(|| {
                ExecutorError::execution_failed(
                    "failed to resolve codex session id",
                    "codex did not return a thread id after running prompt",
                )
            })?;

        Ok(AgentSession {
            session_id,
            executor_type: ExecutorType::Codex,
            working_dir: working_dir.to_path_buf(),
        })
    }
}

#[async_trait]
impl AgentExecutor for CodexExecutor {
    fn executor_type(&self) -> ExecutorType {
        ExecutorType::Codex
    }

    async fn spawn(
        &self,
        working_dir: &Path,
        prompt: &str,
        config: &SpawnConfig,
    ) -> Result<AgentSession> {
        let codex = self.build_client(config)?;
        let thread_options = self.build_thread_options(working_dir, config)?;
        let thread = codex.start_thread(Some(thread_options));

        thread.run(prompt, None).await.map_err(|error| {
            ExecutorError::execution_failed("failed to execute prompt in codex session", error)
        })?;

        Self::wrap_thread(&thread, working_dir, None)
    }

    async fn resume(
        &self,
        working_dir: &Path,
        prompt: &str,
        session_id: &str,
        reset_to: Option<&str>,
        config: &SpawnConfig,
    ) -> Result<AgentSession> {
        if reset_to.is_some() {
            return Err(ExecutorError::invalid_config(
                "failed to resume codex session",
                "codex adapter does not support reset_to",
            ));
        }

        let codex = self.build_client(config)?;
        let thread_options = self.build_thread_options(working_dir, config)?;
        let thread = codex.resume_thread(session_id, Some(thread_options));

        thread.run(prompt, None).await.map_err(|error| {
            ExecutorError::execution_failed(
                format!("failed to execute prompt in resumed codex session '{session_id}'"),
                error,
            )
        })?;

        Self::wrap_thread(&thread, working_dir, Some(session_id))
    }

    fn capabilities(&self) -> AgentCapabilities {
        AgentCapabilities {
            session_fork: false,
            context_usage: true,
            mcp_support: true,
            structured_output: true,
        }
    }

    fn availability(&self) -> AvailabilityStatus {
        match Codex::new(Some(self.options.clone())) {
            Ok(_) => AvailabilityStatus {
                available: true,
                reason: None,
            },
            Err(error) => AvailabilityStatus {
                available: false,
                reason: Some(error.to_string()),
            },
        }
    }
}

fn parse_reasoning_effort(reasoning: Option<&str>) -> Result<Option<ModelReasoningEffort>> {
    let Some(reasoning) = reasoning else {
        return Ok(None);
    };

    let normalized = reasoning.trim().to_ascii_lowercase();
    let effort = match normalized.as_str() {
        "minimal" => ModelReasoningEffort::Minimal,
        "low" => ModelReasoningEffort::Low,
        "medium" => ModelReasoningEffort::Medium,
        "high" => ModelReasoningEffort::High,
        "xhigh" | "x-high" | "extra-high" | "extra_high" => ModelReasoningEffort::XHigh,
        _ => {
            return Err(ExecutorError::invalid_config(
                "failed to parse codex reasoning level",
                format!(
                    "unsupported value '{reasoning}', expected one of: minimal, low, medium, high, xhigh"
                ),
            ));
        }
    };

    Ok(Some(effort))
}

fn map_permission_policy(policy: Option<PermissionPolicy>) -> Option<ApprovalMode> {
    policy.map(|policy| match policy {
        PermissionPolicy::Bypass => ApprovalMode::Never,
        PermissionPolicy::Prompt => ApprovalMode::OnRequest,
        PermissionPolicy::Deny => ApprovalMode::Untrusted,
    })
}
