use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use claude_code::{
    ClaudeAgentOptions, ClaudeSdkClient, Error, InputPrompt, Result, Transport,
    TransportSplitResult, query, split_with_adapter,
};
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

#[tokio::test]
async fn test_control_response_error_propagated() {
    // The CLI responds with an error to the init request.
    let transport = MockTransport::with_messages(vec![json!({
        "type": "control_response",
        "response": {
            "subtype": "error",
            "request_id": "req_1",
            "error": "Initialization failed: bad config"
        }
    })]);

    let mut client = ClaudeSdkClient::new_with_transport(None, Box::new(transport));
    let err = client.connect(None).await.expect_err("must fail");
    assert!(err.to_string().contains("Initialization failed"));
}

#[tokio::test]
async fn test_transport_read_error_propagated_to_consumer() {
    // Transport provides init response, then an error on next read.
    struct FailAfterInitTransport {
        state: Arc<Mutex<FailAfterInitState>>,
    }

    #[derive(Default)]
    struct FailAfterInitState {
        read_calls: usize,
        close_calls: usize,
    }

    #[async_trait]
    impl Transport for FailAfterInitTransport {
        async fn connect(&mut self) -> Result<()> {
            Ok(())
        }
        async fn write(&mut self, _data: &str) -> Result<()> {
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
                Err(Error::Other("transport broken".to_string()))
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

    let state = Arc::new(Mutex::new(FailAfterInitState::default()));
    let transport = FailAfterInitTransport {
        state: state.clone(),
    };

    let err = query(
        InputPrompt::Text("hello".to_string()),
        None,
        Some(Box::new(transport)),
    )
    .await
    .expect_err("must fail");

    assert!(err.to_string().contains("transport broken"));
}

#[tokio::test]
async fn test_can_use_tool_requires_messages_not_text() {
    use claude_code::{PermissionResult, PermissionResultAllow, ToolPermissionContext};
    use futures::FutureExt;

    let callback = Arc::new(
        |_tool_name: String, _input: Value, _ctx: ToolPermissionContext| {
            async move { Ok(PermissionResult::Allow(PermissionResultAllow::default())) }.boxed()
        },
    );

    let options = ClaudeAgentOptions {
        can_use_tool: Some(callback),
        ..Default::default()
    };

    let transport = MockTransport::with_messages(vec![]);

    // Text prompt with can_use_tool should fail.
    let err = query(
        InputPrompt::Text("test".to_string()),
        Some(options),
        Some(Box::new(transport)),
    )
    .await
    .expect_err("must fail");

    assert!(err.to_string().contains("streaming mode"));
}

#[tokio::test]
async fn test_can_use_tool_conflicts_with_permission_prompt_tool_name() {
    use claude_code::{PermissionResult, PermissionResultAllow, ToolPermissionContext};
    use futures::FutureExt;

    let callback = Arc::new(
        |_tool_name: String, _input: Value, _ctx: ToolPermissionContext| {
            async move { Ok(PermissionResult::Allow(PermissionResultAllow::default())) }.boxed()
        },
    );

    let options = ClaudeAgentOptions {
        can_use_tool: Some(callback),
        permission_prompt_tool_name: Some("custom".to_string()),
        ..Default::default()
    };

    let transport = MockTransport::with_messages(vec![]);

    let err = query(
        InputPrompt::Messages(vec![json!({"type": "user", "message": {"role": "user", "content": "test"}, "session_id": "", "parent_tool_use_id": null})]),
        Some(options),
        Some(Box::new(transport)),
    )
    .await
    .expect_err("must fail");

    assert!(err.to_string().contains("permission_prompt_tool_name"));
}

#[tokio::test]
async fn test_empty_transport_returns_no_messages() {
    // Transport provides init response and then EOF. No SDK messages.
    let transport = MockTransport::with_messages(vec![json!({
        "type": "control_response",
        "response": {"subtype": "success", "request_id": "req_1", "response": {}}
    })]);

    let messages = query(
        InputPrompt::Text("hello".to_string()),
        None,
        Some(Box::new(transport)),
    )
    .await
    .expect("query");

    assert!(messages.is_empty());
}
