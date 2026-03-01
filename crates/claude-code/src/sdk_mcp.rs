use std::collections::HashMap;
use std::sync::Arc;

use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::errors::Error;
use crate::types::{McpSdkServerConfig, ToolAnnotations};

pub type SdkMcpToolHandler =
    Arc<dyn Fn(Value) -> BoxFuture<'static, std::result::Result<Value, Error>> + Send + Sync>;

#[derive(Clone)]
pub struct SdkMcpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub handler: SdkMcpToolHandler,
    pub annotations: Option<ToolAnnotations>,
}

impl SdkMcpTool {
    pub fn with_annotations(mut self, annotations: ToolAnnotations) -> Self {
        self.annotations = Some(annotations);
        self
    }
}

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

#[derive(Clone)]
pub struct McpSdkServer {
    pub name: String,
    pub version: String,
    tool_map: HashMap<String, SdkMcpTool>,
}

impl McpSdkServer {
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

    pub fn has_tools(&self) -> bool {
        !self.tool_map.is_empty()
    }

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
