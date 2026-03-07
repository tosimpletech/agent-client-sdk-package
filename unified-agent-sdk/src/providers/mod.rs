//! Provider implementations for specific agent SDKs.
//!
//! Each provider module exposes:
//! - an [`crate::executor::AgentExecutor`] implementation
//! - a provider-specific [`crate::log::LogNormalizer`] implementation
//! - profile discovery helpers for model/reasoning probing

pub mod claude_code;
pub mod codex;

pub use claude_code::{ClaudeCodeExecutor, ClaudeCodeLogNormalizer};
pub use codex::{CodexExecutor, CodexLogNormalizer};
