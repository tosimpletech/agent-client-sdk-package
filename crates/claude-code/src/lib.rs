pub mod client;
pub mod errors;
pub mod internal_client;
pub mod message_parser;
pub mod query;
pub mod query_fn;
pub mod sdk_mcp;
pub mod transport;
pub mod types;

pub use client::{ClaudeSdkClient, InputPrompt};
pub use errors::{
    CLIConnectionError, CLIJSONDecodeError, CLINotFoundError, ClaudeSDKError, Error,
    MessageParseError, ProcessError, Result,
};
pub use message_parser::parse_message;
pub use query::Query;
pub use query_fn::query;
pub use sdk_mcp::{McpSdkServer, SdkMcpTool, create_sdk_mcp_server, tool};
pub use transport::Transport;
pub use transport::subprocess_cli::{DEFAULT_MAX_BUFFER_SIZE, JsonStreamBuffer, Prompt, SubprocessCliTransport};
pub use types::{
    AgentDefinition, AssistantMessage, ClaudeAgentOptions, ContentBlock, HookCallback, HookContext,
    HookInput, HookJSONOutput, HookMatcher, McpHttpServerConfig, McpSSEServerConfig,
    McpSdkServerConfig, McpServerConfig, McpServersOption, McpStdioServerConfig, Message,
    PermissionMode, PermissionResult, PermissionResultAllow, PermissionResultDeny, PermissionUpdate,
    ResultMessage, SandboxIgnoreViolations, SandboxNetworkConfig, SandboxSettings, SdkPluginConfig,
    SettingSource, StreamEvent, SystemMessage, SystemPrompt, SystemPromptPreset, TextBlock, ThinkingBlock, ThinkingConfig,
    ToolAnnotations, ToolPermissionContext, ToolResultBlock, ToolUseBlock, ToolsOption, UserContent,
    ToolsPreset, UserMessage,
};

pub const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");
