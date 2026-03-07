//! Codex provider integration.
//!
//! Use [`CodexExecutor`] for session lifecycle operations and
//! [`CodexLogNormalizer`] when converting Codex thread events into unified logs.

mod executor;
mod normalizer;
pub mod profile;

pub use executor::CodexExecutor;
pub use normalizer::CodexLogNormalizer;
