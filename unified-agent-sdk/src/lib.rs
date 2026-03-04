//! Unified SDK for multiple AI coding agents
//!
//! This SDK provides a unified interface for interacting with different AI coding agents
//! (Claude Code, Codex, etc.) through a common abstraction layer.

pub mod executor;
pub mod profile;
pub mod log;
pub mod session;
pub mod event;
pub mod error;
pub mod types;

pub use executor::{AgentExecutor, AgentCapabilities, AvailabilityStatus};
pub use profile::{ProfileId, ExecutorConfig, ProfileManager};
pub use log::{NormalizedLog, LogNormalizer};
pub use session::{AgentSession, SessionMetadata, SessionResume};
pub use event::{AgentEvent, EventStream, HookManager, EventType};
pub use error::{ExecutorError, Result};
pub use types::*;
