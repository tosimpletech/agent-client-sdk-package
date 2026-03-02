//! # Codex SDK for Rust
//!
//! Rust implementation of the Codex SDK that wraps the `codex` CLI and exchanges
//! JSONL events over stdin/stdout.

pub mod codex;
pub mod codex_options;
pub mod errors;
pub mod events;
pub mod exec;
pub mod items;
pub mod output_schema_file;
pub mod thread;
pub mod thread_options;
pub mod turn_options;

pub use codex::Codex;
pub use codex_options::{CodexConfigObject, CodexConfigValue, CodexOptions};
pub use errors::{Error, Result};
pub use events::{
    ItemCompletedEvent, ItemStartedEvent, ItemUpdatedEvent, ThreadError, ThreadErrorEvent,
    ThreadEvent, ThreadStartedEvent, TurnCompletedEvent, TurnFailedEvent, TurnStartedEvent, Usage,
};
pub use exec::{CodexExec, CodexExecArgs};
pub use items::{
    AgentMessageItem, CommandExecutionItem, CommandExecutionStatus, ErrorItem, FileChangeItem,
    FileUpdateChange, McpToolCallError, McpToolCallItem, McpToolCallResult, McpToolCallStatus,
    PatchApplyStatus, PatchChangeKind, ReasoningItem, ThreadItem, TodoItem, TodoListItem,
    WebSearchItem,
};
pub use thread::{Input, RunResult, RunStreamedResult, Thread, Turn, UserInput};
pub use thread_options::{
    ApprovalMode, ModelReasoningEffort, SandboxMode, ThreadOptions, WebSearchMode,
};
pub use turn_options::TurnOptions;

/// The version of the Codex Rust SDK, sourced from `Cargo.toml`.
pub const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");
