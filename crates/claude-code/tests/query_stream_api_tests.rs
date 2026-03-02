use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use claude_code::{
    Error, InputPrompt, Message, Result, Transport, TransportSplitResult, query, query_from_stream,
    query_stream, query_stream_from_stream, split_with_adapter,
};
use futures::TryStreamExt;
use futures::stream;
use serde_json::{Value, json};
use tokio::sync::Mutex;

#[derive(Default)]
struct MockTransportState {
    written_messages: Vec<String>,
    messages_to_read: VecDeque<Value>,
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
        };
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }
}

#[async_trait]
impl Transport for MockTransport {
    async fn connect(&mut self) -> Result<()> {
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
        Ok(())
    }

    fn is_ready(&self) -> bool {
        true
    }

    fn into_split(self: Box<Self>) -> TransportSplitResult {
        split_with_adapter(self)
    }
}

fn protocol_messages() -> Vec<Value> {
    vec![
        json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": "req_1", "response": {}}
        }),
        json!({
            "type": "assistant",
            "message": {
                "content": [{"type": "text", "text": "stream ok"}],
                "model": "claude-opus-4-1-20250805"
            }
        }),
        json!({
            "type": "result",
            "subtype": "success",
            "duration_ms": 10,
            "duration_api_ms": 8,
            "is_error": false,
            "num_turns": 1,
            "session_id": "test",
            "total_cost_usd": 0.0
        }),
    ]
}

#[tokio::test]
async fn test_query_from_stream_accepts_streamed_input() {
    let transport = MockTransport::with_messages(protocol_messages());
    let state = transport.state.clone();

    let input = stream::iter(vec![json!({
        "type": "user",
        "message": {"role": "user", "content": "Hello from stream"},
        "session_id": "session-1",
        "parent_tool_use_id": null
    })]);

    let messages = query_from_stream(input, None, Some(Box::new(transport)))
        .await
        .expect("query_from_stream");

    assert_eq!(messages.len(), 2);
    assert!(matches!(messages[0], Message::Assistant(_)));
    assert!(matches!(messages[1], Message::Result(_)));

    let state = state.lock().await;
    assert!(
        state
            .written_messages
            .iter()
            .any(|m| m.contains("Hello from stream"))
    );
}

#[tokio::test]
async fn test_query_stream_returns_streamed_messages() {
    let transport = MockTransport::with_messages(protocol_messages());

    let output_stream = query_stream(
        InputPrompt::Text("Hello".to_string()),
        None,
        Some(Box::new(transport)),
    )
    .await
    .expect("query_stream");

    let messages = output_stream
        .try_collect::<Vec<_>>()
        .await
        .expect("collect stream output");
    assert_eq!(messages.len(), 2);
    assert!(matches!(messages[0], Message::Assistant(_)));
    assert!(matches!(messages[1], Message::Result(_)));
}

#[tokio::test]
async fn test_query_stream_from_stream_streams_both_directions() {
    let transport = MockTransport::with_messages(protocol_messages());

    let input = stream::iter(vec![json!({
        "type": "user",
        "message": {"role": "user", "content": "Dual stream"},
        "session_id": "session-2",
        "parent_tool_use_id": null
    })]);

    let output_stream = query_stream_from_stream(input, None, Some(Box::new(transport)))
        .await
        .expect("query_stream_from_stream");

    let messages = output_stream
        .try_collect::<Vec<_>>()
        .await
        .expect("collect stream output");
    assert_eq!(messages.len(), 2);
    assert!(messages.iter().any(|m| matches!(m, Message::Assistant(_))));
    assert!(messages.iter().any(|m| matches!(m, Message::Result(_))));
}

#[derive(Clone, Default)]
struct FailingReadTransport {
    state: Arc<Mutex<FailingReadState>>,
}

#[derive(Default)]
struct FailingReadState {
    close_calls: usize,
    read_calls: usize,
    writes: Vec<String>,
}

#[async_trait]
impl Transport for FailingReadTransport {
    async fn connect(&mut self) -> Result<()> {
        Ok(())
    }

    async fn write(&mut self, data: &str) -> Result<()> {
        self.state.lock().await.writes.push(data.to_string());
        Ok(())
    }

    async fn end_input(&mut self) -> Result<()> {
        Ok(())
    }

    async fn read_next_message(&mut self) -> Result<Option<Value>> {
        let mut state = self.state.lock().await;
        state.read_calls += 1;
        if state.read_calls == 1 {
            Ok(Some(json!({
                "type": "control_response",
                "response": {"subtype": "success", "request_id": "req_1", "response": {}}
            })))
        } else {
            Err(Error::Other("forced read failure".to_string()))
        }
    }

    async fn close(&mut self) -> Result<()> {
        self.state.lock().await.close_calls += 1;
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
async fn test_one_shot_query_closes_transport_on_read_error() {
    let transport = FailingReadTransport::default();
    let state = transport.state.clone();

    let err = query(
        InputPrompt::Text("trigger error".to_string()),
        None,
        Some(Box::new(transport)),
    )
    .await
    .expect_err("must fail");

    assert!(err.to_string().contains("forced read failure"));
    let state = state.lock().await;
    assert_eq!(state.close_calls, 1);
}

#[tokio::test]
async fn test_stream_early_drop_triggers_cleanup() {
    // Dropping a stream before consuming all messages should invoke cleanup
    // via Query::drop (which spawns an async close task) without panicking.
    let transport = MockTransport::with_messages(protocol_messages());

    let stream = query_stream(
        InputPrompt::Text("Hello".to_string()),
        None,
        Some(Box::new(transport)),
    )
    .await
    .expect("query_stream");

    // Drop the stream without consuming it.
    drop(stream);

    // Allow the spawned cleanup task to run.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // The key assertion is that no panic or leak occurs.
    // For MockTransport (SplitAdapter), the close path exercises
    // Query::drop → spawned close_handle.close().
}
