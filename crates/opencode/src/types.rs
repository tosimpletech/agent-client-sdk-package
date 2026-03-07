use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Input for creating a new session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCreateInput {
    pub parent_id: Option<String>,
    pub title: Option<String>,
    pub permission: Option<Value>,
}

/// A prompt part payload for session message input.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PartInput {
    /// Free-form part object for forward compatibility with official schema.
    Raw(Value),
}

/// Input payload for session prompt/prompt_async endpoints.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptInput {
    pub message_id: Option<String>,
    pub model: Option<Value>,
    pub agent: Option<String>,
    pub no_reply: Option<bool>,
    pub tools: Option<Value>,
    pub format: Option<Value>,
    pub system: Option<String>,
    pub variant: Option<String>,
    pub parts: Vec<PartInput>,
}
