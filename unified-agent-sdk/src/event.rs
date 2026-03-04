//! Event system and hooks

use futures::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};

use crate::types::{ExitStatus, Role};

pub mod converter;

pub use converter::{EventConverter, normalized_log_to_event};
/// Agent event types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AgentEvent {
    SessionStarted { session_id: String },
    MessageReceived { role: Role, content: String },
    ToolCallStarted { tool: String, args: Value },
    ToolCallCompleted { tool: String, result: Value },
    ToolCallFailed { tool: String, error: String },
    ThinkingStarted,
    ThinkingCompleted { content: String },
    TokenUsageUpdated { total: u32, limit: u32 },
    ErrorOccurred { error: String },
    SessionCompleted { exit_status: ExitStatus },
}

/// Event type for filtering
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventType {
    SessionStarted,
    MessageReceived,
    ToolCallStarted,
    ToolCallCompleted,
    ToolCallFailed,
    ThinkingStarted,
    ThinkingCompleted,
    TokenUsageUpdated,
    ErrorOccurred,
    SessionCompleted,
}

impl AgentEvent {
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
            AgentEvent::ErrorOccurred { .. } => EventType::ErrorOccurred,
            AgentEvent::SessionCompleted { .. } => EventType::SessionCompleted,
        }
    }
}

/// Event hook callback
pub type EventHook =
    Arc<dyn Fn(&AgentEvent) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Hook manager
pub struct HookManager {
    hooks: RwLock<HashMap<EventType, Vec<EventHook>>>,
}

impl HookManager {
    pub fn new() -> Self {
        Self {
            hooks: RwLock::new(HashMap::new()),
        }
    }

    pub fn register(&self, event_type: EventType, hook: EventHook) {
        let mut hooks = self.hooks.write().unwrap();
        hooks.entry(event_type).or_default().push(hook);
    }

    pub async fn trigger(&self, event: &AgentEvent) {
        let hooks = {
            let hooks_map = self.hooks.read().unwrap();
            hooks_map.get(&event.event_type()).cloned()
        };

        if let Some(hooks) = hooks {
            for hook in hooks {
                hook(event).await;
            }
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
