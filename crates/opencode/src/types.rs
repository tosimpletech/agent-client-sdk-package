use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Input for creating a new session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCreateInput {
    /// Optional parent session id when creating a child session.
    pub parent_id: Option<String>,
    /// Optional human-readable session title.
    pub title: Option<String>,
    /// Optional permission configuration payload.
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
    /// Optional message id for follow-up operations.
    pub message_id: Option<String>,
    /// Optional model selection payload.
    pub model: Option<Value>,
    /// Optional agent selector.
    pub agent: Option<String>,
    /// Whether to suppress immediate reply behavior.
    pub no_reply: Option<bool>,
    /// Optional tools configuration.
    pub tools: Option<Value>,
    /// Optional output format configuration.
    pub format: Option<Value>,
    /// Optional system instruction text.
    pub system: Option<String>,
    /// Optional prompt variant key.
    pub variant: Option<String>,
    /// Prompt parts in official schema-compatible shape.
    pub parts: Vec<PartInput>,
}
