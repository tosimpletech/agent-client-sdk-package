//! Session management primitives.
//!
//! [`AgentSession`] is a lightweight handle returned by executor `spawn`/`resume`
//! operations. It exposes metadata helpers and a streaming pipeline that converts
//! provider raw logs into unified [`crate::event::AgentEvent`] values.

use async_trait::async_trait;
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

/// Serializable metadata snapshot for persistence or resume bookkeeping.
#[derive(Debug, Clone)]
pub struct SessionMetadata {
    /// Executor session identifier.
    pub session_id: String,
    /// Executor backend type.
    pub executor_type: ExecutorType,
    /// Metadata creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last known source message id, if available.
    pub last_message_id: Option<String>,
    /// Session working directory.
    pub working_dir: PathBuf,
    /// Optional context window capacity override passed at session creation.
    pub context_window_override_tokens: Option<u32>,
}

/// Session resume descriptor used by higher-level orchestrators.
#[derive(Debug, Clone)]
pub struct SessionResume {
    /// Existing session identifier.
    pub session_id: String,
    /// Optional message id used for rewind/reset semantics.
    pub reset_to_message: Option<String>,
}

/// Active agent session handle.
///
/// This value is intentionally lightweight and provider-agnostic. It does not own
/// subprocess handles directly in the current implementation.
pub struct AgentSession {
    /// Executor session identifier.
    pub session_id: String,
    /// Executor backend type.
    pub executor_type: ExecutorType,
    /// Working directory used by this session.
    pub working_dir: PathBuf,
    /// Session creation timestamp captured when the session is established.
    pub created_at: DateTime<Utc>,
    /// Last known source message id, if available.
    pub last_message_id: Option<String>,
    /// Optional context window capacity override for context usage normalization.
    pub context_window_override_tokens: Option<u32>,
    lifecycle_controller: SessionControllerRef,
}

impl AgentSession {
    /// Creates a detached session handle with default lifecycle behavior.
    ///
    /// Detached sessions treat `wait` as immediately successful and `cancel` as
    /// a no-op.
    pub fn new(
        session_id: impl Into<String>,
        executor_type: ExecutorType,
        working_dir: impl Into<PathBuf>,
        context_window_override_tokens: Option<u32>,
    ) -> Self {
        Self::from_parts(
            SessionMetadata {
                session_id: session_id.into(),
                executor_type,
                created_at: Utc::now(),
                last_message_id: None,
                working_dir: working_dir.into(),
                context_window_override_tokens,
            },
            Arc::new(DetachedSessionLifecycleController),
        )
    }

    /// Restores a detached session from persisted metadata.
    pub fn from_metadata(metadata: SessionMetadata) -> Self {
        Self::from_parts(metadata, Arc::new(DetachedSessionLifecycleController))
    }

    pub(crate) fn from_metadata_with_exit_status(
        metadata: SessionMetadata,
        exit_status: ExitStatus,
    ) -> Self {
        Self::from_parts(
            metadata,
            Arc::new(CompletedSessionLifecycleController { exit_status }),
        )
    }

    fn from_parts(metadata: SessionMetadata, lifecycle_controller: SessionControllerRef) -> Self {
        Self {
            session_id: metadata.session_id,
            executor_type: metadata.executor_type,
            working_dir: metadata.working_dir,
            created_at: metadata.created_at,
            last_message_id: metadata.last_message_id,
            context_window_override_tokens: metadata.context_window_override_tokens,
            lifecycle_controller,
        }
    }

    /// Build an event stream pipeline:
    /// raw logs -> normalized logs -> unified events.
    ///
    /// Hooks are triggered for each emitted event when provided.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use futures::stream;
    /// use std::path::PathBuf;
    /// use unified_agent_sdk::{AgentSession, CodexLogNormalizer, ExecutorType, session::RawLogStream};
    ///
    /// let session = AgentSession::new("s1", ExecutorType::Codex, PathBuf::from("."), None);
    ///
    /// let raw_logs: RawLogStream = Box::pin(stream::iter(vec![
    ///     br#"{"type":"item.completed","item":{"type":"agent_message","id":"m1","text":"hello"}}"#
    ///         .to_vec(),
    ///     b"\n".to_vec(),
    /// ]));
    ///
    /// let _events = session.event_stream(raw_logs, Box::new(CodexLogNormalizer::new()), None);
    /// ```
    pub fn event_stream(
        &self,
        raw_logs: RawLogStream,
        normalizer: Box<dyn LogNormalizer + Send>,
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
            context_window_override_tokens: self.context_window_override_tokens,
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

    /// Returns immutable metadata snapshot for the session.
    pub fn metadata(&self) -> SessionMetadata {
        SessionMetadata {
            session_id: self.session_id.clone(),
            executor_type: self.executor_type,
            created_at: self.created_at,
            last_message_id: self.last_message_id.clone(),
            working_dir: self.working_dir.clone(),
            context_window_override_tokens: self.context_window_override_tokens,
        }
    }

    /// Waits for session completion and returns a summarized exit status.
    pub async fn wait(&mut self) -> Result<ExitStatus> {
        self.lifecycle_controller.wait().await
    }

    /// Requests cancellation of the active session.
    pub async fn cancel(&mut self) -> Result<()> {
        self.lifecycle_controller.cancel().await
    }
}

#[async_trait]
pub(crate) trait SessionLifecycleController: Send + Sync {
    async fn wait(&self) -> Result<ExitStatus>;
    async fn cancel(&self) -> Result<()>;
}

struct DetachedSessionLifecycleController;

#[async_trait]
impl SessionLifecycleController for DetachedSessionLifecycleController {
    async fn wait(&self) -> Result<ExitStatus> {
        Ok(ExitStatus {
            code: None,
            success: true,
        })
    }

