use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use claude_code::{
    ClaudeAgentOptions, ClaudeSdkClient, ContentBlock, InputPrompt, Message, PermissionMode,
    Result, SandboxIgnoreViolations, SandboxNetworkConfig, SandboxSettings, SdkPluginConfig,
    SettingSource, SystemPrompt, ThinkingConfig, ToolsOption, ToolsPreset, Transport,
    TransportFactory, TransportSplitResult, UserContent, split_with_adapter,
};
use futures::stream;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio::time::{Duration, sleep, timeout};

#[derive(Default)]
struct MockTransportState {
    written_messages: Vec<String>,
    messages_to_read: VecDeque<Value>,
    connected: bool,
    end_input_calls: usize,
}

#[derive(Clone, Default)]
struct MockTransport {
    state: Arc<Mutex<MockTransportState>>,
}

impl MockTransport {
    fn with_messages(messages: Vec<Value>) -> Self {
        let state = MockTransportState {
            written_messages: Vec::new(),
            messages_to_read: messages.into_iter().collect(),
            connected: false,
            end_input_calls: 0,
        };
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }
}

struct CountingTransportFactory {
    call_count: Arc<AtomicUsize>,
}

impl TransportFactory for CountingTransportFactory {
    fn create_transport(&self) -> Result<Box<dyn Transport>> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(MockTransport::with_messages(vec![json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": "req_1", "response": {}}
        })])))
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
        self.state.lock().await.end_input_calls += 1;
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

#[tokio::test]
async fn test_manual_connect_disconnect() {
    let transport = MockTransport::with_messages(vec![json!({
        "type": "control_response",
        "response": {"subtype": "success", "request_id": "req_1", "response": {}}
    })]);
    let state = transport.state.clone();

    let mut client = ClaudeSdkClient::new_with_transport(None, Box::new(transport));
    client.connect(None).await.expect("connect");
    client.disconnect().await.expect("disconnect");

    let state = state.lock().await;
    assert!(!state.connected);
    assert!(
        state
            .written_messages
            .iter()
            .any(|msg| msg.contains("\"subtype\":\"initialize\""))
    );
}

#[tokio::test]
async fn test_query_sends_user_message_with_session() {
    let transport = MockTransport::with_messages(vec![json!({
        "type": "control_response",
        "response": {"subtype": "success", "request_id": "req_1", "response": {}}
    })]);
    let state = transport.state.clone();

    let mut client = ClaudeSdkClient::new_with_transport(None, Box::new(transport));
    client.connect(None).await.expect("connect");
    client
        .query(InputPrompt::Text("Test message".to_string()), "default")
        .await
        .expect("query");

    let state = state.lock().await;
    assert!(
        state
            .written_messages
            .iter()
            .any(|msg| msg.contains("\"type\":\"user\"")
                && msg.contains("\"session_id\":\"default\""))
    );
}

#[tokio::test]
async fn test_receive_response_stops_at_result() {
    let transport = MockTransport::with_messages(vec![
        json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": "req_1", "response": {}}
        }),
        json!({
            "type": "assistant",
            "message": {
                "content": [{"type": "text", "text": "Answer"}],
                "model": "claude-opus-4-1-20250805"
            }
        }),
        json!({
            "type": "result",
            "subtype": "success",
            "duration_ms": 1000,
            "duration_api_ms": 800,
            "is_error": false,
            "num_turns": 1,
            "session_id": "test",
            "total_cost_usd": 0.001
        }),
        json!({
            "type": "assistant",
            "message": {
                "content": [{"type": "text", "text": "Should not be consumed"}],
                "model": "claude-opus-4-1-20250805"
            }
        }),
    ]);

    let mut client = ClaudeSdkClient::new_with_transport(None, Box::new(transport));
    client.connect(None).await.expect("connect");
    let messages = client.receive_response().await.expect("response");
    assert_eq!(messages.len(), 2);
    assert!(matches!(messages[0], Message::Assistant(_)));
    assert!(matches!(messages[1], Message::Result(_)));
}

#[tokio::test]
async fn test_interrupt_sends_control_request() {
    let transport = MockTransport::with_messages(vec![
        json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": "req_1", "response": {}}
        }),
        json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": "req_2", "response": {}}
        }),
    ]);
    let state = transport.state.clone();

    let mut client = ClaudeSdkClient::new_with_transport(None, Box::new(transport));
    client.connect(None).await.expect("connect");
    client.interrupt().await.expect("interrupt");

    let state = state.lock().await;
    assert!(
        state
            .written_messages
            .iter()
            .any(|msg| msg.contains("\"subtype\":\"interrupt\""))
    );
}

