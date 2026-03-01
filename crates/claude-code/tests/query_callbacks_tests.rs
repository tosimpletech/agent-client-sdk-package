use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use async_trait::async_trait;
use claude_code_client_sdk::{
    HookMatcher, PermissionResult, PermissionResultAllow, PermissionResultDeny, Query, Result,
    ToolPermissionContext, Transport,
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
}

#[tokio::test]
async fn test_permission_callback_allow() {
    let transport = MockTransport::default();
    let state = transport.state.clone();
    let callback = Arc::new(
        |_tool_name: String, _input: Value, _ctx: ToolPermissionContext| {
            async move { Ok(PermissionResult::Allow(PermissionResultAllow::default())) }.boxed()
        },
    );

    let mut query = Query::new(
        Box::new(transport),
        true,
        Some(callback),
        None,
        None,
        None,
        std::time::Duration::from_secs(60),
    );
    let request = json!({
        "type": "control_request",
        "request_id": "test-1",
        "request": {
            "subtype": "can_use_tool",
            "tool_name": "TestTool",
            "input": {"param": "value"},
            "permission_suggestions": []
        }
    });
    query
        .handle_control_request(request)
        .await
        .expect("handled");

    let state = state.lock().await;
    assert_eq!(state.written_messages.len(), 1);
    assert!(state.written_messages[0].contains("\"behavior\":\"allow\""));
}

#[tokio::test]
async fn test_permission_callback_deny() {
    let transport = MockTransport::default();
    let state = transport.state.clone();
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

    let mut query = Query::new(
        Box::new(transport),
        true,
        Some(callback),
        None,
        None,
        None,
        std::time::Duration::from_secs(60),
    );
    let request = json!({
        "type": "control_request",
        "request_id": "test-2",
        "request": {
            "subtype": "can_use_tool",
            "tool_name": "DangerousTool",
            "input": {"command": "rm -rf /"},
            "permission_suggestions": []
        }
    });
    query
        .handle_control_request(request)
        .await
        .expect("handled");

    let state = state.lock().await;
    assert_eq!(state.written_messages.len(), 1);
    assert!(state.written_messages[0].contains("\"behavior\":\"deny\""));
    assert!(state.written_messages[0].contains("Security policy violation"));
}

#[tokio::test]
async fn test_hook_field_name_conversion() {
    let transport = MockTransport::default();
    let state = transport.state.clone();

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

    // Pre-seed initialize response.
    state.lock().await.messages_to_read.push_back(json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": "req_1",
            "response": {}
        }
    }));

    let mut query = Query::new(
        Box::new(transport.clone()),
        true,
        None,
        Some(hooks),
        None,
        None,
        std::time::Duration::from_secs(60),
    );
    query.initialize().await.expect("initialize");

    let request = json!({
        "type": "control_request",
        "request_id": "hook-1",
        "request": {
            "subtype": "hook_callback",
            "callback_id": "hook_0",
            "input": {"x": 1},
            "tool_use_id": null
        }
    });
    query
        .handle_control_request(request)
        .await
        .expect("handled");

    let state = state.lock().await;
    let last = state.written_messages.last().expect("response");
    let parsed: Value = serde_json::from_str(last).expect("json");
    let result = &parsed["response"]["response"];
    assert_eq!(result["async"], true);
    assert_eq!(result["continue"], false);
    assert_eq!(result["stopReason"], "Testing field conversion");
    assert!(result.get("async_").is_none());
    assert!(result.get("continue_").is_none());
}
