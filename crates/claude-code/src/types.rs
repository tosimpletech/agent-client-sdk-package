//! Core data types for the Claude Code SDK.
//!
//! This module defines all the configuration, message, and permission types used
//! throughout the SDK. These types correspond to the Python SDK's type definitions
//! documented at <https://platform.claude.com/docs/en/agent-sdk/python>.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::Error;
use crate::sdk_mcp::McpSdkServer;

/// Permission mode controlling how Claude Code handles tool execution permissions.
///
/// Corresponds to the Python SDK's `PermissionMode` literal type.
///
/// # Variants
///
/// - `Default` — Standard permission behavior; Claude prompts for approval on sensitive operations.
/// - `AcceptEdits` — Auto-accept file edits without prompting.
/// - `Plan` — Planning mode; Claude describes actions without executing them.
/// - `BypassPermissions` — Bypass all permission checks. **Use with caution.**
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

/// Controls which filesystem-based configuration sources the SDK loads settings from.
///
/// When `setting_sources` is omitted or `None` in [`ClaudeAgentOptions`], the SDK does
/// **not** load any filesystem settings, providing isolation for SDK applications.
///
/// # Variants
///
/// - `User` — Global user settings (`~/.claude/settings.json`).
/// - `Project` — Shared project settings (`.claude/settings.json`), version controlled.
///   Must be included to load `CLAUDE.md` files.
/// - `Local` — Local project settings (`.claude/settings.local.json`), typically gitignored.
///
/// # Precedence
///
/// When multiple sources are loaded, settings merge with this precedence (highest first):
/// 1. Local settings
/// 2. Project settings
/// 3. User settings
///
/// Programmatic options (e.g., `agents`, `allowed_tools`) always override filesystem settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SettingSource {
    User,
    Project,
    Local,
}

/// Preset configuration for the system prompt.
///
/// Uses Claude Code's built-in system prompt with an optional appended section.
///
/// # Fields
///
/// - `type_` — Must be `"preset"`.
/// - `preset` — Must be `"claude_code"` to use Claude Code's system prompt.
/// - `append` — Optional additional instructions to append to the preset system prompt.
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

/// Preset tools configuration for using Claude Code's default tool set.
///
/// # Fields
///
/// - `type_` — Must be `"preset"`.
/// - `preset` — Must be `"claude_code"` for the default tool set.
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

/// System prompt configuration.
///
/// Either provide a custom text prompt or use Claude Code's preset system prompt.
///
/// # Variants
///
/// - `Text` — A custom system prompt string.
/// - `Preset` — Use Claude Code's built-in system prompt via [`SystemPromptPreset`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum SystemPrompt {
    Text(String),
    Preset(SystemPromptPreset),
}

/// Tools configuration.
///
/// Either provide an explicit list of tool names or use Claude Code's preset tools.
///
/// # Variants
///
/// - `List` — An explicit list of tool name strings.
/// - `Preset` — Use Claude Code's default tool set via [`ToolsPreset`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ToolsOption {
    List(Vec<String>),
    Preset(ToolsPreset),
}

/// Configuration for a programmatically defined subagent.
///
/// Subagents are specialized agents that can be invoked by the main Claude Code agent
/// for specific tasks.
///
/// # Fields
///
/// - `description` — Natural language description of when to use this agent.
/// - `prompt` — The agent's system prompt.
/// - `tools` — Optional list of allowed tool names. If omitted, inherits all tools.
/// - `model` — Optional model override (e.g., `"sonnet"`, `"opus"`, `"haiku"`, `"inherit"`).
///   If omitted, uses the main model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentDefinition {
    pub description: String,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// A rule to add, replace, or remove in a permission update.
///
/// # Fields
///
/// - `tool_name` — The name of the tool this rule applies to.
/// - `rule_content` — Optional rule content string (e.g., a glob pattern or path).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRuleValue {
    pub tool_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_content: Option<String>,
}

