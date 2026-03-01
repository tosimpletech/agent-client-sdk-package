use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::errors::{Error, Result};
use crate::message_parser::parse_message;
use crate::sdk_mcp::McpSdkServer;
use crate::transport::Transport;
use crate::types::{
    AgentDefinition, CanUseToolCallback, HookCallback, HookMatcher, Message, PermissionResult,
    ToolPermissionContext,
};

fn convert_hook_output_for_cli(output: Value) -> Value {
    let Some(obj) = output.as_object() else {
        return output;
    };

    let mut converted = Map::new();
    for (key, value) in obj {
        match key.as_str() {
            "async_" => {
                converted.insert("async".to_string(), value.clone());
            }
            "continue_" => {
                converted.insert("continue".to_string(), value.clone());
            }
            _ => {
                converted.insert(key.clone(), value.clone());
            }
        }
    }
    Value::Object(converted)
}

pub struct Query {
    transport: Box<dyn Transport>,
    is_streaming_mode: bool,
    can_use_tool: Option<CanUseToolCallback>,
    hooks: HashMap<String, Vec<HookMatcher>>,
    sdk_mcp_servers: HashMap<String, std::sync::Arc<McpSdkServer>>,
    agents: Option<HashMap<String, AgentDefinition>>,
    request_counter: usize,
    next_callback_id: usize,
    hook_callbacks: HashMap<String, HookCallback>,
    queued_messages: VecDeque<Value>,
    initialized: bool,
    initialization_result: Option<Value>,
    initialize_timeout: Duration,
}

impl Query {
    pub fn new(
        transport: Box<dyn Transport>,
        is_streaming_mode: bool,
        can_use_tool: Option<CanUseToolCallback>,
        hooks: Option<HashMap<String, Vec<HookMatcher>>>,
        sdk_mcp_servers: Option<HashMap<String, std::sync::Arc<McpSdkServer>>>,
        agents: Option<HashMap<String, AgentDefinition>>,
        initialize_timeout: Duration,
    ) -> Self {
        Self {
            transport,
            is_streaming_mode,
            can_use_tool,
            hooks: hooks.unwrap_or_default(),
            sdk_mcp_servers: sdk_mcp_servers.unwrap_or_default(),
            agents,
            request_counter: 0,
            next_callback_id: 0,
            hook_callbacks: HashMap::new(),
            queued_messages: VecDeque::new(),
            initialized: false,
            initialization_result: None,
            initialize_timeout,
        }
    }

    pub async fn start(&mut self) -> Result<()> {
        Ok(())
    }

    pub async fn initialize(&mut self) -> Result<Option<Value>> {
        if !self.is_streaming_mode {
            return Ok(None);
        }

        let mut hooks_config = Map::new();
        for (event, matchers) in &self.hooks {
            if matchers.is_empty() {
                continue;
            }
            let mut event_matchers = Vec::new();
            for matcher in matchers {
                let mut callback_ids = Vec::new();
                for callback in &matcher.hooks {
                    let callback_id = format!("hook_{}", self.next_callback_id);
                    self.next_callback_id += 1;
                    self.hook_callbacks
                        .insert(callback_id.clone(), callback.clone());
                    callback_ids.push(callback_id);
                }

                let mut matcher_obj = Map::new();
                matcher_obj.insert(
                    "matcher".to_string(),
                    matcher
                        .matcher
                        .as_ref()
                        .map(|m| Value::String(m.clone()))
                        .unwrap_or(Value::Null),
                );
                matcher_obj.insert("hookCallbackIds".to_string(), json!(callback_ids));
                if let Some(timeout) = matcher.timeout {
                    matcher_obj.insert("timeout".to_string(), json!(timeout));
                }
                event_matchers.push(Value::Object(matcher_obj));
            }
            hooks_config.insert(event.clone(), Value::Array(event_matchers));
        }

        let mut request = Map::new();
        request.insert(
            "subtype".to_string(),
            Value::String("initialize".to_string()),
        );
        request.insert(
            "hooks".to_string(),
            if hooks_config.is_empty() {
                Value::Null
            } else {
                Value::Object(hooks_config)
            },
        );

        if let Some(agents) = &self.agents {
            request.insert(
                "agents".to_string(),
                serde_json::to_value(agents).unwrap_or(Value::Null),
            );
        }

        let response = self
            .send_control_request(Value::Object(request), self.initialize_timeout)
            .await?;
        self.initialized = true;
        self.initialization_result = Some(response.clone());
        Ok(Some(response))
    }

    pub fn initialization_result(&self) -> Option<Value> {
        self.initialization_result.clone()
    }

