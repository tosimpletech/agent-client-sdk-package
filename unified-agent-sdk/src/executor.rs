//! Agent executor abstraction

use async_trait::async_trait;
use std::path::Path;

use crate::{error::Result, session::AgentSession, types::ExecutorType};

/// Agent capabilities
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

/// Availability status
#[derive(Debug, Clone)]
pub struct AvailabilityStatus {
    /// Whether the executor is currently available.
    pub available: bool,
    /// Optional human-readable reason for unavailable (or additional diagnostics).
    pub reason: Option<String>,
}

/// Spawn configuration
#[derive(Debug, Clone)]
pub struct SpawnConfig {
    /// Optional model override.
    pub model: Option<String>,
    /// Optional reasoning level/effort override.
    pub reasoning: Option<String>,
    /// Optional permission policy override.
    pub permission_policy: Option<crate::types::PermissionPolicy>,
    /// Extra environment variables to forward to the executor process.
    pub env: Vec<(String, String)>,
}

/// Core executor trait
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
    /// Get executor type
    fn executor_type(&self) -> ExecutorType;

    /// Spawn new session
    async fn spawn(
        &self,
        working_dir: &Path,
        prompt: &str,
        config: &SpawnConfig,
    ) -> Result<AgentSession>;

    /// Resume existing session
    async fn resume(
        &self,
        working_dir: &Path,
        prompt: &str,
        session_id: &str,
        reset_to: Option<&str>,
        config: &SpawnConfig,
    ) -> Result<AgentSession>;

    /// Get capabilities
    fn capabilities(&self) -> AgentCapabilities;

    /// Check availability
    fn availability(&self) -> AvailabilityStatus;
}