/// Destination for applying a permission update.
///
/// Determines where the permission change is persisted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PermissionUpdateDestination {
    UserSettings,
    ProjectSettings,
    LocalSettings,
    Session,
}

/// Behavior for rule-based permission operations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PermissionBehavior {
    Allow,
    Deny,
    Ask,
}

/// The type of a permission update operation.
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

/// Configuration for updating permissions programmatically.
///
/// Used to modify permission rules, change modes, or manage directory access
/// during a session.
///
/// # Fields
///
/// - `type_` — The type of permission update operation.
/// - `rules` — Rules for add/replace/remove operations.
/// - `behavior` — Behavior for rule-based operations (`"allow"`, `"deny"`, `"ask"`).
/// - `mode` — Mode for `SetMode` operations.
/// - `directories` — Directories for add/remove directory operations.
/// - `destination` — Where to apply the permission update.
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
    /// Converts this permission update to a JSON value suitable for the CLI protocol.
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

/// Context information passed to tool permission callbacks.
///
/// Provides additional context when the [`CanUseToolCallback`] is invoked, including
/// permission update suggestions from the CLI.
///
/// # Fields
///
/// - `suggestions` — Permission update suggestions from the CLI for the user to consider.
#[derive(Debug, Clone, Default)]
pub struct ToolPermissionContext {
    pub suggestions: Vec<PermissionUpdate>,
}

/// Result indicating the tool call should be allowed.
///
/// Returned from a [`CanUseToolCallback`] to approve tool execution.
///
/// # Fields
///
/// - `updated_input` — Optional modified input to use instead of the original.
/// - `updated_permissions` — Optional permission updates to apply alongside this approval.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PermissionResultAllow {
    pub updated_input: Option<Value>,
    pub updated_permissions: Option<Vec<PermissionUpdate>>,
}

/// Result indicating the tool call should be denied.
///
/// Returned from a [`CanUseToolCallback`] to reject tool execution.
///
/// # Fields
///
/// - `message` — Message explaining why the tool was denied.
/// - `interrupt` — Whether to interrupt the current execution entirely.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PermissionResultDeny {
    pub message: String,
    pub interrupt: bool,
}

/// Union type for permission callback results.
///
/// Returned by [`CanUseToolCallback`] functions to indicate whether a tool call
/// should be allowed or denied.
#[derive(Debug, Clone, PartialEq)]
pub enum PermissionResult {
    Allow(PermissionResultAllow),
    Deny(PermissionResultDeny),
}

/// Callback type for custom tool permission logic.
///
/// This function is invoked before each tool execution, receiving:
/// - `tool_name` (`String`) — The name of the tool being called.
/// - `input_data` (`Value`) — The tool's input parameters.
/// - `context` ([`ToolPermissionContext`]) — Additional context including permission suggestions.
///
/// Returns a [`PermissionResult`] indicating whether the tool call should be allowed or denied.
///
/// # Note
///
/// When using `can_use_tool`, the prompt must be provided as streaming messages
/// (not a plain text string), and `permission_prompt_tool_name` must not be set.
pub type CanUseToolCallback = Arc<
    dyn Fn(
            String,
            Value,
            ToolPermissionContext,
        ) -> BoxFuture<'static, std::result::Result<PermissionResult, Error>>
        + Send
        + Sync,
>;

/// Context information passed to hook callbacks.
///
/// Currently a marker type; reserved for future abort signal support.
#[derive(Debug, Clone, Default)]
pub struct HookContext;

/// Input data passed to hook callbacks.
///
/// A raw JSON value whose structure depends on the hook event type (e.g.,
/// `PreToolUse`, `PostToolUse`, `UserPromptSubmit`, etc.).
/// See the [hooks documentation](https://platform.claude.com/docs/en/agent-sdk/hooks)
/// for the expected shapes per event.
pub type HookInput = Value;