    async fn cancel(&self) -> Result<()> {
        Ok(())
    }
}

struct CompletedSessionLifecycleController {
    exit_status: ExitStatus,
}

#[async_trait]
impl SessionLifecycleController for CompletedSessionLifecycleController {
    async fn wait(&self) -> Result<ExitStatus> {
        Ok(self.exit_status)
    }

    async fn cancel(&self) -> Result<()> {
        Ok(())
    }
}

type SessionControllerRef = Arc<dyn SessionLifecycleController>;

struct EventPipelineState {
    session_id: String,
    raw_logs: RawLogStream,
    normalizer: Box<dyn LogNormalizer + Send>,
    hooks: Option<Arc<HookManager>>,
    pending_events: VecDeque<AgentEvent>,
    emitted_started: bool,
    finished: bool,
    saw_error: bool,
    context_window_override_tokens: Option<u32>,
}

impl EventPipelineState {
    fn push_logs(&mut self, logs: Vec<crate::log::NormalizedLog>) {
        for log in logs {
            for event in converter::from_normalized_log_with_context_override(
                log,
                self.context_window_override_tokens,
            ) {
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
    use async_trait::async_trait;
    use futures::{StreamExt, stream};
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::Mutex;

    use crate::{
        event::EventType,
        log::{ActionType, NormalizedLog},
        types::{ContextUsageSource, Role, ToolStatus},
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
        let session = AgentSession::new("session-1", ExecutorType::Codex, PathBuf::from("."), None);

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

        let captured = received_messages.lock().await.clone();
        assert_eq!(captured, vec!["hello".to_string()]);
    }

    #[tokio::test]
    async fn session_event_stream_marks_completion_as_failed_when_errors_seen() {
        let session = AgentSession::new(
            "session-2",
            ExecutorType::ClaudeCode,
            PathBuf::from("."),
            None,
        );

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

    struct UnknownLimitNormalizer;

    impl LogNormalizer for UnknownLimitNormalizer {
        fn normalize(&mut self, _chunk: &[u8]) -> Vec<NormalizedLog> {
            Vec::new()
        }

        fn flush(&mut self) -> Vec<NormalizedLog> {
            vec![NormalizedLog::TokenUsage {
                total: 15,
                limit: 0,
            }]
        }
    }

    #[tokio::test]
    async fn session_event_stream_applies_context_window_override() {
        let session = AgentSession::new(
            "session-3",
            ExecutorType::Codex,
            PathBuf::from("."),
            Some(60),
        );

        let raw_logs: RawLogStream = Box::pin(stream::iter(Vec::<Vec<u8>>::new()));
        let events = session
            .event_stream(raw_logs, Box::new(UnknownLimitNormalizer), None)
            .collect::<Vec<_>>()
            .await;

        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::ContextUsageUpdated { usage }
                if usage.used_tokens == 15
                    && usage.window_tokens == Some(60)
                    && usage.remaining_tokens == Some(45)
                    && usage.source == ContextUsageSource::ConfigOverride
        )));
    }

    #[tokio::test]
    async fn wait_defaults_to_completed_success_when_unmanaged() {
        let mut session = AgentSession::new(
            "session-unmanaged",
            ExecutorType::Codex,
            PathBuf::from("."),
            None,
        );

        let exit_status = session.wait().await.expect("wait should succeed");
        assert_eq!(
            exit_status,
            ExitStatus {
                code: None,
                success: true
            }
        );
    }

    #[tokio::test]
    async fn wait_uses_session_lifecycle_controller() {
        let mut session = AgentSession::from_metadata_with_exit_status(
            SessionMetadata {
                session_id: "session-managed".to_string(),
                executor_type: ExecutorType::ClaudeCode,
                created_at: Utc::now(),
                last_message_id: None,
                working_dir: PathBuf::from("."),
                context_window_override_tokens: None,
            },
            ExitStatus {
                code: Some(17),
                success: false,
            },
        );

        let first = session.wait().await.expect("wait should use controller");
        assert_eq!(
            first,
            ExitStatus {
                code: Some(17),
                success: false
            }
        );

        let second = session
            .wait()
            .await
            .expect("second wait should remain stable");
        assert_eq!(
            second,
            ExitStatus {
                code: Some(17),
                success: false
            }
        );
    }

    struct CancelProbeController {
        cancelled: Arc<AtomicBool>,
    }

    #[async_trait]
    impl SessionLifecycleController for CancelProbeController {
        async fn wait(&self) -> Result<ExitStatus> {
            Ok(ExitStatus {
                code: None,
                success: true,
            })
        }

        async fn cancel(&self) -> Result<()> {
            self.cancelled.store(true, Ordering::Relaxed);
            Ok(())
        }
    }

    #[tokio::test]
    async fn cancel_delegates_to_registered_lifecycle_controller() {
        let session_id = "session-cancel".to_string();
        let cancelled = Arc::new(AtomicBool::new(false));

        let mut session = AgentSession::from_parts(
            SessionMetadata {
                session_id,
                executor_type: ExecutorType::ClaudeCode,
                created_at: Utc::now(),
                last_message_id: None,
                working_dir: PathBuf::from("."),
                context_window_override_tokens: None,
            },
            Arc::new(CancelProbeController {
                cancelled: Arc::clone(&cancelled),
            }),
        );

        session.cancel().await.expect("cancel should succeed");
        assert!(cancelled.load(Ordering::Relaxed));
    }
}
