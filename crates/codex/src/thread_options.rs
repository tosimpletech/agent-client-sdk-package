use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ApprovalMode {
    #[serde(rename = "never")]
    Never,
    #[serde(rename = "on-request")]
    OnRequest,
    #[serde(rename = "on-failure")]
    OnFailure,
    #[serde(rename = "untrusted")]
    Untrusted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SandboxMode {
    #[serde(rename = "read-only")]
    ReadOnly,
    #[serde(rename = "workspace-write")]
    WorkspaceWrite,
    #[serde(rename = "danger-full-access")]
    DangerFullAccess,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ModelReasoningEffort {
    #[serde(rename = "minimal")]
    Minimal,
    #[serde(rename = "low")]
    Low,
    #[serde(rename = "medium")]
    Medium,
    #[serde(rename = "high")]
    High,
    #[serde(rename = "xhigh")]
    XHigh,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum WebSearchMode {
    #[serde(rename = "disabled")]
    Disabled,
    #[serde(rename = "cached")]
    Cached,
    #[serde(rename = "live")]
    Live,
}

/// Per-thread options passed to `codex exec`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThreadOptions {
    pub model: Option<String>,
    pub sandbox_mode: Option<SandboxMode>,
    pub working_directory: Option<String>,
    pub skip_git_repo_check: Option<bool>,
    pub model_reasoning_effort: Option<ModelReasoningEffort>,
    pub network_access_enabled: Option<bool>,
    pub web_search_mode: Option<WebSearchMode>,
    pub web_search_enabled: Option<bool>,
    pub approval_policy: Option<ApprovalMode>,
    pub additional_directories: Option<Vec<String>>,
}
