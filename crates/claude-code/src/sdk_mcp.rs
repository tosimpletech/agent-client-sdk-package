//! In-process MCP (Model Context Protocol) server support.
//!
//! This module allows you to define custom tools that run within your Rust application
//! and are exposed to Claude Code via the MCP protocol. These tools appear alongside
//! Claude Code's built-in tools and can be invoked by the model during conversations.
//!
//! # Example
//!
//! ```rust,no_run
//! use claude_code::{tool, create_sdk_mcp_server, McpServerConfig, ToolAnnotations};
//! use serde_json::{json, Value};
//!
//! let weather_tool = tool(
//!     "get_weather",
//!     "Get current weather for a location",
//!     json!({
//!         "type": "object",
//!         "properties": {
//!             "location": {"type": "string", "description": "City name"}
//!         },
//!         "required": ["location"]
//!     }),
//!     |args: Value| async move {
//!         let location = args["location"].as_str().unwrap_or("unknown");
//!         Ok(json!({
//!             "content": [{"type": "text", "text": format!("Weather in {location}: 22°C, sunny")}]
//!         }))
//!     },
//! );
//!
//! let server_config = create_sdk_mcp_server("my-tools", "1.0.0", vec![weather_tool]);
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::errors::Error;
use crate::types::{McpSdkServerConfig, ToolAnnotations};

/// Handler function type for SDK MCP tools.
///
/// Takes a JSON `Value` of input arguments and returns a JSON `Value` result.
/// The result should follow the MCP tool result format with `content` array.
pub type SdkMcpToolHandler =
    Arc<dyn Fn(Value) -> BoxFuture<'static, std::result::Result<Value, Error>> + Send + Sync>;

/// Definition of an in-process MCP tool.
///
/// Created via the [`tool()`] factory function. Can be customized with
/// [`with_annotations()`](Self::with_annotations) before being passed to
/// [`create_sdk_mcp_server()`].
///
/// # Fields
///
/// - `name` — Unique tool name (used by the model to invoke it).
/// - `description` — Human-readable description of what the tool does.
/// - `input_schema` — JSON Schema defining the tool's input parameters.
/// - `handler` — Async function that executes the tool logic.
/// - `annotations` — Optional behavioral hints (read-only, destructive, etc.).
#[derive(Clone)]
pub struct SdkMcpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub handler: SdkMcpToolHandler,
    pub annotations: Option<ToolAnnotations>,
}

impl SdkMcpTool {
    /// Adds behavioral annotations to this tool.
    ///
    /// Annotations provide hints about the tool's behavior (e.g., read-only,
    /// destructive, idempotent) to help with permission handling.
    ///
    /// Returns `self` for method chaining.
    pub fn with_annotations(mut self, annotations: ToolAnnotations) -> Self {
        self.annotations = Some(annotations);
        self
    }
}

/// Creates a new [`SdkMcpTool`] with the given name, description, schema, and handler.
///
/// This is the primary factory function for defining custom tools.
///
/// # Arguments
///
/// * `name` — Unique name for the tool.
/// * `description` — What the tool does (shown to the model).
/// * `input_schema` — JSON Schema for the tool's input parameters.
/// * `handler` — Async function implementing the tool logic. Receives input as
///   a JSON `Value` and should return a JSON `Value` in MCP result format.
///
/// # Example
///
/// ```rust,no_run
/// # use claude_code::tool;
/// # use serde_json::{json, Value};
/// let my_tool = tool(
///     "greet",
///     "Greet someone by name",
///     json!({"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]}),
///     |args: Value| async move {
///         let name = args["name"].as_str().unwrap_or("world");
///         Ok(json!({"content": [{"type": "text", "text": format!("Hello, {name}!")}]}))
///     },
/// );
/// ```
pub fn tool<F, Fut>(name: &str, description: &str, input_schema: Value, handler: F) -> SdkMcpTool
where
    F: Fn(Value) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = std::result::Result<Value, Error>> + Send + 'static,
{
    let wrapped: SdkMcpToolHandler = Arc::new(move |args: Value| Box::pin(handler(args)));
    SdkMcpTool {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
        handler: wrapped,
        annotations: None,
    }
}

