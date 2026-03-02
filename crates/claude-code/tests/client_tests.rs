use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use claude_code::{ClaudeSdkClient, InputPrompt, Message, Result, Transport};
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
            written_messages: Vec::new(),
            messages_to_read: messages.into_iter().collect(),
            connected: false,
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
}

#[tokio::test]
async fn test_manual_connect_disconnect() {
    let transport = MockTransport::with_messages(vec![json!({
        "type": "control_response",
        "response": {"subtype": "success", "request_id": "req_1", "response": {}}
    })]);
    let state = transport.state.clone();

    let mut client = ClaudeSdkClient::new(None, Some(Box::new(transport)));
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

    let mut client = ClaudeSdkClient::new(None, Some(Box::new(transport)));
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

    let mut client = ClaudeSdkClient::new(None, Some(Box::new(transport)));
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

    let mut client = ClaudeSdkClient::new(None, Some(Box::new(transport)));
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
    let mut client = ClaudeSdkClient::new(None, None);
    let err = client
        .query(InputPrompt::Text("Test".to_string()), "default")
        .await
        .expect_err("must fail");
    assert!(err.to_string().contains("Not connected"));

    let err = client.interrupt().await.expect_err("must fail");
    assert!(err.to_string().contains("Not connected"));
}
