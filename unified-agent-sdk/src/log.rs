//! Log normalization and storage

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{error::Result, types::{Role, ToolStatus}};

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

/// Log storage abstraction
#[async_trait]
pub trait LogStorage: Send + Sync {
    /// Store raw log chunk
    async fn store_raw(&self, session_id: &str, chunk: &[u8]) -> Result<()>;

    /// Store normalized log
    async fn store_normalized(&self, session_id: &str, log: &NormalizedLog) -> Result<()>;

    /// Read raw logs
    async fn read_raw(&self, session_id: &str) -> Result<Vec<u8>>;

    /// Read normalized logs
    async fn read_normalized(&self, session_id: &str) -> Result<Vec<NormalizedLog>>;
}

/// Log normalizer trait
pub trait LogNormalizer: Send + Sync {
    /// Process raw log chunk and return normalized logs
    fn normalize(&mut self, chunk: &[u8]) -> Vec<NormalizedLog>;

    /// Flush any buffered state
    fn flush(&mut self) -> Vec<NormalizedLog>;
}