#[tokio::test]
async fn test_errors_when_not_connected() {
    let client = ClaudeSdkClient::new(None, None);
    let err = client
        .query(InputPrompt::Text("Test".to_string()), "default")
        .await
        .expect_err("must fail");
    assert!(err.to_string().contains("Not connected"));

    let err = client.interrupt().await.expect_err("must fail");
    assert!(err.to_string().contains("Not connected"));
}

#[tokio::test]
async fn test_query_stream_injects_session_id() {
    let transport = MockTransport::with_messages(vec![json!({
        "type": "control_response",
        "response": {"subtype": "success", "request_id": "req_1", "response": {}}
    })]);
    let state = transport.state.clone();

    let mut client = ClaudeSdkClient::new_with_transport(None, Box::new(transport));
    client.connect(None).await.expect("connect");
    let input = stream::iter(vec![json!({
        "type": "user",
        "message": {"role": "user", "content": "hello stream"},
        "parent_tool_use_id": null
    })]);
    client
        .query_stream(input, "stream-session")
        .await
        .expect("query_stream");

    let state = state.lock().await;
    assert!(state.written_messages.iter().any(|msg| {
        msg.contains("\"content\":\"hello stream\"")
            && msg.contains("\"session_id\":\"stream-session\"")
    }));
    assert_eq!(state.end_input_calls, 0);
}

#[tokio::test]
async fn test_query_stream_keeps_session_open_for_follow_up_query() {
    let transport = MockTransport::with_messages(vec![json!({
        "type": "control_response",
        "response": {"subtype": "success", "request_id": "req_1", "response": {}}
    })]);
    let state = transport.state.clone();

    let mut client = ClaudeSdkClient::new_with_transport(None, Box::new(transport));
    client.connect(None).await.expect("connect");
    let streamed = stream::iter(vec![json!({
        "type": "user",
        "message": {"role": "user", "content": "first turn"},
        "parent_tool_use_id": null
    })]);
    client
        .query_stream(streamed, "session-1")
        .await
        .expect("query_stream");
    client
        .query(InputPrompt::Text("second turn".to_string()), "session-1")
        .await
        .expect("follow-up query");

    let state = state.lock().await;
    assert!(
        state
            .written_messages
            .iter()
            .any(|msg| msg.contains("\"content\":\"first turn\""))
    );
    assert!(
        state
            .written_messages
            .iter()
            .any(|msg| msg.contains("\"content\":\"second turn\""))
    );
    assert_eq!(state.end_input_calls, 0);
}

#[tokio::test]
async fn test_connect_with_messages_does_not_close_stdin() {
    let transport = MockTransport::with_messages(vec![json!({
        "type": "control_response",
        "response": {"subtype": "success", "request_id": "req_1", "response": {}}
    })]);
    let state = transport.state.clone();

    let mut client = ClaudeSdkClient::new_with_transport(None, Box::new(transport));
    client
        .connect(Some(InputPrompt::Messages(vec![json!({
            "type": "user",
            "message": {"role": "user", "content": "init message"},
            "session_id": "init",
            "parent_tool_use_id": null
        })])))
        .await
        .expect("connect with messages");

    let state = state.lock().await;
    assert_eq!(state.end_input_calls, 0);
}

