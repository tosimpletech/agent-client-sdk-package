use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use claude_code::{
    ClaudeAgentOptions, ClaudeSdkClient, InputPrompt, Message, Result, Transport, TransportFactory,
    TransportSplitResult, split_with_adapter,
};
use serde_json::{Value, json};
use tokio::sync::Mutex;

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
async fn test_query_and_receive_use_shared_ref() {
    // Verifies that query() takes &self, enabling concurrent sends.
    let transport = MockTransport::with_messages(vec![
        json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": "req_1", "response": {}}
        }),
        json!({
            "type": "assistant",
            "message": {
                "content": [{"type": "text", "text": "First response"}],
                "model": "claude-opus-4-1-20250805"
            }
        }),
        json!({
            "type": "result",
            "subtype": "success",
            "duration_ms": 100,
            "duration_api_ms": 80,
            "is_error": false,
            "num_turns": 1,
            "session_id": "test",
            "total_cost_usd": 0.001
        }),
    ]);

    let mut client = ClaudeSdkClient::new_with_transport(None, Box::new(transport));
    client.connect(None).await.expect("connect");

    // query() takes &self — can call while holding other references.
    client
        .query(InputPrompt::Text("Hello".to_string()), "s1")
        .await
        .expect("query");

    let messages = client.receive_response().await.expect("response");
    assert!(messages.iter().any(|m| matches!(m, Message::Assistant(_))));
    assert!(messages.iter().any(|m| matches!(m, Message::Result(_))));
}

#[tokio::test]
async fn test_multiple_control_requests_with_buffered_responses() {
    // All control_responses are pre-loaded; the background task buffers
    // those that arrive before the corresponding send_control_request.
    let transport = MockTransport::with_messages(vec![
        json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": "req_1", "response": {}}
        }),
        json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": "req_2", "response": {}}
        }),
        json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": "req_3", "response": {}}
        }),
        json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": "req_4", "response": {}}
        }),
    ]);
    let state = transport.state.clone();

    let mut client = ClaudeSdkClient::new_with_transport(None, Box::new(transport));
    client.connect(None).await.expect("connect");
    client.interrupt().await.expect("interrupt");
    client.set_model(Some("haiku")).await.expect("set_model");

    let state = state.lock().await;
    assert!(
        state
            .written_messages
            .iter()
            .any(|msg| msg.contains("\"subtype\":\"interrupt\""))
    );
    assert!(
        state
            .written_messages
            .iter()
            .any(|msg| msg.contains("\"subtype\":\"set_model\""))
    );
}

#[tokio::test]
async fn test_background_task_handles_control_request_and_sdk_message() {
    // The background task processes a can_use_tool control_request and also
    // forwards an assistant message and result message to the consumer.
    use claude_code::{PermissionResult, PermissionResultAllow, ToolPermissionContext};
    use futures::FutureExt;

    let callback = Arc::new(
        |_tool_name: String, _input: Value, _ctx: ToolPermissionContext| {
            async move { Ok(PermissionResult::Allow(PermissionResultAllow::default())) }.boxed()
        },
    );

    let transport = MockTransport::with_messages(vec![
        json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": "req_1", "response": {}}
        }),
        json!({
            "type": "control_request",
            "request_id": "perm-1",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "Write",
                "input": {"path": "/tmp/test"},
                "permission_suggestions": []
            }
        }),
        json!({
            "type": "assistant",
            "message": {
                "content": [{"type": "text", "text": "Done"}],
                "model": "claude-opus-4-1-20250805"
            }
        }),
        json!({
            "type": "result",
            "subtype": "success",
            "duration_ms": 500,
            "duration_api_ms": 400,
            "is_error": false,
            "num_turns": 1,
            "session_id": "test",
            "total_cost_usd": 0.01
        }),
    ]);
    let state = transport.state.clone();

    let options = ClaudeAgentOptions {
        can_use_tool: Some(callback),
        ..Default::default()
    };

    let mut client = ClaudeSdkClient::new_with_transport(Some(options), Box::new(transport));
    client
        .connect(Some(InputPrompt::Messages(vec![json!({
            "type": "user",
            "message": {"role": "user", "content": "test"},
            "session_id": "",
            "parent_tool_use_id": null
        })])))
        .await
        .expect("connect");

    let messages = client.receive_response().await.expect("response");
    assert!(messages.iter().any(|m| matches!(m, Message::Assistant(_))));
    assert!(messages.iter().any(|m| matches!(m, Message::Result(_))));

    // Verify the permission callback was invoked and response written.
    let state = state.lock().await;
    assert!(
        state
            .written_messages
            .iter()
            .any(|msg| msg.contains("\"behavior\":\"allow\""))
    );
}

#[tokio::test]
async fn test_disconnect_aborts_background_task() {
    let transport = MockTransport::with_messages(vec![json!({
        "type": "control_response",
        "response": {"subtype": "success", "request_id": "req_1", "response": {}}
    })]);

    let mut client = ClaudeSdkClient::new_with_transport(None, Box::new(transport));
    client.connect(None).await.expect("connect");
    client.disconnect().await.expect("disconnect");

    // After disconnect, operations should fail.
    let err = client
        .query(InputPrompt::Text("test".into()), "s1")
        .await
        .expect_err("must fail");
    assert!(err.to_string().contains("Not connected"));
}

struct MockTransportFactory {
    call_count: Arc<AtomicUsize>,
}

impl TransportFactory for MockTransportFactory {
    fn create_transport(&self) -> Result<Box<dyn Transport>> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(MockTransport::with_messages(vec![json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": "req_1", "response": {}}
        })])))
    }
}

#[tokio::test]
async fn test_reconnect_after_disconnect_with_factory() {
    let call_count = Arc::new(AtomicUsize::new(0));
    let factory = MockTransportFactory {
        call_count: call_count.clone(),
    };

    let mut client = ClaudeSdkClient::new(None, Some(Box::new(factory)));

    // First session.
    client.connect(None).await.expect("connect 1");
    client.disconnect().await.expect("disconnect 1");
    assert_eq!(call_count.load(Ordering::SeqCst), 1);

    // Reconnect — same client, factory produces a fresh transport.
    client.connect(None).await.expect("connect 2 (reconnect)");
    client
        .query(InputPrompt::Text("hello".into()), "s2")
        .await
        .expect("query");
    client.disconnect().await.expect("disconnect 2");
    assert_eq!(call_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_single_use_transport_errors_on_reconnect() {
    let transport = MockTransport::with_messages(vec![json!({
        "type": "control_response",
        "response": {"subtype": "success", "request_id": "req_1", "response": {}}
    })]);

    let mut client = ClaudeSdkClient::new_with_transport(None, Box::new(transport));
    client.connect(None).await.expect("connect 1");
    client.disconnect().await.expect("disconnect 1");

    // Second connect with single-use transport should error.
    let err = client.connect(None).await.expect_err("must fail");
    assert!(err.to_string().contains("already consumed"));
}
