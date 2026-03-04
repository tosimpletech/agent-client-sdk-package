//! Stateless conversion from normalized logs to agent events.

use serde_json::Value;

use crate::{event::AgentEvent, log::NormalizedLog, types::ToolStatus};

/// Converts normalized logs into SDK events.
#[derive(Debug, Default, Clone, Copy)]
pub struct EventConverter;

impl EventConverter {
    /// Converts a single normalized log entry into an event.
    ///
    /// Returns `None` when the log entry has no direct event mapping.
    pub fn convert(log: NormalizedLog) -> Option<AgentEvent> {
        match log {
            NormalizedLog::Message { role, content } => {
                Some(AgentEvent::MessageReceived { role, content })
            }
            NormalizedLog::ToolCall {
                name, args, status, ..
            } => match status {
                ToolStatus::Started => Some(AgentEvent::ToolCallStarted { tool: name, args }),
                ToolStatus::Completed => Some(AgentEvent::ToolCallCompleted {
                    tool: name,
                    result: args,
                }),
                ToolStatus::Failed => Some(AgentEvent::ToolCallFailed {
                    tool: name,
                    error: extract_tool_error(&args),
                }),
                ToolStatus::Running => None,
            },
            NormalizedLog::Thinking { content } => Some(AgentEvent::ThinkingCompleted { content }),
            NormalizedLog::TokenUsage { total, limit } => {
                Some(AgentEvent::TokenUsageUpdated { total, limit })
            }
            NormalizedLog::Error { message, .. } => {
                Some(AgentEvent::ErrorOccurred { error: message })
            }
        }
    }
}

/// Convenience helper for one-off conversion.
pub fn normalized_log_to_event(log: NormalizedLog) -> Option<AgentEvent> {
    EventConverter::convert(log)
}

fn extract_tool_error(args: &Value) -> String {
    args.get("error")
        .or_else(|| args.get("message"))
        .map(|value| match value {
            Value::String(message) => message.clone(),
            other => other.to_string(),
        })
        .filter(|message| !message.is_empty() && message != "null")
        .unwrap_or_else(|| "tool call failed".to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{
        event::{AgentEvent, EventConverter},
        log::{ActionType, NormalizedLog},
        types::{Role, ToolStatus},
    };

    #[test]
    fn maps_message_to_message_received() {
        let event = EventConverter::convert(NormalizedLog::Message {
            role: Role::Assistant,
            content: "hello".to_string(),
        });

        match event {
            Some(AgentEvent::MessageReceived { role, content }) => {
                assert_eq!(role, Role::Assistant);
                assert_eq!(content, "hello");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn maps_tool_call_status_to_expected_events() {
        let started = EventConverter::convert(NormalizedLog::ToolCall {
            name: "read_file".to_string(),
            args: json!({ "path": "README.md" }),
            status: ToolStatus::Started,
            action: ActionType::FileRead {
                path: "README.md".to_string(),
            },
        });
        let completed = EventConverter::convert(NormalizedLog::ToolCall {
            name: "read_file".to_string(),
            args: json!({ "content": "ok" }),
            status: ToolStatus::Completed,
            action: ActionType::FileRead {
                path: "README.md".to_string(),
            },
        });
        let failed = EventConverter::convert(NormalizedLog::ToolCall {
            name: "read_file".to_string(),
            args: json!({ "error": "permission denied" }),
            status: ToolStatus::Failed,
            action: ActionType::FileRead {
                path: "README.md".to_string(),
            },
        });

        match started {
            Some(AgentEvent::ToolCallStarted { tool, args }) => {
                assert_eq!(tool, "read_file");
                assert_eq!(args, json!({ "path": "README.md" }));
            }
            other => panic!("unexpected started event: {other:?}"),
        }

        match completed {
            Some(AgentEvent::ToolCallCompleted { tool, result }) => {
                assert_eq!(tool, "read_file");
                assert_eq!(result, json!({ "content": "ok" }));
            }
            other => panic!("unexpected completed event: {other:?}"),
        }

        match failed {
            Some(AgentEvent::ToolCallFailed { tool, error }) => {
                assert_eq!(tool, "read_file");
                assert_eq!(error, "permission denied");
            }
            other => panic!("unexpected failed event: {other:?}"),
        }
    }

    #[test]
    fn maps_thinking_to_thinking_completed() {
        let event = EventConverter::convert(NormalizedLog::Thinking {
            content: "analysis".to_string(),
        });

        match event {
            Some(AgentEvent::ThinkingCompleted { content }) => {
                assert_eq!(content, "analysis");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn maps_token_usage_to_token_usage_updated() {
        let event = EventConverter::convert(NormalizedLog::TokenUsage {
            total: 12,
            limit: 100,
        });

        match event {
            Some(AgentEvent::TokenUsageUpdated { total, limit }) => {
                assert_eq!(total, 12);
                assert_eq!(limit, 100);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn ignores_tool_call_running_state() {
        let event = EventConverter::convert(NormalizedLog::ToolCall {
            name: "read_file".to_string(),
            args: json!({ "path": "README.md" }),
            status: ToolStatus::Running,
            action: ActionType::FileRead {
                path: "README.md".to_string(),
            },
        });

        assert!(event.is_none());
    }
}
