//! Agent executor abstraction

use async_trait::async_trait;
use std::path::Path;

use crate::{
    error::Result,
    session::AgentSession,
    types::ExecutorType,
};

/// Agent capabilities
#[derive(Debug, Clone)]
pub struct AgentCapabilities {
    pub session_fork: bool,
    pub context_usage: bool,
    pub mcp_support: bool,
    pub structured_output: bool,
}

/// Availability status
#[derive(Debug, Clone)]
pub struct AvailabilityStatus {
    pub available: bool,
    pub reason: Option<String>,
}

/// Spawn configuration
#[derive(Debug, Clone)]
pub struct SpawnConfig {
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub permission_policy: Option<crate::types::PermissionPolicy>,
    pub env: Vec<(String, String)>,
}

/// Core executor trait
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
