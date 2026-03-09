//! Core public types shared across unified executors, sessions, and events.
//!
//! These types are intentionally provider-agnostic. They model concepts that stay
//! stable even when the backing agent changes from Codex to Claude Code or future
//! providers.

use serde::{Deserialize, Serialize};

/// Provider identifier used throughout the unified SDK.
///
/// This enum is used in profile resolution, session metadata, and executor
/// selection logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExecutorType {
    /// Anthropic Claude Code executor backend.
    ClaudeCode,
    /// OpenAI Codex executor backend.
    Codex,
}

/// Logical role associated with a normalized message event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// End-user message.
    User,
    /// Assistant-generated message.
    Assistant,
    /// System-level instruction or status message.
    System,
}

/// High-level lifecycle state for a tool call after normalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolStatus {
    /// Tool call has started.
    Started,
    /// Tool call is in progress.
    Running,
    /// Tool call completed successfully.
    Completed,
    /// Tool call failed.
    Failed,
}

/// Provider-agnostic permission behavior requested by the caller.
///
/// Each executor maps this enum to its provider-specific approval or sandboxing
/// model. Unsupported combinations are surfaced as configuration errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionPolicy {
    /// Automatically allow tool execution without prompts.
    Bypass,
    /// Ask for approval when needed.
    Prompt,
    /// Deny operations that require elevated trust.
    Deny,
}

/// Source used to determine the context-window capacity reported in [`ContextUsage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextUsageSource {
    /// Window capacity reported directly by the underlying SDK/CLI stream.
    ProviderReported,
    /// Window capacity supplied through runtime SDK configuration.
    ConfigOverride,
    /// Window capacity is unknown.
    Unknown,
}

/// Unified context-window usage snapshot emitted through [`crate::event::AgentEvent::ContextUsageUpdated`].
///
/// Use this value when you need consistent usage telemetry across different
/// providers. If a provider does not report a context limit directly, the SDK may
/// derive it from `SpawnConfig::context_window_override_tokens`.
///
/// # Examples
///
/// ```rust
/// use unified_agent_sdk::{ContextUsage, ContextUsageSource};
///
/// let usage = ContextUsage {
///     used_tokens: 1200,
///     window_tokens: Some(8000),
///     remaining_tokens: Some(6800),
///     utilization: Some(0.15),
///     source: ContextUsageSource::ProviderReported,
/// };
///
/// assert_eq!(usage.remaining_tokens, Some(6800));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ContextUsage {
    /// Tokens currently consumed in the active context snapshot.
    pub used_tokens: u32,
    /// Context window capacity when known.
    pub window_tokens: Option<u32>,
    /// Remaining tokens in the context window when capacity is known.
    pub remaining_tokens: Option<u32>,
    /// Usage ratio in range `[0.0, 1.0]` when capacity is known.
    pub utilization: Option<f32>,
    /// Provenance of the reported capacity information.
    pub source: ContextUsageSource,
}

/// Exit summary for a spawned or resumed unified session.
///
/// This is intentionally simpler than provider-native process state. It captures
/// the information most callers need for orchestration and persistence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitStatus {
    /// Numeric exit code if available.
    pub code: Option<i32>,
    /// Whether the process reported success.
    pub success: bool,
}
