//! Executor adapters to concrete SDK implementations.

pub mod claude_code;
pub mod codex;

pub use claude_code::ClaudeCodeExecutor;
pub use codex::CodexExecutor;