#[tokio::test]
async fn test_double_connect() {
    let call_count = Arc::new(AtomicUsize::new(0));
    let factory = CountingTransportFactory {
        call_count: call_count.clone(),
    };

    let mut client = ClaudeSdkClient::new(None, Some(Box::new(factory)));
    client.connect(None).await.expect("connect 1");
    client.connect(None).await.expect("connect 2");

    assert_eq!(call_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_disconnect_without_connect() {
    let mut client = ClaudeSdkClient::new(None, None);
    client
        .disconnect()
        .await
        .expect("disconnect without connect");
}

#[tokio::test]
async fn test_scope_cleanup_on_error() {
    async fn run_and_fail(client: ClaudeSdkClient) -> Result<()> {
        let mut client = client;
        client.connect(None).await?;
        Err(claude_code::Error::Other("scope error".to_string()))
    }

    let transport = MockTransport::with_messages(vec![json!({
        "type": "control_response",
        "response": {"subtype": "success", "request_id": "req_1", "response": {}}
    })]);
    let state = transport.state.clone();

    let client = ClaudeSdkClient::new_with_transport(None, Box::new(transport));
    let err = run_and_fail(client)
        .await
        .expect_err("scope must return error");
    assert!(err.to_string().contains("scope error"));

    // Query::Drop closes transport asynchronously; give it a brief moment.
    sleep(Duration::from_millis(30)).await;
    assert!(!state.lock().await.connected);
}

#[tokio::test]
async fn test_connect_with_initial_messages() {
    let transport = MockTransport::with_messages(vec![json!({
        "type": "control_response",
        "response": {"subtype": "success", "request_id": "req_1", "response": {}}
    })]);
    let state = transport.state.clone();

    let mut client = ClaudeSdkClient::new_with_transport(None, Box::new(transport));
    let stream_prompt = stream::iter(vec![
        json!({
            "type": "user",
            "message": {"role": "user", "content": "Hi"},
            "parent_tool_use_id": null
        }),
        json!({
            "type": "user",
            "message": {"role": "user", "content": "Bye"},
            "parent_tool_use_id": null
        }),
    ]);
    client
        .connect_with_messages(stream_prompt)
        .await
        .expect("connect with stream messages");
    client
        .wait_for_initial_messages()
        .await
        .expect("wait for initial stream completion");

    let state = state.lock().await;
    assert!(
        state
            .written_messages
            .iter()
            .any(|msg| msg.contains("\"content\":\"Hi\""))
    );
    assert!(
        state
            .written_messages
            .iter()
            .any(|msg| msg.contains("\"content\":\"Bye\""))
    );
    assert_eq!(state.end_input_calls, 0);
}

#[tokio::test]
async fn test_connect_with_messages_is_non_blocking_for_pending_stream() {
    let transport = MockTransport::with_messages(vec![json!({
        "type": "control_response",
        "response": {"subtype": "success", "request_id": "req_1", "response": {}}
    })]);
    let mut client = ClaudeSdkClient::new_with_transport(None, Box::new(transport));

    timeout(
        Duration::from_millis(100),
        client.connect_with_messages(stream::pending::<Value>()),
    )
    .await
    .expect("connect_with_messages should return without waiting for pending stream")
    .expect("connect_with_messages should succeed");

    timeout(Duration::from_millis(100), client.disconnect())
        .await
        .expect("disconnect should not block on pending initial stream")
        .expect("disconnect should succeed");
}

#[tokio::test]
async fn test_query_with_stream_input() {
    let transport = MockTransport::with_messages(vec![json!({
        "type": "control_response",
        "response": {"subtype": "success", "request_id": "req_1", "response": {}}
    })]);
    let state = transport.state.clone();

    let mut client = ClaudeSdkClient::new_with_transport(None, Box::new(transport));
    client.connect(None).await.expect("connect");

    let input_stream = stream::iter(vec![
        json!({
            "type": "user",
            "message": {"role": "user", "content": "First"},
            "parent_tool_use_id": null
        }),
        json!({
            "type": "user",
            "message": {"role": "user", "content": "Second"},
            "parent_tool_use_id": null
        }),
    ]);
    client
        .query_stream(input_stream, "stream-session")
        .await
        .expect("query_stream");

    let state = state.lock().await;
    assert!(
        state
            .written_messages
            .iter()
            .any(|msg| msg.contains("\"content\":\"First\"")
                && msg.contains("\"session_id\":\"stream-session\""))
    );
    assert!(
        state
            .written_messages
            .iter()
            .any(|msg| msg.contains("\"content\":\"Second\"")
                && msg.contains("\"session_id\":\"stream-session\""))
    );
    assert_eq!(state.end_input_calls, 0);
}

#[tokio::test]
async fn test_receive_messages_detailed_parsing() {
    let transport = MockTransport::with_messages(vec![
        json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": "req_1", "response": {}}
        }),
        json!({
            "type": "assistant",
            "message": {
                "content": [
                    {"type": "text", "text": "Hello!"},
                    {"type": "thinking", "thinking": "step", "signature": "sig-1"},
                    {"type": "tool_use", "id": "tool_1", "name": "Read", "input": {"path": "README.md"}},
                    {"type": "tool_result", "tool_use_id": "tool_1", "content": {"ok": true}, "is_error": false}
                ],
                "model": "claude-opus-4-1-20250805"
            }
        }),
        json!({
            "type": "user",
            "message": {"role": "user", "content": "Hi there"}
        }),
        json!({
            "type": "result",
            "subtype": "success",
            "duration_ms": 1000,
            "duration_api_ms": 800,
            "is_error": false,
            "num_turns": 1,
            "session_id": "test",
            "total_cost_usd": 0.001
        }),
    ]);

    let mut client = ClaudeSdkClient::new_with_transport(None, Box::new(transport));
    client.connect(None).await.expect("connect");

    let first = client
        .receive_message()
        .await
        .expect("receive first")
        .expect("first exists");
    let Message::Assistant(assistant) = first else {
        panic!("expected assistant message");
    };
    assert_eq!(assistant.content.len(), 4);
    assert!(matches!(&assistant.content[0], ContentBlock::Text(block) if block.text == "Hello!"));
    assert!(matches!(
        &assistant.content[1],
        ContentBlock::Thinking(block) if block.thinking == "step" && block.signature == "sig-1"
    ));
    assert!(matches!(
        &assistant.content[2],
        ContentBlock::ToolUse(block)
            if block.id == "tool_1"
                && block.name == "Read"
                && block.input == json!({"path": "README.md"})
    ));
    assert!(matches!(
        &assistant.content[3],
        ContentBlock::ToolResult(block)
            if block.tool_use_id == "tool_1"
                && block.content == Some(json!({"ok": true}))
                && block.is_error == Some(false)
    ));

    let second = client
        .receive_message()
        .await
        .expect("receive second")
        .expect("second exists");
    let Message::User(user) = second else {
        panic!("expected user message");
    };
    assert!(matches!(user.content, UserContent::Text(ref text) if text == "Hi there"));

    let third = client
        .receive_message()
        .await
        .expect("receive third")
        .expect("third exists");
    assert!(matches!(third, Message::Result(_)));
}

