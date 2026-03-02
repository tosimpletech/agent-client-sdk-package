use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use claude_code::{
    ClaudeAgentOptions, ClaudeSdkClient, InputPrompt, McpServerConfig, McpServersOption, Message,
    create_sdk_mcp_server, query, tool,
};
use serde_json::json;

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
        .set_permission_mode("acceptEdits")
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
async fn test_include_partial_messages_emits_stream_events() {
    let mut options = base_options();
    options.include_partial_messages = true;

    let messages = query(InputPrompt::Text("Hello".to_string()), Some(options), None)
        .await
        .expect("query");

    assert!(
        messages
            .iter()
            .any(|msg| matches!(msg, Message::StreamEvent(_)))
    );
    assert!(messages.iter().any(|msg| matches!(msg, Message::Result(_))));
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
