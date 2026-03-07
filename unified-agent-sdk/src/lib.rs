//! Unified Rust SDK for multiple AI coding agents.
//!
//! `unified-agent-sdk` provides one consistent interface over multiple agent backends
//! (currently Claude Code and Codex). It is designed for applications that want to swap
//! executors without rewriting orchestration logic.
//!
//! # Feature Overview
//!
//! - Unified executor API via [`AgentExecutor`]
//! - Provider adapters for Claude Code and Codex
//! - Unified event pipeline (`raw logs -> normalized logs -> AgentEvent`)
//! - Profile/config resolution with runtime discovery ([`ProfileManager`])
//! - Cross-provider capability and availability introspection
//!
//! # Common Scenarios
//!
//! Start a new session with one provider:
//!
//! ```rust,no_run
//! use unified_agent_sdk::{
//!     AgentExecutor, CodexExecutor, PermissionPolicy, executor::SpawnConfig,
//! };
//!
//! # async fn run() -> unified_agent_sdk::Result<()> {
//! let executor = CodexExecutor::default();
//! let config = SpawnConfig {
//!     model: Some("gpt-5-codex".to_string()),
//!     reasoning: Some("medium".to_string()),
//!     permission_policy: Some(PermissionPolicy::Prompt),
//!     env: vec![],
//!     context_window_override_tokens: None,
//! };
//!
//! let cwd = std::env::current_dir()?;
//! let session = executor
//!     .spawn(&cwd, "Summarize this repository in three bullets.", &config)
//!     .await?;
//!
//! println!("session_id={}", session.session_id);
//! # Ok(())
//! # }
//! ```
//!
//! Resolve profile + runtime overrides before spawning:
//!
//! ```rust,no_run
//! use unified_agent_sdk::{
//!     ExecutorConfig, ExecutorType, ProfileId, ProfileManager,
//! };
//!
//! # async fn run() -> unified_agent_sdk::Result<()> {
//! let manager = ProfileManager::new();
//! let resolved = manager
//!     .resolve(&ExecutorConfig {
//!         profile_id: ProfileId::new(ExecutorType::Codex, Some("default".to_string())),
//!         model_override: None,
//!         reasoning_override: Some("high".to_string()),
//!         permission_policy: None,
//!     })
//!     .await?;
//!
//! let _ = (resolved.model, resolved.reasoning, resolved.permission_policy);
//! # Ok(())
//! # }
//! ```
//!
//! Build a session event stream with hooks:
//!
//! ```rust,no_run
//! use futures::{StreamExt, stream};
//! use std::path::PathBuf;
//! use std::sync::Arc;
//! use unified_agent_sdk::{
//!     AgentEvent, AgentSession, CodexLogNormalizer, EventType, ExecutorType, HookManager,
//!     session::RawLogStream,
//! };
//!
//! # async fn run() {
//! let session = AgentSession {
//!     session_id: "demo-session".to_string(),
//!     executor_type: ExecutorType::Codex,
//!     working_dir: PathBuf::from("."),
//!     created_at: chrono::Utc::now(),
//!     last_message_id: None,
//!     context_window_override_tokens: None,
//! };
//!
//! let hooks = Arc::new(HookManager::new());
//! hooks.register(
//!     EventType::MessageReceived,
//!     Arc::new(|event| Box::pin(async move {
//!         if let AgentEvent::MessageReceived { content, .. } = event {
//!             println!("message={content}");
//!         }
//!     })),
//! );
//!
//! let raw_logs: RawLogStream = Box::pin(stream::iter(vec![
//!     br#"{"type":"item.completed","item":{"type":"agent_message","id":"m1","text":"hello"}}"#
//!         .to_vec(),
//!     b"\n".to_vec(),
//! ]));
//!
//! let mut stream = session.event_stream(raw_logs, Box::new(CodexLogNormalizer::new()), Some(hooks));
//! let _ = stream.next().await;
//! # }
//! ```

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
pub use providers::{
    ClaudeCodeExecutor, ClaudeCodeLogNormalizer, CodexExecutor, CodexLogNormalizer,
};
pub use session::{AgentSession, SessionMetadata, SessionResume};
pub use types::{
    ContextUsage, ContextUsageSource, ExecutorType, ExitStatus, PermissionPolicy, Role, ToolStatus,
};
