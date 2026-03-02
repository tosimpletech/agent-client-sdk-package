use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use async_trait::async_trait;
use claude_code::{
    ClaudeAgentOptions, HookMatcher, InputPrompt, Message, PermissionResult,
    PermissionResultAllow, PermissionResultDeny, Result, ToolPermissionContext, Transport,
    TransportSplitResult, query, split_with_adapter,
};
use futures::FutureExt;
use serde_json::{Value, json};
use tokio::sync::Mutex;

#[derive(Default)]
struct MockTransportState {
    written_messages: Vec<String>,
    messages_to_read: VecDeque<Value>,
    connected: bool,
}

#[derive(Clone, Default)]
struct MockTransport {
    state: Arc<Mutex<MockTransportState>>,
}

impl MockTransport {
    fn with_messages(messages: Vec<Value>) -> Self {
        let state = MockTransportState {
            messages_to_read: messages.into_iter().collect(),
            ..Default::default()
        };
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }
}

#[async_trait]
impl Transport for MockTransport {
    async fn connect(&mut self) -> Result<()> {
        self.state.lock().await.connected = true;
        Ok(())
    }

    async fn write(&mut self, data: &str) -> Result<()> {
        self.state
            .lock()
            .await
            .written_messages
            .push(data.to_string());
        Ok(())
    }

    async fn end_input(&mut self) -> Result<()> {
        Ok(())
    }

    async fn read_next_message(&mut self) -> Result<Option<Value>> {
        Ok(self.state.lock().await.messages_to_read.pop_front())
    }

    async fn close(&mut self) -> Result<()> {
        self.state.lock().await.connected = false;
        Ok(())
    }

    fn is_ready(&self) -> bool {
        true
    }

    fn into_split(self: Box<Self>) -> TransportSplitResult {
        split_with_adapter(self)
    }
}

/// Helper: standard result message to end a query.
fn result_message() -> Value {
    json!({
        "type": "result",
        "subtype": "success",
        "duration_ms": 100,
        "duration_api_ms": 80,
        "is_error": false,
        "num_turns": 1,
        "session_id": "test",
        "total_cost_usd": 0.0
    })
}

#[tokio::test]
async fn test_permission_callback_allow() {
    let callback = Arc::new(
        |_tool_name: String, _input: Value, _ctx: ToolPermissionContext| {
            async move { Ok(PermissionResult::Allow(PermissionResultAllow::default())) }.boxed()
        },
    );

    // Transport message sequence:
    // 1. Init response (consumed by initialize())
    // 2. CLI sends can_use_tool control_request (handled by background task)
    // 3. Result message (returned to caller)
    let transport = MockTransport::with_messages(vec![
        json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": "req_1", "response": {}}
        }),
        json!({
            "type": "control_request",
            "request_id": "test-1",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "TestTool",
                "input": {"param": "value"},
                "permission_suggestions": []
            }
        }),
        result_message(),
    ]);
    let state = transport.state.clone();

    let options = ClaudeAgentOptions {
        can_use_tool: Some(callback),
        ..Default::default()
    };

    let messages = query(
        InputPrompt::Messages(vec![json!({
            "type": "user",
            "message": {"role": "user", "content": "test"},
            "session_id": "",
            "parent_tool_use_id": null
        })]),
        Some(options),
        Some(Box::new(transport)),
    )
    .await
    .expect("query");

    assert!(messages.iter().any(|m| matches!(m, Message::Result(_))));

    let state = state.lock().await;
    assert!(
        state
            .written_messages
            .iter()
            .any(|msg| msg.contains("\"behavior\":\"allow\""))
    );
}

#[tokio::test]
async fn test_permission_callback_deny() {
    let callback = Arc::new(
        |_tool_name: String, _input: Value, _ctx: ToolPermissionContext| {
            async move {
                Ok(PermissionResult::Deny(PermissionResultDeny {
                    message: "Security policy violation".to_string(),
                    interrupt: false,
                }))
            }
            .boxed()
        },
    );

    let transport = MockTransport::with_messages(vec![
        json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": "req_1", "response": {}}
        }),
        json!({
            "type": "control_request",
            "request_id": "test-2",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "DangerousTool",
                "input": {"command": "rm -rf /"},
                "permission_suggestions": []
            }
        }),
        result_message(),
    ]);
    let state = transport.state.clone();

    let options = ClaudeAgentOptions {
        can_use_tool: Some(callback),
        ..Default::default()
    };

    let messages = query(
        InputPrompt::Messages(vec![json!({
            "type": "user",
            "message": {"role": "user", "content": "test"},
            "session_id": "",
            "parent_tool_use_id": null
        })]),
        Some(options),
        Some(Box::new(transport)),
    )
    .await
    .expect("query");

    assert!(messages.iter().any(|m| matches!(m, Message::Result(_))));

    let state = state.lock().await;
    assert!(
        state
            .written_messages
            .iter()
            .any(|msg| msg.contains("\"behavior\":\"deny\"")
                && msg.contains("Security policy violation"))
    );
}

#[tokio::test]
async fn test_hook_field_name_conversion() {
    let hook_callback = Arc::new(|_input: Value, _tool_use_id: Option<String>, _ctx| {
        async move {
            Ok(json!({
                "async_": true,
                "continue_": false,
                "stopReason": "Testing field conversion"
            }))
        }
        .boxed()
    });

    let mut hook_matcher = HookMatcher::default();
    hook_matcher.hooks.push(hook_callback);
    let mut hooks = HashMap::new();
    hooks.insert("PreToolUse".to_string(), vec![hook_matcher]);

    // Transport message sequence:
    // 1. Init response
    // 2. CLI sends hook_callback control_request
    // 3. Result message
    let transport = MockTransport::with_messages(vec![
        json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": "req_1", "response": {}}
        }),
        json!({
            "type": "control_request",
            "request_id": "hook-1",
            "request": {
                "subtype": "hook_callback",
                "callback_id": "hook_0",
                "input": {"x": 1},
                "tool_use_id": null
            }
        }),
        result_message(),
    ]);
    let state = transport.state.clone();

    let options = ClaudeAgentOptions {
        hooks: Some(hooks),
        ..Default::default()
    };

    let messages = query(
        InputPrompt::Text("test".to_string()),
        Some(options),
        Some(Box::new(transport)),
    )
    .await
    .expect("query");

    assert!(messages.iter().any(|m| matches!(m, Message::Result(_))));

    let state = state.lock().await;
    // Find the hook callback response in written messages.
    let hook_response = state
        .written_messages
        .iter()
        .find(|msg| msg.contains("\"request_id\":\"hook-1\""))
        .expect("hook response should be written");
    let parsed: Value = serde_json::from_str(hook_response).expect("valid json");
    let result = &parsed["response"]["response"];
    assert_eq!(result["async"], true);
    assert_eq!(result["continue"], false);
    assert_eq!(result["stopReason"], "Testing field conversion");
    // Verify Rust-safe names are not present in the output.
    assert!(result.get("async_").is_none());
    assert!(result.get("continue_").is_none());
}