#[tokio::test]
async fn test_receive_messages_not_connected() {
    let mut client = ClaudeSdkClient::new(None, None);
    let err = client
        .receive_message()
        .await
        .expect_err("must fail when not connected");
    assert!(err.to_string().contains("Not connected"));
}

#[tokio::test]
async fn test_receive_response_not_connected() {
    let mut client = ClaudeSdkClient::new(None, None);
    let err = client
        .receive_response()
        .await
        .expect_err("must fail when not connected");
    assert!(err.to_string().contains("Not connected"));
}

#[tokio::test]
async fn test_receive_response_collect_pattern() {
    let transport = MockTransport::with_messages(vec![
        json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": "req_1", "response": {}}
        }),
        json!({
            "type": "assistant",
            "message": {
                "content": [{"type": "text", "text": "Hello"}],
                "model": "claude-opus-4-1-20250805"
            }
        }),
        json!({
            "type": "assistant",
            "message": {
                "content": [{"type": "text", "text": "World"}],
                "model": "claude-opus-4-1-20250805"
            }
        }),
        json!({
            "type": "result",
            "subtype": "success",
            "duration_ms": 1000,
            "duration_api_ms": 800,
            "is_error": false,
            "num_turns": 1,
            "session_id": "test",
            "total_cost_usd": 0.001
        }),
    ]);

    let mut client = ClaudeSdkClient::new_with_transport(None, Box::new(transport));
    client.connect(None).await.expect("connect");

    let messages = client.receive_response().await.expect("collect response");
    assert_eq!(messages.len(), 3);
    assert!(matches!(messages[0], Message::Assistant(_)));
    assert!(matches!(messages[1], Message::Assistant(_)));
    assert!(matches!(messages[2], Message::Result(_)));
}

#[tokio::test]
async fn test_concurrent_send_receive() {
    let transport = MockTransport::with_messages(vec![
        json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": "req_1", "response": {}}
        }),
        json!({
            "type": "assistant",
            "message": {
                "content": [{"type": "text", "text": "Response 1"}],
                "model": "claude-opus-4-1-20250805"
            }
        }),
        json!({
            "type": "result",
            "subtype": "success",
            "duration_ms": 1000,
            "duration_api_ms": 800,
            "is_error": false,
            "num_turns": 1,
            "session_id": "test",
            "total_cost_usd": 0.001
        }),
    ]);
    let state = transport.state.clone();

    let mut client = ClaudeSdkClient::new_with_transport(None, Box::new(transport));
    client.connect(None).await.expect("connect");

    let shared_client = Arc::new(Mutex::new(client));
    let send_client = shared_client.clone();
    let receive_client = shared_client.clone();

    let send_task = tokio::spawn(async move {
        let client = send_client.lock().await;
        client
            .query(
                InputPrompt::Text("Question 1".to_string()),
                "concurrent-session",
            )
            .await
            .expect("query");
    });

    let receive_task = tokio::spawn(async move {
        let mut client = receive_client.lock().await;
        client.receive_response().await.expect("receive response")
    });

    let (send_result, receive_result) = tokio::join!(send_task, receive_task);
    send_result.expect("send task should finish");
    let messages = receive_result.expect("receive task should finish");
    assert!(messages.iter().any(|m| matches!(m, Message::Assistant(_))));
    assert!(messages.iter().any(|m| matches!(m, Message::Result(_))));

    let state = state.lock().await;
    assert!(
        state
            .written_messages
            .iter()
            .any(|msg| msg.contains("\"content\":\"Question 1\""))
    );
}

