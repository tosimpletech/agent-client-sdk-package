use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use claude_code::{
    ClaudeAgentOptions, ClaudeSdkClient, ContentBlock, InputPrompt, McpServerConfig,
    McpServersOption, Message, PermissionMode, create_sdk_mcp_server, query, tool,
};
use serde_json::{Value, json};

fn fixture_cli_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock_claude_cli.py")
}

fn base_options() -> ClaudeAgentOptions {
    ClaudeAgentOptions {
        cli_path: Some(fixture_cli_path()),
        ..Default::default()
    }
}

#[tokio::test]
async fn test_dynamic_controls_with_subprocess_transport() {
    let options = base_options();
    let mut client = ClaudeSdkClient::new(Some(options), None);

    client.connect(None).await.expect("connect");
    client
        .set_permission_mode(PermissionMode::AcceptEdits)
        .await
        .expect("set_permission_mode");
    client
        .set_model(Some("claude-3-5-haiku-20241022"))
        .await
        .expect("set_model");
    client.interrupt().await.expect("interrupt");
    client
        .rewind_files("msg-checkpoint-1")
        .await
        .expect("rewind_files");
    let mcp_status = client.get_mcp_status().await.expect("get_mcp_status");
    assert_eq!(mcp_status.mcp_servers.len(), 1);
    client
        .reconnect_mcp_server("mock-sdk")
        .await
        .expect("reconnect_mcp_server");
    client
        .toggle_mcp_server("mock-sdk", false)
        .await
        .expect("toggle_mcp_server");
    client.stop_task("task-1").await.expect("stop_task");

    client
        .query(InputPrompt::Text("What is 2+2?".to_string()), "default")
        .await
        .expect("query");
    let messages = client.receive_response().await.expect("receive_response");
    assert!(messages.iter().any(|m| matches!(m, Message::Result(_))));

    client.disconnect().await.expect("disconnect");
}

#[tokio::test]
async fn test_stderr_callback_receives_debug_lines() {
    let stderr_lines = Arc::new(Mutex::new(Vec::<String>::new()));
    let captured = stderr_lines.clone();

    let mut options = base_options();
    options.stderr = Some(Arc::new(move |line: String| {
        captured.lock().expect("lock").push(line);
    }));
    options
        .extra_args
        .insert("debug-to-stderr".to_string(), None);

    let _ = query(InputPrompt::Text("Hello".to_string()), Some(options), None)
        .await
        .expect("query");

    let lines = stderr_lines.lock().expect("lock");
    assert!(lines.iter().any(|line| line.contains("[DEBUG]")));
}

#[tokio::test]
async fn test_partial_messages_stream_event_types() {
    let mut options = base_options();
    options.include_partial_messages = true;

    let messages = query(InputPrompt::Text("Hello".to_string()), Some(options), None)
        .await
        .expect("query");

    assert!(matches!(
        messages.first(),
        Some(Message::System(system)) if system.subtype == "init"
    ));

    let stream_events: Vec<&Value> = messages
        .iter()
        .filter_map(|msg| match msg {
            Message::StreamEvent(stream_event) => Some(&stream_event.event),
            _ => None,
        })
        .collect();
    assert!(!stream_events.is_empty(), "No stream events found");

    let event_types: Vec<String> = stream_events
        .iter()
        .filter_map(|event| {
            event
                .get("type")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .collect();

    for expected in [
        "message_start",
        "content_block_start",
        "content_block_delta",
        "content_block_stop",
        "message_stop",
    ] {
        assert!(
            event_types.iter().any(|event_type| event_type == expected),
            "Missing stream event type: {expected}"
        );
    }

    let has_thinking_delta = stream_events.iter().any(|event| {
        event.get("type").and_then(Value::as_str) == Some("content_block_delta")
            && event
                .get("delta")
                .and_then(Value::as_object)
                .and_then(|delta| delta.get("type"))
                .and_then(Value::as_str)
                == Some("thinking_delta")
            && event
                .get("delta")
                .and_then(Value::as_object)
                .and_then(|delta| delta.get("thinking"))
                .and_then(Value::as_str)
                .is_some()
    });
    assert!(
        has_thinking_delta,
        "No thinking_delta stream event with thinking text"
    );

    let has_text_delta = stream_events.iter().any(|event| {
        event.get("type").and_then(Value::as_str) == Some("content_block_delta")
            && event
                .get("delta")
                .and_then(Value::as_object)
                .and_then(|delta| delta.get("type"))
                .and_then(Value::as_str)
                == Some("text_delta")
            && event
                .get("delta")
                .and_then(Value::as_object)
                .and_then(|delta| delta.get("text"))
                .and_then(Value::as_str)
                .is_some()
    });
    assert!(has_text_delta, "No text_delta stream event with text");

    let has_thinking_block = messages.iter().any(|msg| match msg {
        Message::Assistant(assistant) => assistant
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::Thinking(_))),
        _ => false,
    });
    assert!(
        has_thinking_block,
        "No assistant message contained a Thinking block"
    );

    let has_text_block = messages.iter().any(|msg| match msg {
        Message::Assistant(assistant) => assistant
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::Text(_))),
        _ => false,
    });
    assert!(
        has_text_block,
        "No assistant message contained a Text block"
    );

    assert!(matches!(
        messages.last(),
        Some(Message::Result(result)) if result.subtype == "success"
    ));
}