/// Return value from hook callbacks.
///
/// A JSON value that may contain control fields such as:
/// - `decision` — `"block"` to block the action.
/// - `systemMessage` — A system message to add to the transcript.
/// - `hookSpecificOutput` — Hook-specific output data.
/// - `continue_` — Whether to proceed (maps to `"continue"` in the CLI protocol).
/// - `async_` — Set to `true` to defer execution (maps to `"async"` in the CLI protocol).
pub type HookJSONOutput = Value;

/// Callback type for hook functions.
///
/// Invoked when a matching hook event occurs. Receives:
/// - `input` ([`HookInput`]) — Event-specific input data.
/// - `tool_use_id` (`Option<String>`) — Optional tool use identifier (for tool-related hooks).
/// - `context` ([`HookContext`]) — Hook context with additional information.
///
/// Returns a [`HookJSONOutput`] JSON value with optional control and output fields.
pub type HookCallback = Arc<
    dyn Fn(
            HookInput,
            Option<String>,
            HookContext,
        ) -> BoxFuture<'static, std::result::Result<HookJSONOutput, Error>>
        + Send
        + Sync,
>;

/// Configuration for matching hooks to specific events or tools.
///
/// # Fields
///
/// - `matcher` — Optional tool name or regex pattern to match (e.g., `"Bash"`, `"Write|Edit"`).
///   If `None`, the hook applies to all tools.
/// - `hooks` — List of callback functions to execute when matched.
/// - `timeout` — Optional timeout in seconds for all hooks in this matcher (default: 60).
#[derive(Clone, Default)]
pub struct HookMatcher {
    pub matcher: Option<String>,
    pub hooks: Vec<HookCallback>,
    pub timeout: Option<f64>,
}

/// Configuration for an MCP server using stdio transport.
///
/// Launches an external process and communicates via stdin/stdout.
///
/// # Fields
///
/// - `type_` — Optional; set to `"stdio"` for explicit typing (backwards compatible if omitted).
/// - `command` — The command to execute.
/// - `args` — Optional command-line arguments.
/// - `env` — Optional environment variables to set.
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

/// Configuration for an MCP server using Server-Sent Events (SSE) transport.
///
/// # Fields
///
/// - `type_` — Must be `"sse"`.
/// - `url` — The SSE endpoint URL.
/// - `headers` — Optional HTTP headers to include in requests.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpSSEServerConfig {
    #[serde(rename = "type")]
    pub type_: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
}

/// Configuration for an MCP server using HTTP transport.
///
/// # Fields
///
/// - `type_` — Must be `"http"`.
/// - `url` — The HTTP endpoint URL.
/// - `headers` — Optional HTTP headers to include in requests.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpHttpServerConfig {
    #[serde(rename = "type")]
    pub type_: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
}

/// Configuration for an in-process SDK MCP server.
///
/// Created via [`create_sdk_mcp_server()`](crate::create_sdk_mcp_server). The server
/// runs within your Rust application and handles tool calls in-process.
///
/// # Fields
///
/// - `type_` — Always `"sdk"`.
/// - `name` — Unique name identifier for the server.
/// - `instance` — Shared reference to the [`McpSdkServer`] instance.
#[derive(Clone)]
pub struct McpSdkServerConfig {
    pub type_: String,
    pub name: String,
    pub instance: Arc<McpSdkServer>,
}

/// Union type for MCP server configurations.
///
/// Supports four transport types for MCP (Model Context Protocol) servers:
///
/// - `Stdio` — External process communicating via stdin/stdout.
/// - `Sse` — Remote server using Server-Sent Events.
/// - `Http` — Remote server using HTTP.
/// - `Sdk` — In-process server running within your application.
#[derive(Clone)]
pub enum McpServerConfig {
    Stdio(McpStdioServerConfig),
    Sse(McpSSEServerConfig),
    Http(McpHttpServerConfig),
    Sdk(McpSdkServerConfig),
}