    async fn send_control_response(
        &mut self,
        request_id: &str,
        subtype: &str,
        payload: Value,
    ) -> Result<()> {
        let response = match subtype {
            "success" => json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": request_id,
                    "response": payload
                }
            }),
            "error" => json!({
                "type": "control_response",
                "response": {
                    "subtype": "error",
                    "request_id": request_id,
                    "error": payload.as_str().unwrap_or("Unknown error")
                }
            }),
            _ => {
                return Err(Error::Other(format!(
                    "Unsupported control response subtype: {subtype}"
                )));
            }
        };

        self.transport.write(&(response.to_string() + "\n")).await
    }

    pub async fn handle_control_request(&mut self, request: Value) -> Result<()> {
        let Some(request_obj) = request.as_object() else {
            return Err(Error::Other("Invalid control request format".to_string()));
        };
        let request_id = request_obj
            .get("request_id")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Other("Missing request_id in control request".to_string()))?
            .to_string();
        let request_data = request_obj
            .get("request")
            .and_then(Value::as_object)
            .ok_or_else(|| Error::Other("Missing request payload".to_string()))?;
        let subtype = request_data
            .get("subtype")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Other("Missing request subtype".to_string()))?;

        let result: Result<Value> = match subtype {
            "can_use_tool" => {
                let callback = self.can_use_tool.clone().ok_or_else(|| {
                    Error::Other("canUseTool callback is not provided".to_string())
                })?;
                let tool_name = request_data
                    .get("tool_name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let input = request_data
                    .get("input")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let suggestions = request_data
                    .get("permission_suggestions")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|value| serde_json::from_value(value).ok())
                    .collect();
                let context = ToolPermissionContext { suggestions };

                let callback_result = callback(tool_name, input.clone(), context).await?;
                let output = match callback_result {
                    PermissionResult::Allow(allow) => {
                        let mut obj = Map::new();
                        obj.insert("behavior".to_string(), Value::String("allow".to_string()));
                        obj.insert(
                            "updatedInput".to_string(),
                            allow.updated_input.unwrap_or(input),
                        );
                        if let Some(updated_permissions) = allow.updated_permissions {
                            let permissions_json: Vec<Value> = updated_permissions
                                .into_iter()
                                .map(|permission| permission.to_cli_dict())
                                .collect();
                            obj.insert(
                                "updatedPermissions".to_string(),
                                Value::Array(permissions_json),
                            );
                        }
                        Value::Object(obj)
                    }
                    PermissionResult::Deny(deny) => {
                        let mut obj = Map::new();
                        obj.insert("behavior".to_string(), Value::String("deny".to_string()));
                        obj.insert("message".to_string(), Value::String(deny.message));
                        if deny.interrupt {
                            obj.insert("interrupt".to_string(), Value::Bool(true));
                        }
                        Value::Object(obj)
                    }
                };
                Ok(output)
            }
            "hook_callback" => {
                let callback_id = request_data
                    .get("callback_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        Error::Other("Missing callback_id in hook_callback".to_string())
                    })?;
                let callback = self
                    .hook_callbacks
                    .get(callback_id)
                    .cloned()
                    .ok_or_else(|| {
                        Error::Other(format!("No hook callback found for ID: {callback_id}"))
                    })?;
                let input = request_data.get("input").cloned().unwrap_or(Value::Null);
                let tool_use_id = request_data
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string);
                let output = callback(input, tool_use_id, Default::default()).await?;
                Ok(convert_hook_output_for_cli(output))
            }
            "mcp_message" => {
                let server_name = request_data
                    .get("server_name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        Error::Other("Missing server_name in mcp_message".to_string())
                    })?;
                let message = request_data
                    .get("message")
                    .cloned()
                    .ok_or_else(|| Error::Other("Missing message in mcp_message".to_string()))?;
                let response = self.handle_sdk_mcp_request(server_name, &message).await;
                Ok(json!({ "mcp_response": response }))
            }
            _ => Err(Error::Other(format!(
                "Unsupported control request subtype: {subtype}"
            ))),
        };

        match result {
            Ok(payload) => {
                self.send_control_response(&request_id, "success", payload)
                    .await
            }
            Err(err) => {
                self.send_control_response(&request_id, "error", Value::String(err.to_string()))
                    .await
            }
        }
    }

    async fn send_control_request(&mut self, request: Value, timeout: Duration) -> Result<Value> {
        if !self.is_streaming_mode {
            return Err(Error::Other(
                "Control requests require streaming mode".to_string(),
            ));
        }

        self.request_counter += 1;
        let request_id = format!("req_{}", self.request_counter);

        let control_request = json!({
            "type": "control_request",
            "request_id": request_id,
            "request": request,
        });
        self.transport
            .write(&(control_request.to_string() + "\n"))
            .await?;

        let deadline = Instant::now() + timeout;
        loop {
            if Instant::now() > deadline {
                let subtype = request
                    .get("subtype")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                return Err(Error::Other(format!("Control request timeout: {subtype}")));
            }

            let Some(message) = self.transport.read_next_message().await? else {
                return Err(Error::Other(
                    "Transport closed while waiting for control response".to_string(),
                ));
            };

            let msg_type = message
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();

            if msg_type == "control_response" {
                let Some(response) = message.get("response").and_then(Value::as_object) else {
                    continue;
                };
                let response_request_id = response
                    .get("request_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if response_request_id != request_id {
                    continue;
                }

                let subtype = response
                    .get("subtype")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if subtype == "error" {
                    let error = response
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("Unknown error");
                    return Err(Error::Other(error.to_string()));
                }

                return Ok(response
                    .get("response")
                    .cloned()
                    .unwrap_or_else(|| json!({})));
            }

            if msg_type == "control_request" {
                self.handle_control_request(message).await?;
                continue;
            }

            self.queued_messages.push_back(message);
        }
    }

    pub async fn handle_sdk_mcp_request(&self, server_name: &str, message: &Value) -> Value {
        let Some(server) = self.sdk_mcp_servers.get(server_name) else {
            return json!({
                "jsonrpc": "2.0",
                "id": message.get("id").cloned().unwrap_or(Value::Null),
                "error": {
                    "code": -32601,
                    "message": format!("Server '{server_name}' not found")
                }
            });
        };

        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let id = message.get("id").cloned().unwrap_or(Value::Null);
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));

        match method {
            "initialize" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {"tools": {}},
                    "serverInfo": {
                        "name": server.name,
                        "version": server.version
                    }
                }
            }),
            "tools/list" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "tools": server.list_tools_json()
                }
            }),
            "tools/call" => {
                let tool_name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let arguments = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let result = server.call_tool_json(tool_name, arguments).await;
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": result
                })
            }
            "notifications/initialized" => json!({
                "jsonrpc": "2.0",
                "result": {}
            }),
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32601,
                    "message": format!("Method '{method}' not found")
                }
            }),
        }
    }

    pub async fn send_user_message(&mut self, prompt: &str, session_id: &str) -> Result<()> {
        let message = json!({
            "type": "user",
            "message": {"role": "user", "content": prompt},
            "parent_tool_use_id": Value::Null,
            "session_id": session_id
        });
        self.transport.write(&(message.to_string() + "\n")).await
    }

    pub async fn send_raw_message(&mut self, message: Value) -> Result<()> {
        self.transport.write(&(message.to_string() + "\n")).await
    }

    pub async fn stream_input(&mut self, messages: Vec<Value>) -> Result<()> {
        for message in messages {
            self.send_raw_message(message).await?;
        }
        self.transport.end_input().await
    }

    pub async fn end_input(&mut self) -> Result<()> {
        self.transport.end_input().await
    }

    pub async fn receive_next_message(&mut self) -> Result<Option<Message>> {
        loop {
            let raw = if let Some(message) = self.queued_messages.pop_front() {
                Some(message)
            } else {
                self.transport.read_next_message().await?
            };

            let Some(raw) = raw else {
                return Ok(None);
            };

            let msg_type = raw.get("type").and_then(Value::as_str).unwrap_or_default();
            if msg_type == "control_request" {
                self.handle_control_request(raw).await?;
                continue;
            }
            if msg_type == "control_response" || msg_type == "control_cancel_request" {
                continue;
            }

            let parsed = parse_message(&raw)?;
            if parsed.is_some() {
                return Ok(parsed);
            }
        }
    }

    pub async fn get_mcp_status(&mut self) -> Result<Value> {
        self.send_control_request(json!({ "subtype": "mcp_status" }), Duration::from_secs(60))
            .await
    }

    pub async fn interrupt(&mut self) -> Result<()> {
        self.send_control_request(json!({ "subtype": "interrupt" }), Duration::from_secs(60))
            .await?;
        Ok(())
    }

    pub async fn set_permission_mode(&mut self, mode: &str) -> Result<()> {
        self.send_control_request(
            json!({ "subtype": "set_permission_mode", "mode": mode }),
            Duration::from_secs(60),
        )
        .await?;
        Ok(())
    }

    pub async fn set_model(&mut self, model: Option<&str>) -> Result<()> {
        self.send_control_request(
            json!({ "subtype": "set_model", "model": model }),
            Duration::from_secs(60),
        )
        .await?;
        Ok(())
    }

    pub async fn rewind_files(&mut self, user_message_id: &str) -> Result<()> {
        self.send_control_request(
            json!({ "subtype": "rewind_files", "user_message_id": user_message_id }),
            Duration::from_secs(60),
        )
        .await?;
        Ok(())
    }

    pub async fn close(&mut self) -> Result<()> {
        self.transport.close().await
    }
}
