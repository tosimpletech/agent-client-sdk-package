//! Log normalization

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{Role, ToolStatus};

/// Normalized log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum NormalizedLog {
    Message {
        role: Role,
        content: String,
    },
    ToolCall {
        name: String,
        args: Value,
        status: ToolStatus,
        action: ActionType,
    },
    Thinking {
        content: String,
    },
    TokenUsage {
        total: u32,
        limit: u32,
    },
    Error {
        error_type: String,
        message: String,
    },
}

/// Tool action type
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum ActionType {
    FileRead { path: String },
    FileEdit { path: String },
    CommandRun { command: String },
    WebSearch { query: String },
    McpTool { tool: String },
    AskUser,
}

/// Log normalizer trait
pub trait LogNormalizer: Send + Sync {
    /// Process raw log chunk and return normalized logs
    fn normalize(&mut self, chunk: &[u8]) -> Vec<NormalizedLog>;

    /// Flush any buffered state
    fn flush(&mut self) -> Vec<NormalizedLog>;
}
