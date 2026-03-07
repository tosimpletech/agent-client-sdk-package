//! Claude Code provider integration.
//!
//! Use [`ClaudeCodeExecutor`] for session lifecycle operations and
//! [`ClaudeCodeLogNormalizer`] when converting Claude stream messages into
//! unified logs.

mod executor;
mod normalizer;
pub mod profile;

pub use executor::ClaudeCodeExecutor;
pub use normalizer::ClaudeCodeLogNormalizer;
