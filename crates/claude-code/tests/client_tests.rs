use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use claude_code::{
    ClaudeSdkClient, InputPrompt, Message, Result, Transport, TransportSplitResult,
    split_with_adapter,
};
use futures::stream;
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
