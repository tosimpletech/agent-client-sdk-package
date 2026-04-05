//! # OpenCode SDK for Rust
//!
//! Rust implementation aligned with the official OpenCode JavaScript SDK.
//! It provides:
//! - Local server lifecycle helpers (`create_opencode_server`, `create_opencode_tui`)
//! - HTTP client for OpenCode API (`create_opencode_client`)
//! - Combined helper (`create_opencode`)

pub mod client;
pub mod errors;
pub mod server;
pub mod types;

pub use client::{
    ApiResponse, AppApi, AuthApi, CommandApi, ConfigApi, ControlApi, EventApi, ExperimentalApi,
    ExperimentalSessionApi, FileApi, FindApi, FormatterApi, GlobalApi, InstanceApi, LspApi, McpApi,
    McpAuthApi, OauthApi, OpencodeClient, OpencodeClientConfig, PartApi, PathApi, PermissionApi,
    ProjectApi, ProviderApi, PtyApi, QuestionApi, RequestOptions, ResourceApi, SessionApi,
    SseEvent, SseStream, ToolApi, TuiApi, TuiControlApi, VcsApi, WorkspaceApi, WorktreeApi,
    create_opencode_client,
};
pub use errors::{Error, Result};
pub use server::{
    Opencode, OpencodeServer, OpencodeServerOptions, OpencodeTui, OpencodeTuiOptions,
    create_opencode, create_opencode_server, create_opencode_tui,
};
pub use types::{PartInput, PromptInput, SessionCreateInput};

/// The version of the OpenCode Rust SDK, sourced from `Cargo.toml`.
pub const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");