#[tokio::test]
async fn test_partial_messages_thinking_deltas() {
    let mut options = base_options();
    options.include_partial_messages = true;

    let messages = query(
        InputPrompt::Text("Think step by step about what 2 + 2 equals".to_string()),
        Some(options),
        None,
    )
    .await
    .expect("query");

    let thinking_deltas: Vec<String> = messages
        .iter()
        .filter_map(|msg| match msg {
            Message::StreamEvent(stream_event)
                if stream_event.event.get("type").and_then(Value::as_str)
                    == Some("content_block_delta") =>
            {
                stream_event
                    .event
                    .get("delta")
                    .and_then(Value::as_object)
                    .and_then(|delta| {
                        if delta.get("type").and_then(Value::as_str) == Some("thinking_delta") {
                            delta
                                .get("thinking")
                                .and_then(Value::as_str)
                                .map(ToString::to_string)
                        } else {
                            None
                        }
                    })
            }
            _ => None,
        })
        .collect();

    assert!(
        thinking_deltas.len() > 1,
        "Expected multiple thinking deltas, got {}",
        thinking_deltas.len()
    );

    let combined_thinking = thinking_deltas.join("");
    assert!(
        combined_thinking.len() > 10,
        "Expected combined thinking delta content length > 10"
    );
}

#[tokio::test]
async fn test_partial_messages_disabled_by_default() {
    let options = base_options();

    let messages = query(
        InputPrompt::Text("Say hello".to_string()),
        Some(options),
        None,
    )
    .await
    .expect("query");

    assert!(
        !messages
            .iter()
            .any(|msg| matches!(msg, Message::StreamEvent(_))),
        "Stream events should not be present when include_partial_messages is disabled"
    );

    assert!(
        messages.iter().any(|msg| matches!(msg, Message::System(_))),
        "Expected System message"
    );
    assert!(
        messages
            .iter()
            .any(|msg| matches!(msg, Message::Assistant(_))),
        "Expected Assistant message"
    );
    assert!(
        messages.iter().any(|msg| matches!(msg, Message::Result(_))),
        "Expected Result message"
    );
}

#[tokio::test]
async fn test_sdk_mcp_control_message_roundtrip() {
    let list_tool = tool(
        "list_files",
        "List files",
        json!({"type": "object", "properties": {}}),
        |_args| async move { Ok(json!({"content": [{"type": "text", "text": "ok"}]})) },
    );
    let sdk_server = create_sdk_mcp_server("mock-sdk", "1.0.0", vec![list_tool]);

    let mut servers = HashMap::new();
    servers.insert("mock-sdk".to_string(), McpServerConfig::Sdk(sdk_server));

    let mut options = base_options();
    options.mcp_servers = McpServersOption::Servers(servers);
    options
        .env
        .insert("MOCK_CLAUDE_TRIGGER_MCP".to_string(), "1".to_string());

    let messages = query(
        InputPrompt::Text("Trigger MCP".to_string()),
        Some(options),
        None,
    )
    .await
    .expect("query");

    let assistant_text = messages
        .iter()
        .find_map(|msg| match msg {
            Message::Assistant(assistant) => {
                assistant.content.iter().find_map(|block| match block {
                    claude_code::ContentBlock::Text(text) => Some(text.text.clone()),
                    _ => None,
                })
            }
            _ => None,
        })
        .unwrap_or_default();
    assert!(assistant_text.contains("MCP response handled"));
    assert!(messages.iter().any(|msg| matches!(msg, Message::Result(_))));
}
