//! Unified SDK for multiple AI coding agents
//!
//! This SDK provides a unified interface for interacting with different AI coding agents
//! (Claude Code, Codex, etc.) through a common abstraction layer.

pub mod adapters;
pub mod error;
pub mod event;
pub mod executor;
pub mod log;
pub mod profile;
pub mod session;
pub mod types;

pub use adapters::ClaudeCodeExecutor;
pub use adapters::CodexExecutor;
pub use error::{ExecutorError, Result};
pub use event::{AgentEvent, EventStream, EventType, HookManager};
pub use executor::{AgentCapabilities, AgentExecutor, AvailabilityStatus};
pub use log::{LogNormalizer, NormalizedLog};
pub use profile::{ExecutorConfig, ProfileId, ProfileManager};
pub use session::{AgentSession, SessionMetadata, SessionResume};
pub use types::*;
