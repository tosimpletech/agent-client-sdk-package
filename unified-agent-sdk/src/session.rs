//! Session management

use chrono::{DateTime, Utc};
use futures::{Stream, StreamExt, stream};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use crate::{
    error::Result,
    event::{AgentEvent, EventStream, HookManager, converter},
    log::LogNormalizer,
    types::{ExecutorType, ExitStatus},
};

/// Raw log stream emitted by an executor process.
pub type RawLogStream = Pin<Box<dyn Stream<Item = Vec<u8>> + Send>>;

/// Session metadata for persistence
#[derive(Debug, Clone)]
pub struct SessionMetadata {
    pub session_id: String,
    pub executor_type: ExecutorType,
    pub created_at: DateTime<Utc>,
    pub last_message_id: Option<String>,
    pub working_dir: PathBuf,
}

/// Session resume information
#[derive(Debug, Clone)]
pub struct SessionResume {
    pub session_id: String,
    pub reset_to_message: Option<String>,
}

/// Active agent session
pub struct AgentSession {
    pub session_id: String,
    pub executor_type: ExecutorType,
    pub working_dir: PathBuf,
    // Internal process handle (implementation-specific)
}

impl AgentSession {
    /// Build an event stream pipeline:
    /// raw logs -> normalized logs -> unified events.
    ///
    /// Hooks are triggered for each emitted event when provided.
    pub fn event_stream(
        &self,
        raw_logs: RawLogStream,
        normalizer: Box<dyn LogNormalizer + Send + Sync>,
        hooks: Option<Arc<HookManager>>,
    ) -> EventStream {
        let state = EventPipelineState {
            session_id: self.session_id.clone(),
            raw_logs,
            normalizer,
            hooks,
            pending_events: VecDeque::new(),
            emitted_started: false,
            finished: false,
            saw_error: false,
        };

        let stream = stream::unfold(state, |mut state| async move {
            loop {
                if let Some(event) = state.pending_events.pop_front() {
                    if let Some(hook_manager) = &state.hooks {
                        hook_manager.trigger(&event).await;
                    }
                    return Some((event, state));
                }

                if !state.emitted_started {
                    state.emitted_started = true;
                    state.push_event(AgentEvent::SessionStarted {
                        session_id: state.session_id.clone(),
                    });
                    continue;
                }

                if state.finished {
                    return None;
                }

                match state.raw_logs.next().await {
                    Some(chunk) => {
                        let logs = state.normalizer.normalize(&chunk);
                        state.push_logs(logs);
                    }
                    None => {
                        let logs = state.normalizer.flush();
                        state.push_logs(logs);
                        state.push_event(AgentEvent::SessionCompleted {
                            exit_status: ExitStatus {
                                code: None,
                                success: !state.saw_error,
                            },
                        });
                        state.finished = true;
                    }
                }
            }
        });

        EventStream::new(Box::pin(stream))
    }

    pub fn metadata(&self) -> SessionMetadata {
        SessionMetadata {
            session_id: self.session_id.clone(),
            executor_type: self.executor_type,
            created_at: Utc::now(),
            last_message_id: None,
            working_dir: self.working_dir.clone(),
        }
    }

    pub async fn wait(&mut self) -> Result<ExitStatus> {
        // TODO: implement wait
        Ok(ExitStatus {
            code: Some(0),
            success: true,
        })
    }

    pub async fn cancel(&mut self) -> Result<()> {
        // TODO: implement cancel
        Ok(())
    }
}

struct EventPipelineState {
    session_id: String,
    raw_logs: RawLogStream,
    normalizer: Box<dyn LogNormalizer + Send + Sync>,
    hooks: Option<Arc<HookManager>>,
    pending_events: VecDeque<AgentEvent>,
    emitted_started: bool,
    finished: bool,
    saw_error: bool,
}

impl EventPipelineState {
    fn push_logs(&mut self, logs: Vec<crate::log::NormalizedLog>) {
        for log in logs {
            for event in converter::from_normalized_log(log) {
                self.push_event(event);
            }
        }
    }