impl McpServerConfig {
    /// Converts this configuration to a JSON value for the CLI protocol.
    ///
    /// SDK-type servers are serialized as `{"type": "sdk", "name": "<name>"}` since
    /// the actual server instance runs in-process and doesn't need full serialization.
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

/// MCP server configuration option for [`ClaudeAgentOptions`].
///
/// # Variants
///
/// - `None` — No MCP servers configured (default).
/// - `Servers` — A map of server name to [`McpServerConfig`].
/// - `Raw` — A raw JSON string or file path to an MCP configuration.
#[derive(Clone, Default)]
pub enum McpServersOption {
    #[default]
    None,
    Servers(HashMap<String, McpServerConfig>),
    Raw(String),
}

/// Configuration for loading plugins in the SDK.
///
/// Only local plugins are currently supported.
///
/// # Fields
///
/// - `type_` — Must be `"local"`.
/// - `path` — Absolute or relative path to the plugin directory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SdkPluginConfig {
    #[serde(rename = "type")]
    pub type_: String,
    pub path: String,
}

/// Network-specific configuration for sandbox mode.
///
/// Controls how sandboxed processes can access network resources.
///
/// # Fields
///
/// - `allow_unix_sockets` — Unix socket paths that processes can access (e.g., Docker socket).
/// - `allow_all_unix_sockets` — Allow access to all Unix sockets.
/// - `allow_local_binding` — Allow processes to bind to local ports (e.g., for dev servers).
/// - `http_proxy_port` — HTTP proxy port for network requests.
/// - `socks_proxy_port` — SOCKS proxy port for network requests.
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

/// Configuration for ignoring specific sandbox violations.
///
/// # Fields
///
/// - `file` — File path patterns to ignore violations for.
/// - `network` — Network patterns to ignore violations for.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SandboxIgnoreViolations {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<Vec<String>>,
}

/// Sandbox configuration for controlling command execution isolation.
///
/// Use this to enable command sandboxing and configure network restrictions
/// programmatically.
///
/// # Fields
///
/// - `enabled` — Enable sandbox mode for command execution.
/// - `auto_allow_bash_if_sandboxed` — Auto-approve bash commands when sandbox is enabled.
/// - `excluded_commands` — Commands that always bypass sandbox restrictions (e.g., `["docker"]`).
/// - `allow_unsandboxed_commands` — Allow the model to request running commands outside the sandbox.
/// - `network` — Network-specific sandbox configuration.
/// - `ignore_violations` — Configure which sandbox violations to ignore.
/// - `enable_weaker_nested_sandbox` — Enable a weaker nested sandbox for compatibility.
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

/// A text content block in an assistant message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TextBlock {
    pub text: String,
}

/// A thinking content block (for models with extended thinking capability).
///
/// Contains the model's internal reasoning and a cryptographic signature.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThinkingBlock {
    pub thinking: String,
    pub signature: String,
}

/// A tool use request block.
///
/// Represents Claude's request to invoke a specific tool with given parameters.
///
/// # Fields
///
/// - `id` — Unique identifier for this tool use request.
/// - `name` — Name of the tool to invoke.
/// - `input` — JSON input parameters for the tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolUseBlock {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// A tool execution result block.
///
/// Contains the output from a previously executed tool.
///
/// # Fields
///
/// - `tool_use_id` — The ID of the [`ToolUseBlock`] this result corresponds to.
/// - `content` — Optional result content (text or structured data).
/// - `is_error` — Whether the tool execution resulted in an error.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolResultBlock {
    pub tool_use_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

/// Union type for all content block types in messages.
///
/// Content blocks make up the body of [`AssistantMessage`] and [`UserMessage`] responses.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ContentBlock {
    Text(TextBlock),
    Thinking(ThinkingBlock),
    ToolUse(ToolUseBlock),
    ToolResult(ToolResultBlock),
}

/// User message content — either plain text or structured content blocks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum UserContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

