//! Core type definitions

use serde::{Deserialize, Serialize};

/// Executor type identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExecutorType {
    ClaudeCode,
    Codex,
}

/// Message role
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    User,
    Assistant,
    System,
}

/// Tool execution status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolStatus {
    Started,
    Running,
    Completed,
    Failed,
}

/// Permission policy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionPolicy {
    Bypass,
    Prompt,
    Deny,
}

/// Process exit status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitStatus {
    pub code: Option<i32>,
    pub success: bool,
}
