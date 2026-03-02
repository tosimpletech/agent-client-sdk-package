use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use claude_code::{
    ClaudeAgentOptions, Error, HookMatcher, InputPrompt, Message, PermissionResult,
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

fn initialize_success_message() -> Value {
    json!({
        "type": "control_response",
        "response": {"subtype": "success", "request_id": "req_1", "response": {}}
    })
}

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

fn user_input_messages() -> InputPrompt {
    InputPrompt::Messages(vec![json!({
        "type": "user",
        "message": {"role": "user", "content": "test"},
        "session_id": "",
        "parent_tool_use_id": null
    })])
}

async fn find_written_message(state: &Arc<Mutex<MockTransportState>>, needle: &str) -> String {
    let state = state.lock().await;
    state
        .written_messages
        .iter()
        .find(|msg| msg.contains(needle))
        .unwrap_or_else(|| panic!("expected written message containing: {needle}"))
        .clone()
}

async fn find_control_response_by_request_id(
    state: &Arc<Mutex<MockTransportState>>,
    request_id: &str,
) -> Value {
    let needle = format!("\"request_id\":\"{request_id}\"");
    let message = find_written_message(state, &needle).await;
    serde_json::from_str(&message).expect("valid JSON response")
}

#[tokio::test]
async fn test_permission_callback_allow() {
    let callback_invoked = Arc::new(AtomicBool::new(false));
    let callback_invoked_clone = callback_invoked.clone();
    let callback = Arc::new(
        move |_tool_name: String, _input: Value, _ctx: ToolPermissionContext| {
            callback_invoked_clone.store(true, Ordering::SeqCst);
            async move { Ok(PermissionResult::Allow(PermissionResultAllow::default())) }.boxed()
        },
    );

    let transport = MockTransport::with_messages(vec![
        initialize_success_message(),
        json!({
            "type": "control_request",
            "request_id": "test-allow",
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

    let messages = query(
        user_input_messages(),
        Some(ClaudeAgentOptions {
            can_use_tool: Some(callback),
            ..Default::default()
        }),
        Some(Box::new(transport)),
    )
    .await
    .expect("query");

    assert!(messages.iter().any(|m| matches!(m, Message::Result(_))));
    assert!(callback_invoked.load(Ordering::SeqCst));

    let response = find_control_response_by_request_id(&state, "test-allow").await;
    assert_eq!(response["response"]["subtype"], "success");
    assert_eq!(response["response"]["response"]["behavior"], "allow");
    assert_eq!(
        response["response"]["response"]["updatedInput"],
        json!({"param": "value"})
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
        initialize_success_message(),
        json!({
            "type": "control_request",
            "request_id": "test-deny",
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

    let messages = query(
        user_input_messages(),
        Some(ClaudeAgentOptions {
            can_use_tool: Some(callback),
            ..Default::default()
        }),
        Some(Box::new(transport)),
    )
    .await
    .expect("query");

    assert!(messages.iter().any(|m| matches!(m, Message::Result(_))));

    let response = find_control_response_by_request_id(&state, "test-deny").await;
    assert_eq!(response["response"]["subtype"], "success");
    assert_eq!(response["response"]["response"]["behavior"], "deny");
    assert_eq!(
        response["response"]["response"]["message"],
        "Security policy violation"
    );
}

#[tokio::test]
async fn test_permission_callback_input_modification() {
    let callback = Arc::new(
        |_tool_name: String, input: Value, _ctx: ToolPermissionContext| {
            async move {
                let mut modified_input = input.as_object().cloned().unwrap_or_default();
                modified_input.insert("safe_mode".to_string(), Value::Bool(true));
                Ok(PermissionResult::Allow(PermissionResultAllow {
                    updated_input: Some(Value::Object(modified_input)),
                    ..Default::default()
                }))
            }
            .boxed()
        },
    );

    let transport = MockTransport::with_messages(vec![
        initialize_success_message(),
        json!({
            "type": "control_request",
            "request_id": "test-modify-input",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "WriteTool",
                "input": {"file_path": "/etc/passwd"},
                "permission_suggestions": []
            }
        }),
        result_message(),
    ]);
    let state = transport.state.clone();

    let messages = query(
        user_input_messages(),
        Some(ClaudeAgentOptions {
            can_use_tool: Some(callback),
            ..Default::default()
        }),
        Some(Box::new(transport)),
    )
    .await
    .expect("query");

    assert!(messages.iter().any(|m| matches!(m, Message::Result(_))));

    let response = find_control_response_by_request_id(&state, "test-modify-input").await;
    assert_eq!(response["response"]["subtype"], "success");
    assert_eq!(response["response"]["response"]["behavior"], "allow");
    assert_eq!(
        response["response"]["response"]["updatedInput"]["safe_mode"],
        true
    );
}

#[tokio::test]
async fn test_callback_exception_handling() {
    let can_use_call_count = Arc::new(AtomicUsize::new(0));
    let can_use_call_count_clone = can_use_call_count.clone();
    let can_use_tool = Arc::new(
        move |_tool_name: String, _input: Value, _ctx: ToolPermissionContext| {
            let call = can_use_call_count_clone.fetch_add(1, Ordering::SeqCst);
            match call {
                0 => async move {
                    panic!("permission callback panic");
                    #[allow(unreachable_code)]
                    Ok(PermissionResult::Allow(PermissionResultAllow::default()))
                }
                .boxed(),
                1 => async move { Err(Error::Other("permission callback error".to_string())) }
                    .boxed(),
                _ => async move { Ok(PermissionResult::Allow(PermissionResultAllow::default())) }
                    .boxed(),
            }
        },
    );

    let hook_call_count = Arc::new(AtomicUsize::new(0));
    let hook_call_count_clone = hook_call_count.clone();
    let hook_callback = Arc::new(move |_input: Value, _tool_use_id: Option<String>, _ctx| {
        let call = hook_call_count_clone.fetch_add(1, Ordering::SeqCst);
        match call {
            0 => panic!("hook callback panic"),
            1 => async move { Err(Error::Other("hook callback error".to_string())) }.boxed(),
            _ => async move { Ok(json!({"processed": true})) }.boxed(),
        }
    });

    let mut hook_matcher = HookMatcher::default();
    hook_matcher.hooks.push(hook_callback);
    let mut hooks = HashMap::new();
    hooks.insert("PreToolUse".to_string(), vec![hook_matcher]);

    let transport = MockTransport::with_messages(vec![
        initialize_success_message(),
        json!({
            "type": "control_request",
            "request_id": "test-can-panic",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "TestTool",
                "input": {},
                "permission_suggestions": []
            }
        }),
        json!({
            "type": "control_request",
            "request_id": "test-can-err",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "TestTool",
                "input": {},
                "permission_suggestions": []
            }
        }),
        json!({
            "type": "control_request",
            "request_id": "test-hook-panic",
            "request": {
                "subtype": "hook_callback",
                "callback_id": "hook_0",
                "input": {"test": "hook"},
                "tool_use_id": "tool-1"
            }
        }),
        json!({
            "type": "control_request",
            "request_id": "test-hook-err",
            "request": {
                "subtype": "hook_callback",
                "callback_id": "hook_0",
                "input": {"test": "hook"},
                "tool_use_id": "tool-2"
            }
        }),
        json!({
            "type": "control_request",
            "request_id": "test-can-ok",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "TestTool",
                "input": {"ok": true},
                "permission_suggestions": []
            }
        }),
        json!({
            "type": "control_request",
            "request_id": "test-hook-ok",
            "request": {
                "subtype": "hook_callback",
                "callback_id": "hook_0",
                "input": {"ok": true},
                "tool_use_id": "tool-3"
            }
        }),
        result_message(),
    ]);
    let state = transport.state.clone();

    let messages = query(
        user_input_messages(),
        Some(ClaudeAgentOptions {
            can_use_tool: Some(can_use_tool),
            hooks: Some(hooks),
            ..Default::default()
        }),
        Some(Box::new(transport)),
    )
    .await
    .expect("query");

    assert!(messages.iter().any(|m| matches!(m, Message::Result(_))));

    let can_panic_response = find_control_response_by_request_id(&state, "test-can-panic").await;
    assert_eq!(can_panic_response["response"]["subtype"], "error");
    assert!(
        can_panic_response["response"]["error"]
            .as_str()
            .expect("panic error string")
            .contains("permission callback panic")
    );

    let can_err_response = find_control_response_by_request_id(&state, "test-can-err").await;
    assert_eq!(can_err_response["response"]["subtype"], "error");
    assert_eq!(
        can_err_response["response"]["error"],
        "permission callback error"
    );

    let hook_panic_response = find_control_response_by_request_id(&state, "test-hook-panic").await;
    assert_eq!(hook_panic_response["response"]["subtype"], "error");
    assert!(
        hook_panic_response["response"]["error"]
            .as_str()
            .expect("panic error string")
            .contains("hook callback panic")
    );

    let hook_err_response = find_control_response_by_request_id(&state, "test-hook-err").await;
    assert_eq!(hook_err_response["response"]["subtype"], "error");
    assert_eq!(
        hook_err_response["response"]["error"],
        "hook callback error"
    );

    let can_ok_response = find_control_response_by_request_id(&state, "test-can-ok").await;
    assert_eq!(can_ok_response["response"]["subtype"], "success");
    assert_eq!(can_ok_response["response"]["response"]["behavior"], "allow");

    let hook_ok_response = find_control_response_by_request_id(&state, "test-hook-ok").await;
    assert_eq!(hook_ok_response["response"]["subtype"], "success");
    assert_eq!(hook_ok_response["response"]["response"]["processed"], true);
}

#[tokio::test]
async fn test_hook_execution() {
    let hook_calls = Arc::new(Mutex::new(Vec::<(Value, Option<String>)>::new()));
    let hook_calls_clone = hook_calls.clone();
    let hook_callback = Arc::new(move |input: Value, tool_use_id: Option<String>, _ctx| {
        let hook_calls = hook_calls_clone.clone();
        async move {
            hook_calls
                .lock()
                .await
                .push((input.clone(), tool_use_id.clone()));
            Ok(json!({"processed": true}))
        }
        .boxed()
    });

    let mut hook_matcher = HookMatcher {
        matcher: Some("TestTool".to_string()),
        ..Default::default()
    };
    hook_matcher.hooks.push(hook_callback);
    let mut hooks = HashMap::new();
    hooks.insert("tool_use_start".to_string(), vec![hook_matcher]);

    let transport = MockTransport::with_messages(vec![
        initialize_success_message(),
        json!({
            "type": "control_request",
            "request_id": "test-hook-exec",
            "request": {
                "subtype": "hook_callback",
                "callback_id": "hook_0",
                "input": {"test": "data"},
                "tool_use_id": "tool-123"
            }
        }),
        result_message(),
    ]);
    let state = transport.state.clone();

    let messages = query(
        InputPrompt::Text("test".to_string()),
        Some(ClaudeAgentOptions {
            hooks: Some(hooks),
            ..Default::default()
        }),
        Some(Box::new(transport)),
    )
    .await
    .expect("query");

    assert!(messages.iter().any(|m| matches!(m, Message::Result(_))));

    let hook_calls = hook_calls.lock().await;
    assert_eq!(hook_calls.len(), 1);
    assert_eq!(hook_calls[0].0, json!({"test": "data"}));
    assert_eq!(hook_calls[0].1, Some("tool-123".to_string()));

    let response = find_control_response_by_request_id(&state, "test-hook-exec").await;
    assert_eq!(response["response"]["subtype"], "success");
    assert_eq!(response["response"]["response"]["processed"], true);
}

#[tokio::test]
async fn test_hook_output_fields() {
    let hook_callback = Arc::new(|_input: Value, _tool_use_id: Option<String>, _ctx| {
        async move {
            Ok(json!({
                "continue_": true,
                "suppressOutput": false,
                "stopReason": "Test stop reason",
                "decision": "block",
                "systemMessage": "Test system message",
                "reason": "Test reason for blocking",
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": "Security policy violation",
                    "updatedInput": {"modified": "input"}
                }
            }))
        }
        .boxed()
    });

    let mut hook_matcher = HookMatcher::default();
    hook_matcher.hooks.push(hook_callback);
    let mut hooks = HashMap::new();
    hooks.insert("PreToolUse".to_string(), vec![hook_matcher]);

    let transport = MockTransport::with_messages(vec![
        initialize_success_message(),
        json!({
            "type": "control_request",
            "request_id": "test-hook-output-fields",
            "request": {
                "subtype": "hook_callback",
                "callback_id": "hook_0",
                "input": {"test": "data"},
                "tool_use_id": "tool-456"
            }
        }),
        result_message(),
    ]);
    let state = transport.state.clone();

    let messages = query(
        InputPrompt::Text("test".to_string()),
        Some(ClaudeAgentOptions {
            hooks: Some(hooks),
            ..Default::default()
        }),
        Some(Box::new(transport)),
    )
    .await
    .expect("query");

    assert!(messages.iter().any(|m| matches!(m, Message::Result(_))));

    let response = find_control_response_by_request_id(&state, "test-hook-output-fields").await;
    let result = &response["response"]["response"];
    assert_eq!(result["continue"], true);
    assert!(result.get("continue_").is_none());
    assert_eq!(result["suppressOutput"], false);
    assert_eq!(result["stopReason"], "Test stop reason");
    assert_eq!(result["decision"], "block");
    assert_eq!(result["systemMessage"], "Test system message");
    assert_eq!(result["reason"], "Test reason for blocking");
    assert_eq!(result["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    assert_eq!(result["hookSpecificOutput"]["permissionDecision"], "deny");
    assert_eq!(
        result["hookSpecificOutput"]["permissionDecisionReason"],
        "Security policy violation"
    );
    assert_eq!(
        result["hookSpecificOutput"]["updatedInput"],
        json!({"modified": "input"})
    );
}

#[tokio::test]
async fn test_async_hook_output() {
    let hook_callback = Arc::new(|_input: Value, _tool_use_id: Option<String>, _ctx| {
        async move { Ok(json!({"async_": true, "asyncTimeout": 5000})) }.boxed()
    });

    let mut hook_matcher = HookMatcher::default();
    hook_matcher.hooks.push(hook_callback);
    let mut hooks = HashMap::new();
    hooks.insert("PreToolUse".to_string(), vec![hook_matcher]);

    let transport = MockTransport::with_messages(vec![
        initialize_success_message(),
        json!({
            "type": "control_request",
            "request_id": "test-async-hook",
            "request": {
                "subtype": "hook_callback",
                "callback_id": "hook_0",
                "input": {"test": "async_data"},
                "tool_use_id": null
            }
        }),
        result_message(),
    ]);
    let state = transport.state.clone();

    let messages = query(
        InputPrompt::Text("test".to_string()),
        Some(ClaudeAgentOptions {
            hooks: Some(hooks),
            ..Default::default()
        }),
        Some(Box::new(transport)),
    )
    .await
    .expect("query");

    assert!(messages.iter().any(|m| matches!(m, Message::Result(_))));

    let response = find_control_response_by_request_id(&state, "test-async-hook").await;
    let result = &response["response"]["response"];
    assert_eq!(result["async"], true);
    assert!(result.get("async_").is_none());
    assert_eq!(result["asyncTimeout"], 5000);
}

#[tokio::test]
async fn test_field_name_conversion() {
    let hook_callback = Arc::new(|_input: Value, _tool_use_id: Option<String>, _ctx| {
        async move {
            Ok(json!({
                "async_": true,
                "asyncTimeout": 10000,
                "continue_": false,
                "stopReason": "Testing field conversion",
                "systemMessage": "Fields should be converted"
            }))
        }
        .boxed()
    });

    let mut hook_matcher = HookMatcher::default();
    hook_matcher.hooks.push(hook_callback);
    let mut hooks = HashMap::new();
    hooks.insert("PreToolUse".to_string(), vec![hook_matcher]);

    let transport = MockTransport::with_messages(vec![
        initialize_success_message(),
        json!({
            "type": "control_request",
            "request_id": "test-field-conversion",
            "request": {
                "subtype": "hook_callback",
                "callback_id": "hook_0",
                "input": {"test": "data"},
                "tool_use_id": null
            }
        }),
        result_message(),
    ]);
    let state = transport.state.clone();

    let messages = query(
        InputPrompt::Text("test".to_string()),
        Some(ClaudeAgentOptions {
            hooks: Some(hooks),
            ..Default::default()
        }),
        Some(Box::new(transport)),
    )
    .await
    .expect("query");

    assert!(messages.iter().any(|m| matches!(m, Message::Result(_))));

    let response = find_control_response_by_request_id(&state, "test-field-conversion").await;
    let result = &response["response"]["response"];
    assert_eq!(result["async"], true);
    assert!(result.get("async_").is_none());
    assert_eq!(result["continue"], false);
    assert!(result.get("continue_").is_none());
    assert_eq!(result["asyncTimeout"], 10000);
    assert_eq!(result["stopReason"], "Testing field conversion");
    assert_eq!(result["systemMessage"], "Fields should be converted");
}

#[test]
fn test_options_with_callbacks() {
    let permission_callback = Arc::new(
        |_tool_name: String, _input: Value, _ctx: ToolPermissionContext| {
            async move { Ok(PermissionResult::Allow(PermissionResultAllow::default())) }.boxed()
        },
    );
    let hook_callback = Arc::new(|_input: Value, _tool_use_id: Option<String>, _ctx| {
        async move { Ok(json!({})) }.boxed()
    });

    let mut hook_matcher = HookMatcher {
        matcher: Some("Bash".to_string()),
        ..Default::default()
    };
    hook_matcher.hooks.push(hook_callback);

    let mut hooks = HashMap::new();
    hooks.insert("tool_use_start".to_string(), vec![hook_matcher]);

    let options = ClaudeAgentOptions {
        can_use_tool: Some(permission_callback),
        hooks: Some(hooks),
        ..Default::default()
    };

    assert!(options.can_use_tool.is_some());
    let hooks = options.hooks.expect("hooks should exist");
    assert!(hooks.contains_key("tool_use_start"));
    assert_eq!(hooks["tool_use_start"].len(), 1);
    assert_eq!(hooks["tool_use_start"][0].hooks.len(), 1);
}

#[tokio::test]
async fn test_notification_hook_callback() {
    let hook_calls = Arc::new(Mutex::new(Vec::<(Value, Option<String>)>::new()));
    let hook_calls_clone = hook_calls.clone();
    let hook_callback = Arc::new(move |input: Value, tool_use_id: Option<String>, _ctx| {
        let hook_calls = hook_calls_clone.clone();
        async move {
            hook_calls
                .lock()
                .await
                .push((input.clone(), tool_use_id.clone()));
            Ok(json!({
                "hookSpecificOutput": {
                    "hookEventName": "Notification",
                    "additionalContext": "Notification processed"
                }
            }))
        }
        .boxed()
    });

    let mut hook_matcher = HookMatcher::default();
    hook_matcher.hooks.push(hook_callback);
    let mut hooks = HashMap::new();
    hooks.insert("Notification".to_string(), vec![hook_matcher]);

    let transport = MockTransport::with_messages(vec![
        initialize_success_message(),
        json!({
            "type": "control_request",
            "request_id": "test-notification",
            "request": {
                "subtype": "hook_callback",
                "callback_id": "hook_0",
                "input": {
                    "session_id": "sess-1",
                    "transcript_path": "/tmp/t",
                    "cwd": "/home",
                    "hook_event_name": "Notification",
                    "message": "Task completed",
                    "notification_type": "info"
                },
                "tool_use_id": null
            }
        }),
        result_message(),
    ]);
    let state = transport.state.clone();

    let messages = query(
        InputPrompt::Text("test".to_string()),
        Some(ClaudeAgentOptions {
            hooks: Some(hooks),
            ..Default::default()
        }),
        Some(Box::new(transport)),
    )
    .await
    .expect("query");

    assert!(messages.iter().any(|m| matches!(m, Message::Result(_))));

    let hook_calls = hook_calls.lock().await;
    assert_eq!(hook_calls.len(), 1);
    assert_eq!(hook_calls[0].0["hook_event_name"], "Notification");
    assert_eq!(hook_calls[0].0["message"], "Task completed");

    let response = find_control_response_by_request_id(&state, "test-notification").await;
    let result = &response["response"]["response"];
    assert_eq!(
        result["hookSpecificOutput"]["hookEventName"],
        "Notification"
    );
    assert_eq!(
        result["hookSpecificOutput"]["additionalContext"],
        "Notification processed"
    );
}

#[tokio::test]
async fn test_permission_request_hook_callback() {
    let hook_callback = Arc::new(|_input: Value, _tool_use_id: Option<String>, _ctx| {
        async move {
            Ok(json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": {"type": "allow"}
                }
            }))
        }
        .boxed()
    });

    let mut hook_matcher = HookMatcher::default();
    hook_matcher.hooks.push(hook_callback);
    let mut hooks = HashMap::new();
    hooks.insert("PermissionRequest".to_string(), vec![hook_matcher]);

    let transport = MockTransport::with_messages(vec![
        initialize_success_message(),
        json!({
            "type": "control_request",
            "request_id": "test-perm-req",
            "request": {
                "subtype": "hook_callback",
                "callback_id": "hook_0",
                "input": {
                    "session_id": "sess-1",
                    "transcript_path": "/tmp/t",
                    "cwd": "/home",
                    "hook_event_name": "PermissionRequest",
                    "tool_name": "Bash",
                    "tool_input": {"command": "ls"}
                },
                "tool_use_id": null
            }
        }),
        result_message(),
    ]);
    let state = transport.state.clone();

    let messages = query(
        InputPrompt::Text("test".to_string()),
        Some(ClaudeAgentOptions {
            hooks: Some(hooks),
            ..Default::default()
        }),
        Some(Box::new(transport)),
    )
    .await
    .expect("query");

    assert!(messages.iter().any(|m| matches!(m, Message::Result(_))));

    let response = find_control_response_by_request_id(&state, "test-perm-req").await;
    let result = &response["response"]["response"];
    assert_eq!(
        result["hookSpecificOutput"]["hookEventName"],
        "PermissionRequest"
    );
    assert_eq!(
        result["hookSpecificOutput"]["decision"],
        json!({"type": "allow"})
    );
}

#[tokio::test]
async fn test_subagent_start_hook_callback() {
    let hook_callback = Arc::new(|_input: Value, _tool_use_id: Option<String>, _ctx| {
        async move {
            Ok(json!({
                "hookSpecificOutput": {
                    "hookEventName": "SubagentStart",
                    "additionalContext": "Subagent approved"
                }
            }))
        }
        .boxed()
    });

    let mut hook_matcher = HookMatcher::default();
    hook_matcher.hooks.push(hook_callback);
    let mut hooks = HashMap::new();
    hooks.insert("SubagentStart".to_string(), vec![hook_matcher]);

    let transport = MockTransport::with_messages(vec![
        initialize_success_message(),
        json!({
            "type": "control_request",
            "request_id": "test-subagent-start",
            "request": {
                "subtype": "hook_callback",
                "callback_id": "hook_0",
                "input": {
                    "session_id": "sess-1",
                    "transcript_path": "/tmp/t",
                    "cwd": "/home",
                    "hook_event_name": "SubagentStart",
                    "agent_id": "agent-42",
                    "agent_type": "researcher"
                },
                "tool_use_id": null
            }
        }),
        result_message(),
    ]);
    let state = transport.state.clone();

    let messages = query(
        InputPrompt::Text("test".to_string()),
        Some(ClaudeAgentOptions {
            hooks: Some(hooks),
            ..Default::default()
        }),
        Some(Box::new(transport)),
    )
    .await
    .expect("query");

    assert!(messages.iter().any(|m| matches!(m, Message::Result(_))));

    let response = find_control_response_by_request_id(&state, "test-subagent-start").await;
    let result = &response["response"]["response"];
    assert_eq!(
        result["hookSpecificOutput"]["hookEventName"],
        "SubagentStart"
    );
    assert_eq!(
        result["hookSpecificOutput"]["additionalContext"],
        "Subagent approved"
    );
}

#[tokio::test]
async fn test_post_tool_use_hook_with_updated_mcp_output() {
    let hook_callback = Arc::new(|_input: Value, _tool_use_id: Option<String>, _ctx| {
        async move {
            Ok(json!({
                "hookSpecificOutput": {
                    "hookEventName": "PostToolUse",
                    "updatedMCPToolOutput": {"result": "modified output"}
                }
            }))
        }
        .boxed()
    });

    let mut hook_matcher = HookMatcher::default();
    hook_matcher.hooks.push(hook_callback);
    let mut hooks = HashMap::new();
    hooks.insert("PostToolUse".to_string(), vec![hook_matcher]);

    let transport = MockTransport::with_messages(vec![
        initialize_success_message(),
        json!({
            "type": "control_request",
            "request_id": "test-post-tool-mcp",
            "request": {
                "subtype": "hook_callback",
                "callback_id": "hook_0",
                "input": {
                    "session_id": "sess-1",
                    "transcript_path": "/tmp/t",
                    "cwd": "/home",
                    "hook_event_name": "PostToolUse",
                    "tool_name": "mcp_tool",
                    "tool_input": {},
                    "tool_response": "original output",
                    "tool_use_id": "tu-123"
                },
                "tool_use_id": "tu-123"
            }
        }),
        result_message(),
    ]);
    let state = transport.state.clone();

    let messages = query(
        InputPrompt::Text("test".to_string()),
        Some(ClaudeAgentOptions {
            hooks: Some(hooks),
            ..Default::default()
        }),
        Some(Box::new(transport)),
    )
    .await
    .expect("query");

    assert!(messages.iter().any(|m| matches!(m, Message::Result(_))));

    let response = find_control_response_by_request_id(&state, "test-post-tool-mcp").await;
    let result = &response["response"]["response"];
    assert_eq!(
        result["hookSpecificOutput"]["updatedMCPToolOutput"],
        json!({"result": "modified output"})
    );
}

#[tokio::test]
async fn test_pre_tool_use_hook_with_additional_context() {
    let hook_callback = Arc::new(|_input: Value, _tool_use_id: Option<String>, _ctx| {
        async move {
            Ok(json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "allow",
                    "additionalContext": "Extra context for Claude"
                }
            }))
        }
        .boxed()
    });

    let mut hook_matcher = HookMatcher::default();
    hook_matcher.hooks.push(hook_callback);
    let mut hooks = HashMap::new();
    hooks.insert("PreToolUse".to_string(), vec![hook_matcher]);

    let transport = MockTransport::with_messages(vec![
        initialize_success_message(),
        json!({
            "type": "control_request",
            "request_id": "test-pre-tool-ctx",
            "request": {
                "subtype": "hook_callback",
                "callback_id": "hook_0",
                "input": {
                    "session_id": "sess-1",
                    "transcript_path": "/tmp/t",
                    "cwd": "/home",
                    "hook_event_name": "PreToolUse",
                    "tool_name": "Bash",
                    "tool_input": {"command": "ls"},
                    "tool_use_id": "tu-456"
                },
                "tool_use_id": "tu-456"
            }
        }),
        result_message(),
    ]);
    let state = transport.state.clone();

    let messages = query(
        InputPrompt::Text("test".to_string()),
        Some(ClaudeAgentOptions {
            hooks: Some(hooks),
            ..Default::default()
        }),
        Some(Box::new(transport)),
    )
    .await
    .expect("query");

    assert!(messages.iter().any(|m| matches!(m, Message::Result(_))));

    let response = find_control_response_by_request_id(&state, "test-pre-tool-ctx").await;
    let result = &response["response"]["response"];
    assert_eq!(
        result["hookSpecificOutput"]["additionalContext"],
        "Extra context for Claude"
    );
    assert_eq!(result["hookSpecificOutput"]["permissionDecision"], "allow");
}

