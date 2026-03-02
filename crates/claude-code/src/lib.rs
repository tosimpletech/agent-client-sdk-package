//! # Claude Code SDK for Rust
//!
//! A Rust implementation of the Claude Code Agent SDK, providing programmatic access
//! to Claude Code as a subprocess. This crate is a port of the official
//! [Python Claude Code SDK](https://platform.claude.com/docs/en/agent-sdk/python).
//!
//! ## Overview
//!
//! The SDK provides two main ways to interact with Claude Code:
//!
//! - **[`query()`]** — Creates a new session for each interaction. Best for one-off tasks,
//!   independent operations, and simple automation scripts.
//! - **[`ClaudeSdkClient`]** — Maintains a conversation session across multiple exchanges.
//!   Best for continuing conversations, follow-up questions, interactive applications, and
//!   session lifecycle management.
//!
//! ## Quick Comparison
//!
//! | Feature | `query()` | `ClaudeSdkClient` |
//! |---------|-----------|-------------------|
//! | Session | New session each time | Reuses same session |
//! | Conversation | Single exchange | Multiple exchanges in same context |
//! | Connection | Managed automatically | Manual control |
//! | Interrupts | Not supported | Supported |
//! | Custom Tools | Supported | Supported |
//! | Use Case | One-off tasks | Continuous conversations |
//!
//! ## Example — One-off query
//!
//! ```rust,no_run
//! use claude_code::{query, ClaudeAgentOptions, InputPrompt, Message};
//!
//! # async fn example() -> claude_code::Result<()> {
//! let messages = query(
//!     InputPrompt::Text("What is 2 + 2?".to_string()),
//!     Some(ClaudeAgentOptions {
//!         permission_mode: Some(claude_code::PermissionMode::BypassPermissions),
//!         ..Default::default()
//!     }),
//!     None,
//! ).await?;
//!
//! for msg in messages {
//!     if let Message::Assistant(assistant) = msg {
//!         println!("{:?}", assistant.content);
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Example — Continuous conversation with `ClaudeSdkClient`
//!
//! ```rust,no_run
//! use claude_code::{ClaudeSdkClient, InputPrompt};
//!
//! # async fn example() -> claude_code::Result<()> {
//! let mut client = ClaudeSdkClient::new(None, None);
//! client.connect(None).await?;
//!
//! // First question
//! client.query(InputPrompt::Text("What's the capital of France?".into()), "default").await?;
//! let response = client.receive_response().await?;
//!
//! // Follow-up — session retains context
//! client.query(InputPrompt::Text("What's the population of that city?".into()), "default").await?;
//! let follow_up = client.receive_response().await?;
//!
//! client.disconnect().await?;
//! # Ok(())
//! # }
//! ```

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
pub use query::{Query, handle_sdk_mcp_request};
pub use query_fn::{query, query_from_stream, query_stream, query_stream_from_stream};
pub use sdk_mcp::{McpSdkServer, SdkMcpTool, create_sdk_mcp_server, tool};
pub use transport::{
    SplitAdapter, Transport, TransportCloseHandle, TransportFactory, TransportReader,
    TransportSplitResult, TransportWriter, split_with_adapter,
};
pub use transport::subprocess_cli::{
    DEFAULT_MAX_BUFFER_SIZE, JsonStreamBuffer, Prompt, SubprocessCliTransport,
};
pub use types::{
    AgentDefinition, AssistantMessage, ClaudeAgentOptions, ContentBlock, HookCallback, HookContext,
    HookInput, HookJSONOutput, HookMatcher, McpHttpServerConfig, McpSSEServerConfig,
    McpSdkServerConfig, McpServerConfig, McpServersOption, McpStdioServerConfig, Message,
    PermissionMode, PermissionResult, PermissionResultAllow, PermissionResultDeny,
    PermissionUpdate, ResultMessage, SandboxIgnoreViolations, SandboxNetworkConfig,
    SandboxSettings, SdkPluginConfig, SettingSource, StderrCallback, StreamEvent, SystemMessage,
    SystemPrompt, SystemPromptPreset, TextBlock, ThinkingBlock, ThinkingConfig, ToolAnnotations,
    ToolPermissionContext, ToolResultBlock, ToolUseBlock, ToolsOption, ToolsPreset, UserContent,
    UserMessage,
};

/// The version of the Claude Code Rust SDK, sourced from `Cargo.toml`.
pub const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");