/// A user input message.
///
/// # Fields
///
/// - `content` — Message content as text or content blocks.
/// - `uuid` — Optional unique message identifier.
/// - `parent_tool_use_id` — Tool use ID if this message is a tool result response.
/// - `tool_use_result` — Tool result data if applicable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserMessage {
    pub content: UserContent,
    pub uuid: Option<String>,
    pub parent_tool_use_id: Option<String>,
    pub tool_use_result: Option<Value>,
}

/// An assistant response message with content blocks.
///
/// # Fields
///
/// - `content` — List of content blocks in the response.
/// - `model` — The model that generated this response.
/// - `parent_tool_use_id` — Tool use ID if this is a nested subagent response.
/// - `error` — Error type string if the response encountered an error
///   (e.g., `"authentication_failed"`, `"rate_limit"`, `"server_error"`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistantMessage {
    pub content: Vec<ContentBlock>,
    pub model: String,
    pub parent_tool_use_id: Option<String>,
    pub error: Option<String>,
}

/// A system message with metadata.
///
/// # Fields
///
/// - `subtype` — The system message subtype identifier.
/// - `data` — The full raw data of the system message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SystemMessage {
    pub subtype: String,
    pub data: Value,
}

/// Final result message with cost and usage information.
///
/// This is the last message received for a query, containing summary statistics.
///
/// # Fields
///
/// - `subtype` — The result subtype (e.g., `"success"`, `"error"`).
/// - `duration_ms` — Total wall-clock duration in milliseconds.
/// - `duration_api_ms` — Time spent in API calls in milliseconds.
/// - `is_error` — Whether the query resulted in an error.
/// - `num_turns` — Number of conversation turns in the query.
/// - `session_id` — The session identifier.
/// - `total_cost_usd` — Optional total cost in USD.
/// - `usage` — Optional token usage breakdown (input_tokens, output_tokens,
///   cache_creation_input_tokens, cache_read_input_tokens).
/// - `result` — Optional result text.
/// - `structured_output` — Optional structured output if `output_format` was configured.
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

/// Stream event for partial message updates during streaming.
///
/// Only received when `include_partial_messages` is set to `true` in [`ClaudeAgentOptions`].
///
/// # Fields
///
/// - `uuid` — Unique identifier for this event.
/// - `session_id` — Session identifier.
/// - `event` — The raw Claude API stream event data.
/// - `parent_tool_use_id` — Parent tool use ID if this event is from a subagent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StreamEvent {
    pub uuid: String,
    pub session_id: String,
    pub event: Value,
    pub parent_tool_use_id: Option<String>,
}

/// Union type of all possible messages from the Claude Code CLI.
///
/// When receiving messages via [`ClaudeSdkClient::receive_message()`](crate::ClaudeSdkClient::receive_message)
/// or iterating results from [`query()`](crate::query), each message will be one of these variants.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Message {
    /// A user input message echoed back.
    User(UserMessage),
    /// An assistant response with content blocks.
    Assistant(AssistantMessage),
    /// A system notification or status message.
    System(SystemMessage),
    /// The final result message with cost/usage information.
    Result(ResultMessage),
    /// A partial streaming event (only when `include_partial_messages` is enabled).
    StreamEvent(StreamEvent),
}

/// Controls extended thinking behavior.
///
/// Extended thinking allows Claude to reason through complex problems before responding.
///
/// # Variants
///
/// - `Adaptive` — Claude adaptively decides when and how much to think.
/// - `Enabled { budget_tokens }` — Enable thinking with a specific token budget.
/// - `Disabled` — Disable extended thinking entirely.
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

/// MCP tool annotations providing hints about tool behavior.
///
/// These annotations help Claude and the system understand tool characteristics
/// for better permission handling and execution planning.
///
/// # Fields
///
/// - `read_only_hint` — Whether the tool only reads data without side effects.
/// - `destructive_hint` — Whether the tool performs destructive operations.
/// - `idempotent_hint` — Whether calling the tool multiple times has the same effect as once.
/// - `open_world_hint` — Whether the tool interacts with external systems.
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