#[tokio::test]
async fn test_interrupt_not_connected() {
    let client = ClaudeSdkClient::new(None, None);
    let err = client.interrupt().await.expect_err("must fail");
    assert!(err.to_string().contains("Not connected"));
}

#[tokio::test]
async fn test_client_with_full_options() {
    let mut env = HashMap::new();
    env.insert("APP_ENV".to_string(), "test".to_string());

    let mut extra_args = HashMap::new();
    extra_args.insert("verbose".to_string(), None);
    extra_args.insert("output-format".to_string(), Some("json".to_string()));

    let options = ClaudeAgentOptions {
        tools: Some(ToolsOption::Preset(ToolsPreset::default())),
        allowed_tools: vec!["Read".to_string(), "Write".to_string()],
        system_prompt: Some(SystemPrompt::Text("Be helpful".to_string())),
        permission_mode: Some(PermissionMode::Default),
        continue_conversation: true,
        resume: Some("session-123".to_string()),
        max_turns: Some(3),
        max_budget_usd: Some(2.5),
        disallowed_tools: vec!["Bash".to_string()],
        model: Some("sonnet".to_string()),
        fallback_model: Some("haiku".to_string()),
        betas: vec!["beta-flag".to_string()],
        permission_prompt_tool_name: Some("permissions".to_string()),
        cwd: Some(PathBuf::from("/tmp/project")),
        cli_path: Some(PathBuf::from("/usr/local/bin/claude")),
        settings: Some("{\"feature\":\"enabled\"}".to_string()),
        add_dirs: vec![PathBuf::from("/tmp/project/src")],
        env,
        extra_args,
        max_buffer_size: Some(2_000_000),
        user: Some("sdk-user".to_string()),
        include_partial_messages: true,
        fork_session: true,
        setting_sources: Some(vec![SettingSource::User, SettingSource::Project]),
        sandbox: Some(SandboxSettings {
            enabled: Some(true),
            auto_allow_bash_if_sandboxed: Some(true),
            excluded_commands: Some(vec!["docker".to_string()]),
            allow_unsandboxed_commands: Some(false),
            network: Some(SandboxNetworkConfig {
                allow_unix_sockets: Some(vec!["/var/run/docker.sock".to_string()]),
                allow_all_unix_sockets: Some(false),
                allow_local_binding: Some(true),
                http_proxy_port: Some(8080),
                socks_proxy_port: Some(1080),
            }),
            ignore_violations: Some(SandboxIgnoreViolations {
                file: Some(vec!["/tmp/**".to_string()]),
                network: Some(vec!["127.0.0.1/**".to_string()]),
            }),
            enable_weaker_nested_sandbox: Some(false),
        }),
        plugins: vec![SdkPluginConfig {
            type_: "local".to_string(),
            path: "./plugins/test".to_string(),
        }],
        max_thinking_tokens: Some(128),
        thinking: Some(ThinkingConfig::Enabled { budget_tokens: 256 }),
        effort: Some("high".to_string()),
        output_format: Some(json!({
            "type": "json_schema",
            "schema": {"type": "object"}
        })),
        enable_file_checkpointing: true,
        ..Default::default()
    };

    let transport = MockTransport::with_messages(vec![json!({
        "type": "control_response",
        "response": {"subtype": "success", "request_id": "req_1", "response": {}}
    })]);
    let mut client = ClaudeSdkClient::new_with_transport(Some(options), Box::new(transport));
    client.connect(None).await.expect("connect");
    client
        .query(
            InputPrompt::Text("Test full options".to_string()),
            "opts-session",
        )
        .await
        .expect("query");

    let server_info = client.get_server_info().expect("server info");
    assert!(server_info.is_some());
}
