//! Agent executor abstraction.
//!
//! This module defines the provider-agnostic execution contract used by the SDK.
//! Concrete implementations live under [`crate::providers`].

use async_trait::async_trait;
use std::path::Path;

use crate::{error::Result, session::AgentSession, types::ExecutorType};

/// Capability declaration for one executor implementation.
#[derive(Debug, Clone)]
pub struct AgentCapabilities {
    /// Whether the executor can fork from an existing session history.
    pub session_fork: bool,
    /// Whether context/token usage events are exposed.
    pub context_usage: bool,
    /// Whether Model Context Protocol (MCP) tool calls are supported.
    pub mcp_support: bool,
    /// Whether structured output is supported.
    pub structured_output: bool,
}

/// Runtime availability state for an executor backend.
#[derive(Debug, Clone)]
pub struct AvailabilityStatus {
    /// Whether the executor is currently available.
    pub available: bool,
    /// Optional human-readable reason for unavailable (or additional diagnostics).
    pub reason: Option<String>,
}

/// Session spawn/resume configuration.
#[derive(Debug, Clone, Default)]
pub struct SpawnConfig {
    /// Optional model override.
    pub model: Option<String>,
    /// Optional reasoning level/effort override.
    pub reasoning: Option<String>,
    /// Optional permission policy override.
    pub permission_policy: Option<crate::types::PermissionPolicy>,
    /// Extra environment variables to forward to the executor process.
    pub env: Vec<(String, String)>,
    /// Optional context window capacity override (tokens) used for unified context usage events.
    pub context_window_override_tokens: Option<u32>,
}

/// Core executor trait implemented by all providers.
///
/// The trait intentionally models only common behavior. Provider-specific advanced
/// features should be layered outside this trait to keep API parity predictable.
///
/// # Examples
///
/// ```rust
/// use std::path::Path;
/// use unified_agent_sdk::{AgentExecutor, executor::SpawnConfig};
///
/// async fn run_prompt(executor: &dyn AgentExecutor) -> unified_agent_sdk::Result<()> {
///     let session = executor
///         .spawn(
///             Path::new("."),
///             "Summarize this repository.",
///             &SpawnConfig {
///                 model: None,
///                 reasoning: Some("medium".to_string()),
///                 permission_policy: None,
///                 env: Vec::new(),
///                 context_window_override_tokens: None,
///             },
///         )
///         .await?;
///
///     let _metadata = session.metadata();
///     Ok(())
/// }
/// ```
#[async_trait]
pub trait AgentExecutor: Send + Sync {
    /// Returns the provider type implemented by this executor.
    fn executor_type(&self) -> ExecutorType;

    /// Starts a fresh session and sends `prompt` as the first turn.
    ///
    /// `working_dir` should point to the repository/workspace where tools and file
    /// operations are expected to run.
    async fn spawn(
        &self,
        working_dir: &Path,
        prompt: &str,
        config: &SpawnConfig,
    ) -> Result<AgentSession>;

    /// Resumes a previously created session and sends a follow-up `prompt`.
    ///
    /// `session_id` is the provider-native identifier returned by a previous session.
    /// `reset_to` is optional and only supported by providers that expose rewind/reset
    /// semantics.
    async fn resume(
        &self,
        working_dir: &Path,
        prompt: &str,
        session_id: &str,
        reset_to: Option<&str>,
        config: &SpawnConfig,
    ) -> Result<AgentSession>;

    /// Returns static capability flags for this executor implementation.
    fn capabilities(&self) -> AgentCapabilities;

    /// Performs a lightweight availability check for required runtime dependencies.
    fn availability(&self) -> AvailabilityStatus;
}
