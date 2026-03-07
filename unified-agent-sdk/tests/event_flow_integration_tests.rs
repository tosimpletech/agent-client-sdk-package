use chrono::Utc;
use futures::{StreamExt, stream};
use std::path::PathBuf;
use unified_agent_sdk::{
    AgentEvent, AgentSession, ClaudeCodeLogNormalizer, CodexLogNormalizer, ContextUsageSource,
    ExecutorType, session::RawLogStream,
};

fn test_session(session_id: &str, executor_type: ExecutorType) -> AgentSession {
    AgentSession {
        session_id: session_id.to_string(),
        executor_type,
        working_dir: PathBuf::from("."),
        created_at: Utc::now(),
        last_message_id: None,
        context_window_override_tokens: None,
    }
}

#[tokio::test]
async fn codex_pipeline_emits_successful_event_flow() {
    let session = test_session("codex-success", ExecutorType::Codex);
    let raw = concat!(
        r#"{"type":"item.completed","item":{"type":"agent_message","id":"m1","text":"hello from codex"}}"#,
        "\n",
        r#"{"type":"turn.completed","usage":{"input_tokens":3,"cached_input_tokens":1,"output_tokens":2}}"#,
        "\n"
    );

    let raw_logs: RawLogStream = Box::pin(stream::iter(vec![raw.as_bytes().to_vec()]));
    let events = session
        .event_stream(raw_logs, Box::new(CodexLogNormalizer::new()), None)
        .collect::<Vec<_>>()
        .await;

    assert!(matches!(
        events.first(),
        Some(AgentEvent::SessionStarted { session_id }) if session_id == "codex-success"
    ));
    assert!(events.iter().any(
        |event| matches!(event, AgentEvent::MessageReceived { content, .. } if content == "hello from codex")
    ));
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::ContextUsageUpdated { usage }
            if usage.used_tokens == 6
                && usage.window_tokens.is_none()
                && usage.source == ContextUsageSource::Unknown
    )));
    assert!(matches!(
        events.last(),
        Some(AgentEvent::SessionCompleted { exit_status }) if exit_status.success
    ));
}

#[tokio::test]
async fn codex_pipeline_marks_session_failed_on_error_event() {
    let session = test_session("codex-failure", ExecutorType::Codex);
    let raw_logs: RawLogStream = Box::pin(stream::iter(vec![b"{not-json}\n".to_vec()]));

    let events = session
        .event_stream(raw_logs, Box::new(CodexLogNormalizer::new()), None)
        .collect::<Vec<_>>()
        .await;

    assert!(
        events
            .iter()
            .any(|event| matches!(event, AgentEvent::ErrorOccurred { .. }))
    );
    assert!(matches!(
        events.last(),
        Some(AgentEvent::SessionCompleted { exit_status }) if !exit_status.success
    ));
}

#[tokio::test]
async fn claude_pipeline_emits_message_and_usage_events() {
    let session = test_session("claude-success", ExecutorType::ClaudeCode);
    let raw = concat!(
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hello from claude"}],"model":"claude-3-7-sonnet"}}"#,
        "\n",
        r#"{"type":"result","subtype":"success","duration_ms":1,"duration_api_ms":1,"is_error":false,"num_turns":1,"session_id":"s1","usage":{"input_tokens":4,"output_tokens":6,"limit":100}}"#,
        "\n"
    );

    let raw_logs: RawLogStream = Box::pin(stream::iter(vec![raw.as_bytes().to_vec()]));
    let events = session
        .event_stream(raw_logs, Box::new(ClaudeCodeLogNormalizer::new()), None)
        .collect::<Vec<_>>()
        .await;

    assert!(events.iter().any(
        |event| matches!(event, AgentEvent::MessageReceived { content, .. } if content == "hello from claude")
    ));
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::ContextUsageUpdated { usage }
            if usage.used_tokens == 10
                && usage.window_tokens == Some(100)
                && usage.remaining_tokens == Some(90)
                && usage.source == ContextUsageSource::ProviderReported
    )));
    assert!(matches!(
        events.last(),
        Some(AgentEvent::SessionCompleted { exit_status }) if exit_status.success
    ));
}