#[tokio::test]
async fn test_new_hook_events_registered_in_hooks_config() {
    let noop_hook = Arc::new(|_input: Value, _tool_use_id: Option<String>, _ctx| {
        async move { Ok(json!({})) }.boxed()
    });

    let mut notification_matcher = HookMatcher::default();
    notification_matcher.hooks.push(noop_hook.clone());
    let mut subagent_start_matcher = HookMatcher::default();
    subagent_start_matcher.hooks.push(noop_hook.clone());
    let mut permission_request_matcher = HookMatcher::default();
    permission_request_matcher.hooks.push(noop_hook);

    let mut hooks = HashMap::new();
    hooks.insert("Notification".to_string(), vec![notification_matcher]);
    hooks.insert("SubagentStart".to_string(), vec![subagent_start_matcher]);
    hooks.insert(
        "PermissionRequest".to_string(),
        vec![permission_request_matcher],
    );

    let transport =
        MockTransport::with_messages(vec![initialize_success_message(), result_message()]);
    let state = transport.state.clone();

    let messages = query(
        InputPrompt::Text("test".to_string()),
        Some(ClaudeAgentOptions {
            hooks: Some(hooks),
            ..Default::default()
        }),
        Some(Box::new(transport)),
    )
    .await
    .expect("query");

    assert!(messages.iter().any(|m| matches!(m, Message::Result(_))));

    let init_request = find_written_message(&state, "\"subtype\":\"initialize\"").await;
    let init_json: Value = serde_json::from_str(&init_request).expect("valid initialize request");
    let hooks = &init_json["request"]["hooks"];
    assert!(hooks.get("Notification").is_some());
    assert!(hooks.get("SubagentStart").is_some());
    assert!(hooks.get("PermissionRequest").is_some());
}
