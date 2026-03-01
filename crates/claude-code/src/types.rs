use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::Error;
use crate::sdk_mcp::McpSdkServer;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PermissionMode {
    #[serde(rename = "default")]
    Default,
    #[serde(rename = "acceptEdits")]
    AcceptEdits,
    #[serde(rename = "plan")]
    Plan,
    #[serde(rename = "bypassPermissions")]
    BypassPermissions,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SettingSource {
    User,
    Project,
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SystemPromptPreset {
    #[serde(rename = "type")]
    pub type_: String,
    pub preset: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub append: Option<String>,
}

impl Default for SystemPromptPreset {
    fn default() -> Self {
        Self {
            type_: "preset".to_string(),
            preset: "claude_code".to_string(),
            append: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolsPreset {
    #[serde(rename = "type")]
    pub type_: String,
    pub preset: String,
}

impl Default for ToolsPreset {
    fn default() -> Self {
        Self {
            type_: "preset".to_string(),
            preset: "claude_code".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum SystemPrompt {
    Text(String),
    Preset(SystemPromptPreset),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ToolsOption {
    List(Vec<String>),
    Preset(ToolsPreset),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentDefinition {
    pub description: String,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRuleValue {
    pub tool_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PermissionUpdateDestination {
    UserSettings,
    ProjectSettings,
    LocalSettings,
    Session,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PermissionBehavior {
    Allow,
    Deny,
    Ask,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PermissionUpdateType {
    AddRules,
    ReplaceRules,
    RemoveRules,
    SetMode,
    AddDirectories,
    RemoveDirectories,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PermissionUpdate {
    #[serde(rename = "type")]
    pub type_: PermissionUpdateType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rules: Option<Vec<PermissionRuleValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behavior: Option<PermissionBehavior>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<PermissionMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directories: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<PermissionUpdateDestination>,
}

impl PermissionUpdate {
    pub fn to_cli_dict(&self) -> Value {
        let mut result = serde_json::Map::new();
        result.insert(
            "type".to_string(),
            serde_json::to_value(&self.type_).unwrap_or(Value::Null),
        );

        if let Some(destination) = &self.destination {
            result.insert(
                "destination".to_string(),
                serde_json::to_value(destination).unwrap_or(Value::Null),
            );
        }

        match self.type_ {
            PermissionUpdateType::AddRules
            | PermissionUpdateType::ReplaceRules
            | PermissionUpdateType::RemoveRules => {
                if let Some(rules) = &self.rules {
                    let rules_json: Vec<Value> = rules
                        .iter()
                        .map(|rule| {
                            serde_json::json!({
                                "toolName": rule.tool_name,
                                "ruleContent": rule.rule_content
                            })
                        })
                        .collect();
                    result.insert("rules".to_string(), Value::Array(rules_json));
                }
                if let Some(behavior) = &self.behavior {
                    result.insert(
                        "behavior".to_string(),
                        serde_json::to_value(behavior).unwrap_or(Value::Null),
                    );
                }
            }
            PermissionUpdateType::SetMode => {
                if let Some(mode) = &self.mode {
                    result.insert(
                        "mode".to_string(),
                        serde_json::to_value(mode).unwrap_or(Value::Null),
                    );
                }
            }
            PermissionUpdateType::AddDirectories | PermissionUpdateType::RemoveDirectories => {
                if let Some(directories) = &self.directories {
                    result.insert(
                        "directories".to_string(),
                        serde_json::to_value(directories).unwrap_or(Value::Null),
                    );
                }
            }
        }

        Value::Object(result)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ToolPermissionContext {
    pub suggestions: Vec<PermissionUpdate>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PermissionResultAllow {
    pub updated_input: Option<Value>,
    pub updated_permissions: Option<Vec<PermissionUpdate>>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PermissionResultDeny {
    pub message: String,
    pub interrupt: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PermissionResult {
    Allow(PermissionResultAllow),
    Deny(PermissionResultDeny),
}

pub type CanUseToolCallback = Arc<
    dyn Fn(
            String,
            Value,
            ToolPermissionContext,
        ) -> BoxFuture<'static, std::result::Result<PermissionResult, Error>>
        + Send
        + Sync,
>;

#[derive(Debug, Clone, Default)]
pub struct HookContext;

pub type HookInput = Value;
pub type HookJSONOutput = Value;

pub type HookCallback = Arc<
    dyn Fn(
            HookInput,
            Option<String>,
            HookContext,
        ) -> BoxFuture<'static, std::result::Result<HookJSONOutput, Error>>
        + Send
        + Sync,
>;

#[derive(Clone, Default)]
pub struct HookMatcher {
    pub matcher: Option<String>,
    pub hooks: Vec<HookCallback>,
    pub timeout: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpStdioServerConfig {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_: Option<String>,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpSSEServerConfig {
    #[serde(rename = "type")]
    pub type_: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpHttpServerConfig {
    #[serde(rename = "type")]
    pub type_: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
}

#[derive(Clone)]
pub struct McpSdkServerConfig {
    pub type_: String,
    pub name: String,
    pub instance: Arc<McpSdkServer>,
}

#[derive(Clone)]
pub enum McpServerConfig {
    Stdio(McpStdioServerConfig),
    Sse(McpSSEServerConfig),
    Http(McpHttpServerConfig),
    Sdk(McpSdkServerConfig),
}

impl McpServerConfig {
    pub fn to_cli_json(&self) -> Value {
        match self {
            McpServerConfig::Stdio(config) => serde_json::to_value(config).unwrap_or(Value::Null),
            McpServerConfig::Sse(config) => serde_json::to_value(config).unwrap_or(Value::Null),
            McpServerConfig::Http(config) => serde_json::to_value(config).unwrap_or(Value::Null),
            McpServerConfig::Sdk(config) => {
                serde_json::json!({
                    "type": "sdk",
                    "name": config.name
                })
            }
        }
    }
}

#[derive(Clone, Default)]
pub enum McpServersOption {
    #[default]
    None,
    Servers(HashMap<String, McpServerConfig>),
    Raw(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SdkPluginConfig {
    #[serde(rename = "type")]
    pub type_: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SandboxNetworkConfig {
    #[serde(rename = "allowUnixSockets", skip_serializing_if = "Option::is_none")]
    pub allow_unix_sockets: Option<Vec<String>>,
    #[serde(
        rename = "allowAllUnixSockets",
        skip_serializing_if = "Option::is_none"
    )]
    pub allow_all_unix_sockets: Option<bool>,
    #[serde(rename = "allowLocalBinding", skip_serializing_if = "Option::is_none")]
    pub allow_local_binding: Option<bool>,
    #[serde(rename = "httpProxyPort", skip_serializing_if = "Option::is_none")]
    pub http_proxy_port: Option<u16>,
    #[serde(rename = "socksProxyPort", skip_serializing_if = "Option::is_none")]
    pub socks_proxy_port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SandboxIgnoreViolations {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SandboxSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(
        rename = "autoAllowBashIfSandboxed",
        skip_serializing_if = "Option::is_none"
    )]
    pub auto_allow_bash_if_sandboxed: Option<bool>,
    #[serde(rename = "excludedCommands", skip_serializing_if = "Option::is_none")]
    pub excluded_commands: Option<Vec<String>>,
    #[serde(
        rename = "allowUnsandboxedCommands",
        skip_serializing_if = "Option::is_none"
    )]
    pub allow_unsandboxed_commands: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<SandboxNetworkConfig>,
    #[serde(rename = "ignoreViolations", skip_serializing_if = "Option::is_none")]
    pub ignore_violations: Option<SandboxIgnoreViolations>,
    #[serde(
        rename = "enableWeakerNestedSandbox",
        skip_serializing_if = "Option::is_none"
    )]
    pub enable_weaker_nested_sandbox: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TextBlock {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThinkingBlock {
    pub thinking: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolUseBlock {
    pub id: String,
    pub name: String,
    pub input: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolResultBlock {
    pub tool_use_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ContentBlock {
    Text(TextBlock),
    Thinking(ThinkingBlock),
    ToolUse(ToolUseBlock),
    ToolResult(ToolResultBlock),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum UserContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserMessage {
    pub content: UserContent,
    pub uuid: Option<String>,
    pub parent_tool_use_id: Option<String>,
    pub tool_use_result: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistantMessage {
    pub content: Vec<ContentBlock>,
    pub model: String,
    pub parent_tool_use_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SystemMessage {
    pub subtype: String,
    pub data: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResultMessage {
    pub subtype: String,
    pub duration_ms: i64,
    pub duration_api_ms: i64,
    pub is_error: bool,
    pub num_turns: i64,
    pub session_id: String,
    pub total_cost_usd: Option<f64>,
    pub usage: Option<Value>,
    pub result: Option<String>,
    pub structured_output: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StreamEvent {
    pub uuid: String,
    pub session_id: String,
    pub event: Value,
    pub parent_tool_use_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Message {
    User(UserMessage),
    Assistant(AssistantMessage),
    System(SystemMessage),
    Result(ResultMessage),
    StreamEvent(StreamEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum ThinkingConfig {
    #[serde(rename = "adaptive")]
    Adaptive,
    #[serde(rename = "enabled")]
    Enabled { budget_tokens: i64 },
    #[serde(rename = "disabled")]
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ToolAnnotations {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_only_hint: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destructive_hint: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotent_hint: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_world_hint: Option<bool>,
}

#[derive(Clone)]
pub struct ClaudeAgentOptions {
    pub tools: Option<ToolsOption>,
    pub allowed_tools: Vec<String>,
    pub system_prompt: Option<SystemPrompt>,
    pub mcp_servers: McpServersOption,
    pub permission_mode: Option<PermissionMode>,
    pub continue_conversation: bool,
    pub resume: Option<String>,
    pub max_turns: Option<i64>,
    pub max_budget_usd: Option<f64>,
    pub disallowed_tools: Vec<String>,
    pub model: Option<String>,
    pub fallback_model: Option<String>,
    pub betas: Vec<String>,
    pub permission_prompt_tool_name: Option<String>,
    pub cwd: Option<PathBuf>,
    pub cli_path: Option<PathBuf>,
    pub settings: Option<String>,
    pub add_dirs: Vec<PathBuf>,
    pub env: HashMap<String, String>,
    pub extra_args: HashMap<String, Option<String>>,
    pub max_buffer_size: Option<usize>,
    pub can_use_tool: Option<CanUseToolCallback>,
    pub hooks: Option<HashMap<String, Vec<HookMatcher>>>,
    pub user: Option<String>,
    pub include_partial_messages: bool,
    pub fork_session: bool,
    pub agents: Option<HashMap<String, AgentDefinition>>,
    pub setting_sources: Option<Vec<SettingSource>>,
    pub sandbox: Option<SandboxSettings>,
    pub plugins: Vec<SdkPluginConfig>,
    pub max_thinking_tokens: Option<i64>,
    pub thinking: Option<ThinkingConfig>,
    pub effort: Option<String>,
    pub output_format: Option<Value>,
    pub enable_file_checkpointing: bool,
}

impl Default for ClaudeAgentOptions {
    fn default() -> Self {
        Self {
            tools: None,
            allowed_tools: Vec::new(),
            system_prompt: None,
            mcp_servers: McpServersOption::None,
            permission_mode: None,
            continue_conversation: false,
            resume: None,
            max_turns: None,
            max_budget_usd: None,
            disallowed_tools: Vec::new(),
            model: None,
            fallback_model: None,
            betas: Vec::new(),
            permission_prompt_tool_name: None,
            cwd: None,
            cli_path: None,
            settings: None,
            add_dirs: Vec::new(),
            env: HashMap::new(),
            extra_args: HashMap::new(),
            max_buffer_size: None,
            can_use_tool: None,
            hooks: None,
            user: None,
            include_partial_messages: false,
            fork_session: false,
            agents: None,
            setting_sources: None,
            sandbox: None,
            plugins: Vec::new(),
            max_thinking_tokens: None,
            thinking: None,
            effort: None,
            output_format: None,
            enable_file_checkpointing: false,
        }
    }
}
