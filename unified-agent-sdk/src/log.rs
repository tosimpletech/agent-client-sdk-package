//! Provider log normalization primitives.
//!
//! Executors emit provider-specific raw output. A [`LogNormalizer`] converts that
//! output into [`NormalizedLog`] values so the rest of the SDK can operate on one
//! stable event model.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{Role, ToolStatus};

/// Provider-agnostic log entry produced by a [`LogNormalizer`].
///
/// Normalized logs are an intermediate representation between raw provider output
/// and higher-level [`crate::event::AgentEvent`] values. The enum is intentionally
/// non-exhaustive for forward compatibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[non_exhaustive]
pub enum NormalizedLog {
    /// Message emitted by user/assistant/system.
    Message {
        /// Message role.
        role: Role,
        /// Message content.
        content: String,
    },
    /// Tool invocation state update.
    ToolCall {
        /// Tool name.
        name: String,
        /// Tool arguments or raw payload.
        args: Value,
        /// Tool execution status.
        status: ToolStatus,
        /// Unified high-level action metadata.
        action: ActionType,
    },
    /// Assistant reasoning/thinking content.
    Thinking {
        /// Reasoning text.
        content: String,
    },
    /// Token usage signal.
    TokenUsage {
        /// Total tokens consumed.
        total: u32,
        /// Token limit (if available).
        limit: u32,
    },
    /// Error signal emitted by the source normalizer.
    Error {
        /// Stable error category.
        error_type: String,
        /// Human-readable error message.
        message: String,
    },
}

/// Best-effort classification of the action a tool update represents.
///
/// This metadata is useful when downstream consumers want to distinguish file,
/// command, web, or MCP activity without parsing provider-specific payloads. The
/// enum is intentionally non-exhaustive for forward compatibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action")]
#[non_exhaustive]
pub enum ActionType {
    /// File read operation.
    FileRead {
        /// Target path.
        path: String,
    },
    /// File edit/write operation.
    FileEdit {
        /// Target path.
        path: String,
    },
    /// Shell command execution.
    CommandRun {
        /// Raw command string.
        command: String,
    },
    /// Web search operation.
    WebSearch {
        /// Search query.
        query: String,
    },
    /// MCP tool invocation.
    McpTool {
        /// Tool identifier.
        tool: String,
    },
    /// Explicit "ask user" interaction point.
    AskUser,
}

/// Trait implemented by provider adapters that translate raw output into [`NormalizedLog`] values.
///
/// Normalizers may buffer partial chunks internally. Call [`LogNormalizer::flush`]
/// when the raw stream ends to emit any trailing state.
pub trait LogNormalizer: Send {
    /// Processes one raw output chunk and returns any normalized records derived from it.
    fn normalize(&mut self, chunk: &[u8]) -> Vec<NormalizedLog>;

    /// Flushes any buffered state after the upstream raw stream has ended.
    fn flush(&mut self) -> Vec<NormalizedLog>;
}