    fn push_event(&mut self, event: AgentEvent) {
        if matches!(event, AgentEvent::ErrorOccurred { .. }) {
            self.saw_error = true;
        }
        self.pending_events.push_back(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{StreamExt, stream};
    use serde_json::json;
    use tokio::sync::Mutex;

    use crate::{
        event::EventType,
        log::{ActionType, NormalizedLog},
        types::{Role, ToolStatus},
    };

    struct TestNormalizer;

    impl LogNormalizer for TestNormalizer {
        fn normalize(&mut self, chunk: &[u8]) -> Vec<NormalizedLog> {
            match chunk {
                b"message" => vec![NormalizedLog::Message {
                    role: Role::Assistant,
                    content: "hello".to_string(),
                }],
                b"tool-start" => vec![NormalizedLog::ToolCall {
                    name: "bash".to_string(),
                    args: json!({"cmd":"ls"}),
                    status: ToolStatus::Started,
                    action: ActionType::CommandRun {
                        command: "ls".to_string(),
                    },
                }],
                b"tool-done" => vec![NormalizedLog::ToolCall {
                    name: "bash".to_string(),
                    args: json!({"cmd":"ls"}),
                    status: ToolStatus::Completed,
                    action: ActionType::CommandRun {
                        command: "ls".to_string(),
                    },
                }],
                b"error" => vec![NormalizedLog::Error {
                    error_type: "execution_failed".to_string(),
                    message: "boom".to_string(),
                }],
                _ => Vec::new(),
            }
        }

        fn flush(&mut self) -> Vec<NormalizedLog> {
            vec![NormalizedLog::TokenUsage {
                total: 10,
                limit: 100,
            }]
        }
    }

    #[tokio::test]
    async fn session_event_stream_builds_pipeline_and_triggers_hooks() {
        let session = AgentSession {
            session_id: "session-1".to_string(),
            executor_type: ExecutorType::Codex,
            working_dir: PathBuf::from("."),
        };

        let received_messages = Arc::new(Mutex::new(Vec::<String>::new()));
        let hooks = Arc::new(HookManager::new());
        hooks.register(
            EventType::MessageReceived,
            Arc::new({
                let received_messages = Arc::clone(&received_messages);
                move |event| {
                    let received_messages = Arc::clone(&received_messages);
                    let content = match event {
                        AgentEvent::MessageReceived { content, .. } => Some(content.clone()),
                        _ => None,
                    };
                    Box::pin(async move {
                        if let Some(content) = content {
                            received_messages.lock().await.push(content);
                        }
                    })
                }
            }),
        );

        let raw_logs: RawLogStream = Box::pin(stream::iter(vec![
            b"message".to_vec(),
            b"tool-start".to_vec(),
            b"tool-done".to_vec(),
        ]));

        let events = session
            .event_stream(raw_logs, Box::new(TestNormalizer), Some(hooks))
            .collect::<Vec<_>>()
            .await;

        assert!(matches!(
            events.first(),
            Some(AgentEvent::SessionStarted { session_id }) if session_id == "session-1"
        ));
        assert!(events
            .iter()
            .any(|event| matches!(event, AgentEvent::MessageReceived { content, .. } if content == "hello")));
        assert!(events.iter().any(
            |event| matches!(event, AgentEvent::ToolCallStarted { tool, .. } if tool == "bash")
        ));
        assert!(events.iter().any(
            |event| matches!(event, AgentEvent::ToolCallCompleted { tool, .. } if tool == "bash")
        ));
        assert!(matches!(
            events.last(),
            Some(AgentEvent::SessionCompleted { exit_status }) if exit_status.success
        ));

        let captured = received_messages.lock().await.clone();
        assert_eq!(captured, vec!["hello".to_string()]);
    }

    #[tokio::test]
    async fn session_event_stream_marks_completion_as_failed_when_errors_seen() {
        let session = AgentSession {
            session_id: "session-2".to_string(),
            executor_type: ExecutorType::ClaudeCode,
            working_dir: PathBuf::from("."),
        };

        let raw_logs: RawLogStream = Box::pin(stream::iter(vec![b"error".to_vec()]));
        let events = session
            .event_stream(raw_logs, Box::new(TestNormalizer), None)
            .collect::<Vec<_>>()
            .await;

        assert!(events.iter().any(
            |event| matches!(event, AgentEvent::ErrorOccurred { error } if error.contains("boom"))
        ));
        assert!(matches!(
            events.last(),
            Some(AgentEvent::SessionCompleted { exit_status }) if !exit_status.success
        ));
    }
}
