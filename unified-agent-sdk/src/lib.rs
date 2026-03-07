//! Unified SDK for multiple AI coding agents
//!
//! This SDK provides a unified interface for interacting with different AI coding agents
//! (Claude Code, Codex, etc.) through a common abstraction layer.

pub mod error;
pub mod event;
pub mod executor;
pub mod log;
pub mod profile;
pub mod providers;
pub mod session;
pub mod types;

pub use error::{ExecutorError, Result};
pub use event::{
    AgentEvent, EventConverter, EventStream, EventType, HookManager, normalized_log_to_event,
};
pub use executor::{AgentCapabilities, AgentExecutor, AvailabilityStatus};
pub use log::{LogNormalizer, NormalizedLog};
pub use profile::{DiscoveryData, ExecutorConfig, ProfileId, ProfileManager};
pub use providers::{ClaudeCodeExecutor, ClaudeCodeLogNormalizer, CodexExecutor, CodexLogNormalizer};
pub use session::{AgentSession, SessionMetadata, SessionResume};
pub use types::{
    ContextUsage, ContextUsageSource, ExecutorType, ExitStatus, PermissionPolicy, Role, ToolStatus,
};