/// In-process MCP server that hosts custom tools.
///
/// Implements the MCP tool listing and calling protocol. Tool calls are dispatched
/// to the registered handler functions and executed within your application.
#[derive(Clone)]
pub struct McpSdkServer {
    /// Server name identifier.
    pub name: String,
    /// Server version string.
    pub version: String,
    tool_map: HashMap<String, SdkMcpTool>,
}

impl McpSdkServer {
    /// Creates a new MCP server with the given name, version, and tools.
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        tools: Vec<SdkMcpTool>,
    ) -> Self {
        let mut tool_map = HashMap::new();
        for tool in tools {
            tool_map.insert(tool.name.clone(), tool);
        }
        Self {
            name: name.into(),
            version: version.into(),
            tool_map,
        }
    }

    /// Returns `true` if the server has any registered tools.
    pub fn has_tools(&self) -> bool {
        !self.tool_map.is_empty()
    }

    /// Returns JSON representations of all registered tools (for `tools/list` responses).
    pub fn list_tools_json(&self) -> Vec<Value> {
        self.tool_map
            .values()
            .map(|tool| {
                let mut base = json!({
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": tool.input_schema,
                });
                if let Some(annotations) = &tool.annotations
                    && let Value::Object(ref mut obj) = base
                {
                    obj.insert(
                        "annotations".to_string(),
                        serde_json::to_value(annotations).unwrap_or(Value::Null),
                    );
                }
                base
            })
            .collect()
    }

    /// Calls a tool by name with the given arguments and returns the JSON result.
    ///
    /// If the tool is not found or the handler returns an error, an error result
    /// in MCP format is returned (with `is_error: true`).
    pub async fn call_tool_json(&self, tool_name: &str, arguments: Value) -> Value {
        let Some(tool) = self.tool_map.get(tool_name) else {
            return json!({
                "content": [
                    {"type": "text", "text": format!("Tool '{tool_name}' not found")}
                ],
                "is_error": true
            });
        };

        match (tool.handler)(arguments).await {
            Ok(result) => result,
            Err(err) => json!({
                "content": [{"type": "text", "text": err.to_string()}],
                "is_error": true
            }),
        }
    }
}

/// Creates an [`McpSdkServerConfig`] for use in [`ClaudeAgentOptions::mcp_servers`](crate::ClaudeAgentOptions::mcp_servers).
///
/// This is the entry point for registering in-process MCP servers with the SDK.
///
/// # Arguments
///
/// * `name` — Unique server name.
/// * `version` — Server version string.
/// * `tools` — List of tools to register on this server.
///
/// # Returns
///
/// An [`McpSdkServerConfig`] that can be added to the `mcp_servers` map.
///
/// # Example
///
/// ```rust,no_run
/// # use claude_code::{tool, create_sdk_mcp_server, ClaudeAgentOptions, McpServerConfig, McpServersOption};
/// # use serde_json::{json, Value};
/// # use std::collections::HashMap;
/// let server = create_sdk_mcp_server("my-server", "1.0.0", vec![
///     tool("hello", "Say hello", json!({"type": "object"}), |_| async { Ok(json!({"content": []})) }),
/// ]);
///
/// let options = ClaudeAgentOptions {
///     mcp_servers: McpServersOption::Servers(HashMap::from([
///         ("my-server".to_string(), McpServerConfig::Sdk(server)),
///     ])),
///     ..Default::default()
/// };
/// ```
pub fn create_sdk_mcp_server(
    name: impl Into<String>,
    version: impl Into<String>,
    tools: Vec<SdkMcpTool>,
) -> McpSdkServerConfig {
    let name_string = name.into();
    let version_string = version.into();
    let server = Arc::new(McpSdkServer::new(
        name_string.clone(),
        version_string,
        tools,
    ));

    McpSdkServerConfig {
        type_: "sdk".to_string(),
        name: name_string,
        instance: server,
    }
}