/// Main configuration for Claude Code queries and sessions.
///
/// This is the primary configuration struct passed to [`query()`](crate::query) or
/// [`ClaudeSdkClient::new()`](crate::ClaudeSdkClient::new). All fields are optional
/// and have sensible defaults.
///
/// Corresponds to the Python SDK's `ClaudeAgentOptions` dataclass.
///
/// # Fields
///
/// | Field | Description |
/// |-------|-------------|
/// | `tools` | Tools configuration — explicit list or preset |
/// | `allowed_tools` | List of allowed tool names |
/// | `system_prompt` | System prompt — custom text or preset |
/// | `mcp_servers` | MCP server configurations |
/// | `permission_mode` | Permission mode for tool usage |
/// | `continue_conversation` | Continue the most recent conversation |
/// | `resume` | Session ID to resume |
/// | `max_turns` | Maximum conversation turns |
/// | `max_budget_usd` | Maximum budget in USD for the session |
/// | `disallowed_tools` | List of disallowed tool names |
/// | `model` | Claude model to use |
/// | `fallback_model` | Fallback model if the primary fails |
/// | `betas` | Beta features to enable |
/// | `permission_prompt_tool_name` | MCP tool name for permission prompts |
/// | `cwd` | Current working directory |
/// | `cli_path` | Custom path to the Claude Code CLI executable |
/// | `settings` | Path to settings file or inline JSON |
/// | `add_dirs` | Additional directories Claude can access |
/// | `env` | Environment variables |
/// | `extra_args` | Additional CLI arguments |
/// | `max_buffer_size` | Maximum bytes when buffering CLI stdout |
/// | `can_use_tool` | Tool permission callback function |
/// | `hooks` | Hook configurations for intercepting events |
/// | `user` | User identifier |
/// | `include_partial_messages` | Include [`StreamEvent`] partial messages |
/// | `fork_session` | Fork to new session ID when resuming |
/// | `agents` | Programmatically defined subagents |
/// | `setting_sources` | Which filesystem settings to load |
/// | `sandbox` | Sandbox configuration |
/// | `strict_settings_merge` | Fail instead of warn when sandbox/settings JSON merge fails |
/// | `plugins` | Local plugins to load |
/// | `max_thinking_tokens` | *Deprecated:* use `thinking` instead |
/// | `thinking` | Extended thinking configuration |
/// | `effort` | Effort level (`"low"`, `"medium"`, `"high"`, `"max"`) |
/// | `output_format` | Structured output format (e.g., JSON schema) |
/// | `enable_file_checkpointing` | Enable file change tracking for rewinding |
#[derive(Clone)]
pub struct ClaudeAgentOptions {
    /// Tools configuration. Use [`ToolsOption::Preset`] with [`ToolsPreset::default()`]
    /// for Claude Code's default tools.
    pub tools: Option<ToolsOption>,
    /// List of allowed tool names.
    pub allowed_tools: Vec<String>,
    /// System prompt configuration. Pass a string via [`SystemPrompt::Text`] for a custom
    /// prompt, or use [`SystemPrompt::Preset`] for Claude Code's built-in system prompt.
    pub system_prompt: Option<SystemPrompt>,
    /// MCP server configurations or path to config file.
    pub mcp_servers: McpServersOption,
    /// Permission mode for tool usage.
    pub permission_mode: Option<PermissionMode>,
    /// Continue the most recent conversation.
    pub continue_conversation: bool,
    /// Session ID to resume.
    pub resume: Option<String>,
    /// Maximum conversation turns.
    pub max_turns: Option<i64>,
    /// Maximum budget in USD for the session.
    pub max_budget_usd: Option<f64>,
    /// List of disallowed tool names.
    pub disallowed_tools: Vec<String>,
    /// Claude model to use (e.g., `"sonnet"`, `"opus"`).
    pub model: Option<String>,
    /// Fallback model to use if the primary model fails.
    pub fallback_model: Option<String>,
    /// Beta features to enable.
    pub betas: Vec<String>,
    /// MCP tool name for permission prompts. Mutually exclusive with `can_use_tool`.
    pub permission_prompt_tool_name: Option<String>,
    /// Current working directory for the Claude Code process.
    pub cwd: Option<PathBuf>,
    /// Custom path to the Claude Code CLI executable.
    pub cli_path: Option<PathBuf>,
    /// Path to settings file or inline JSON string.
    pub settings: Option<String>,
    /// Additional directories Claude can access.
    pub add_dirs: Vec<PathBuf>,
    /// Environment variables to pass to the CLI process.
    pub env: HashMap<String, String>,
    /// Additional CLI arguments to pass directly to the CLI.
    /// Keys are flag names (without `--`), values are optional flag values.
    pub extra_args: HashMap<String, Option<String>>,
    /// Maximum bytes when buffering CLI stdout. Defaults to 1MB.
    pub max_buffer_size: Option<usize>,
    /// Custom tool permission callback function.
    pub can_use_tool: Option<CanUseToolCallback>,
    /// Hook configurations for intercepting events. Keys are hook event names
    /// (e.g., `"PreToolUse"`, `"PostToolUse"`, `"UserPromptSubmit"`).
    pub hooks: Option<HashMap<String, Vec<HookMatcher>>>,
    /// User identifier.
    pub user: Option<String>,
    /// Include partial message streaming events ([`StreamEvent`]).
    pub include_partial_messages: bool,
    /// When resuming with `resume`, fork to a new session ID instead of continuing
    /// the original session.
    pub fork_session: bool,
    /// Programmatically defined subagents.
    pub agents: Option<HashMap<String, AgentDefinition>>,
    /// Control which filesystem settings to load.
    /// When omitted, no settings are loaded (SDK isolation).
    pub setting_sources: Option<Vec<SettingSource>>,
    /// Sandbox configuration for command execution isolation.
    pub sandbox: Option<SandboxSettings>,
    /// When `true`, fail command construction if sandbox merge with `settings` fails.
    /// When `false`, merge failures emit a warning and fallback to sandbox-only settings.
    pub strict_settings_merge: bool,
    /// Local plugins to load.
    pub plugins: Vec<SdkPluginConfig>,
    /// *Deprecated:* Maximum tokens for thinking blocks. Use `thinking` instead.
    pub max_thinking_tokens: Option<i64>,
    /// Extended thinking configuration. Takes precedence over `max_thinking_tokens`.
    pub thinking: Option<ThinkingConfig>,
    /// Effort level for thinking depth (`"low"`, `"medium"`, `"high"`, `"max"`).
    pub effort: Option<String>,
    /// Output format for structured responses.
    /// Example: `{"type": "json_schema", "schema": {...}}`
    pub output_format: Option<Value>,
    /// Enable file change tracking for rewinding via
    /// [`ClaudeSdkClient::rewind_files()`](crate::ClaudeSdkClient::rewind_files).
    pub enable_file_checkpointing: bool,
    /// Optional callback for stderr output lines from the CLI process.
    ///
    /// When set, stderr is piped and each non-empty line is passed to this callback.
    /// When `None`, stderr is still drained to prevent subprocess blocking, but
    /// lines are discarded.
    pub stderr: Option<StderrCallback>,
}

/// Callback type for receiving stderr output lines from the CLI process.
pub type StderrCallback = Arc<dyn Fn(String) + Send + Sync>;

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
            strict_settings_merge: false,
            plugins: Vec::new(),
            max_thinking_tokens: None,
            thinking: None,
            effort: None,
            output_format: None,
            enable_file_checkpointing: false,
            stderr: None,
        }
    }
}
