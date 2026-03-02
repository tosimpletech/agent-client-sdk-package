use serde::{Deserialize, Serialize};

use crate::items::ThreadItem;

/// Token usage for a completed turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
}

/// Turn failure payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThreadError {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThreadStartedEvent {
    pub thread_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnStartedEvent;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnCompletedEvent {
    pub usage: Usage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnFailedEvent {
    pub error: ThreadError,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ItemStartedEvent {
    pub item: ThreadItem,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ItemUpdatedEvent {
    pub item: ThreadItem,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ItemCompletedEvent {
    pub item: ThreadItem,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThreadErrorEvent {
    pub message: String,
}

/// Top-level JSONL events emitted by `codex exec --experimental-json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum ThreadEvent {
    #[serde(rename = "thread.started")]
    ThreadStarted { thread_id: String },
    #[serde(rename = "turn.started")]
    TurnStarted,
    #[serde(rename = "turn.completed")]
    TurnCompleted { usage: Usage },
    #[serde(rename = "turn.failed")]
    TurnFailed { error: ThreadError },
    #[serde(rename = "item.started")]
    ItemStarted { item: ThreadItem },
    #[serde(rename = "item.updated")]
    ItemUpdated { item: ThreadItem },
    #[serde(rename = "item.completed")]
    ItemCompleted { item: ThreadItem },
    #[serde(rename = "error")]
    Error { message: String },
}
