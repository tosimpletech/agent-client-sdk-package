//! Core type definitions

use serde::{Deserialize, Serialize};

/// Executor type identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExecutorType {
    /// Anthropic Claude Code executor backend.
    ClaudeCode,
    /// OpenAI Codex executor backend.
    Codex,
}

/// Message role
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// End-user message.
    User,
    /// Assistant-generated message.
    Assistant,
    /// System-level instruction or status message.
    System,
}

/// Tool execution status
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

/// Permission policy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionPolicy {
    /// Automatically allow tool execution without prompts.
    Bypass,
    /// Ask for approval when needed.
    Prompt,
    /// Deny operations that require elevated trust.
    Deny,
}

/// Source used to determine context window capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextUsageSource {
    /// Window capacity reported directly by the underlying SDK/CLI stream.
    ProviderReported,
    /// Window capacity supplied through runtime SDK configuration.
    ConfigOverride,
    /// Window capacity is unknown.
    Unknown,
}

/// Unified context window usage snapshot.
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

/// Process exit status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitStatus {
    /// Numeric exit code if available.
    pub code: Option<i32>,
    /// Whether the process reported success.
    pub success: bool,
}
