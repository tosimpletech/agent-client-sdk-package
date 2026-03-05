//! Log normalization

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{Role, ToolStatus};

/// Normalized log entry
///
/// This enum is intentionally non-exhaustive for forward compatibility.
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

/// Tool action type
///
/// This enum is intentionally non-exhaustive for forward compatibility.
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

/// Log normalizer trait
pub trait LogNormalizer: Send {
    /// Process raw log chunk and return normalized logs
    fn normalize(&mut self, chunk: &[u8]) -> Vec<NormalizedLog>;

    /// Flush any buffered state
    fn flush(&mut self) -> Vec<NormalizedLog>;
}
