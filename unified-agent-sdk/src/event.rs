//! Event system and hooks

use futures::{Stream, future::join_all};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};

use crate::types::{ContextUsage, ExitStatus, Role};

pub mod converter;

pub use converter::{EventConverter, normalized_log_to_event};
/// Agent event types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AgentEvent {
    /// Session lifecycle event emitted once when streaming starts.
    SessionStarted {
        /// Executor session identifier.
        session_id: String,
    },
    /// Assistant/user/system message emitted from normalized logs.
    MessageReceived {
        /// Role that produced the message.
        role: Role,
        /// Message content.
        content: String,
    },
    /// Tool call has started.
    ToolCallStarted {
        /// Tool name.
        tool: String,
        /// Tool arguments.
        args: Value,
    },
    /// Tool call completed successfully.
    ToolCallCompleted {
        /// Tool name.
        tool: String,
        /// Tool result payload.
        result: Value,
    },
    /// Tool call failed.
    ToolCallFailed {
        /// Tool name.
        tool: String,
        /// Error message returned by the tool.
        error: String,
    },
    /// Thinking sequence started.
    ThinkingStarted,
    /// Thinking sequence completed with final text.
    ThinkingCompleted {
        /// Thinking content captured from the source stream.
        content: String,
    },
    /// Token usage update.
    TokenUsageUpdated {
        /// Total tokens consumed.
        total: u32,
        /// Token limit (if provided by source stream; can be `0` when unknown).
        limit: u32,
    },
    /// Context window usage update.
    ContextUsageUpdated {
        /// Unified context usage snapshot.
        usage: ContextUsage,
    },
    /// Error event propagated through the session event stream.
    ErrorOccurred {
        /// Error message.
        error: String,
    },
    /// Session lifecycle event emitted once at stream completion.
    SessionCompleted {
        /// Final exit status.
        exit_status: ExitStatus,
    },
}

/// Event type for filtering
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventType {
    /// Filter for [`AgentEvent::SessionStarted`].
    SessionStarted,
    /// Filter for [`AgentEvent::MessageReceived`].
    MessageReceived,
    /// Filter for [`AgentEvent::ToolCallStarted`].
    ToolCallStarted,
    /// Filter for [`AgentEvent::ToolCallCompleted`].
    ToolCallCompleted,
    /// Filter for [`AgentEvent::ToolCallFailed`].
    ToolCallFailed,
    /// Filter for [`AgentEvent::ThinkingStarted`].
    ThinkingStarted,
    /// Filter for [`AgentEvent::ThinkingCompleted`].
    ThinkingCompleted,
    /// Filter for [`AgentEvent::TokenUsageUpdated`].
    TokenUsageUpdated,
    /// Filter for [`AgentEvent::ContextUsageUpdated`].
    ContextUsageUpdated,
    /// Filter for [`AgentEvent::ErrorOccurred`].
    ErrorOccurred,
    /// Filter for [`AgentEvent::SessionCompleted`].
    SessionCompleted,
}

impl AgentEvent {
    /// Returns the static [`EventType`] corresponding to this event value.
    pub fn event_type(&self) -> EventType {
        match self {
            AgentEvent::SessionStarted { .. } => EventType::SessionStarted,
            AgentEvent::MessageReceived { .. } => EventType::MessageReceived,
            AgentEvent::ToolCallStarted { .. } => EventType::ToolCallStarted,
            AgentEvent::ToolCallCompleted { .. } => EventType::ToolCallCompleted,
            AgentEvent::ToolCallFailed { .. } => EventType::ToolCallFailed,
            AgentEvent::ThinkingStarted => EventType::ThinkingStarted,
            AgentEvent::ThinkingCompleted { .. } => EventType::ThinkingCompleted,
            AgentEvent::TokenUsageUpdated { .. } => EventType::TokenUsageUpdated,
            AgentEvent::ContextUsageUpdated { .. } => EventType::ContextUsageUpdated,
            AgentEvent::ErrorOccurred { .. } => EventType::ErrorOccurred,
            AgentEvent::SessionCompleted { .. } => EventType::SessionCompleted,
        }
    }
}

/// Event hook callback
pub type EventHook = Arc<
    dyn for<'event> Fn(&'event AgentEvent) -> Pin<Box<dyn Future<Output = ()> + Send + 'event>>
        + Send
        + Sync,
>;

/// Hook manager
pub struct HookManager {
    hooks: RwLock<HashMap<EventType, Vec<EventHook>>>,
}

impl HookManager {
    /// Creates an empty hook manager.
    pub fn new() -> Self {
        Self {
            hooks: RwLock::new(HashMap::new()),
        }
    }

    /// Registers a hook for a specific [`EventType`].
    pub fn register(&self, event_type: EventType, hook: EventHook) {
        let mut hooks = self
            .hooks
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        hooks.entry(event_type).or_default().push(hook);
    }

    /// Triggers all hooks registered for `event`'s type.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::sync::Arc;
    /// use unified_agent_sdk::{AgentEvent, EventType, HookManager, Role};
    ///
    /// async fn run() {
    ///     let hooks = HookManager::new();
    ///     hooks.register(
    ///         EventType::MessageReceived,
    ///         Arc::new(|_event| Box::pin(async move {})),
    ///     );
    ///
    ///     hooks.trigger(&AgentEvent::MessageReceived {
    ///         role: Role::Assistant,
    ///         content: "hello".to_string(),
    ///     }).await;
    /// }
    /// ```
    pub async fn trigger(&self, event: &AgentEvent) {
        let hooks = {
            let hooks_map = self
                .hooks
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            hooks_map.get(&event.event_type()).cloned()
        };

        if let Some(hooks) = hooks {
            join_all(hooks.into_iter().map(|hook| hook(event))).await;
        }
    }
}

impl Default for HookManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Event stream
pub struct EventStream {
    inner: Pin<Box<dyn Stream<Item = AgentEvent> + Send>>,
}

impl EventStream {
    /// Wraps any `Stream<Item = AgentEvent>` into the SDK event stream type.
    pub fn new(stream: Pin<Box<dyn Stream<Item = AgentEvent> + Send>>) -> Self {
        Self { inner: stream }
    }
}

impl Stream for EventStream {
    type Item = AgentEvent;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Barrier;
    use tokio::time::timeout;

    #[tokio::test]
    async fn trigger_executes_hooks_concurrently() {
        let manager = HookManager::new();
        let barrier = Arc::new(Barrier::new(3));

        for _ in 0..2 {
            let barrier = Arc::clone(&barrier);
            manager.register(
                EventType::ThinkingStarted,
                Arc::new(move |_event| {
                    let barrier = Arc::clone(&barrier);
                    Box::pin(async move {
                        barrier.wait().await;
                    })
                }),
            );
        }

        let event = AgentEvent::ThinkingStarted;
        timeout(Duration::from_secs(1), async {
            let _ = tokio::join!(manager.trigger(&event), barrier.wait());
        })
        .await
        .expect("hooks should all start and complete without sequential blocking");
    }
}
