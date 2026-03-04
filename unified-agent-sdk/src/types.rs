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

/// Process exit status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitStatus {
    /// Numeric exit code if available.
    pub code: Option<i32>,
    /// Whether the process reported success.
    pub success: bool,
}
