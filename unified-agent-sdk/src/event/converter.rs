//! Stateless conversion from normalized logs to agent events.

use serde_json::{Value, json};

use crate::{
    event::AgentEvent,
    log::{ActionType, NormalizedLog},
    types::ToolStatus,
};

/// Converts normalized logs into SDK events.
#[derive(Debug, Default, Clone, Copy)]
pub struct EventConverter;

impl EventConverter {
    /// Converts a single normalized log entry into one event.
    ///
    /// Returns `None` when the log entry has no direct one-to-one mapping.
    pub fn convert(log: NormalizedLog) -> Option<AgentEvent> {
        match log {
            NormalizedLog::Message { role, content } => {
                Some(AgentEvent::MessageReceived { role, content })
            }
            NormalizedLog::ToolCall {
                name,
                args,
                status,
                action,
            } => match status {
                ToolStatus::Started => Some(AgentEvent::ToolCallStarted { tool: name, args }),
                ToolStatus::Completed => Some(AgentEvent::ToolCallCompleted {
                    tool: name,
                    result: completed_tool_result(args, action),
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

/// Convert one normalized log entry into one or more [`AgentEvent`] values.
///
/// This powers session-level pipelines where a single normalized record can fan out
/// to multiple events (for example, thinking start/completed).
pub fn from_normalized_log(log: NormalizedLog) -> Vec<AgentEvent> {
    match log {
        NormalizedLog::Message { role, content } => {
            vec![AgentEvent::MessageReceived { role, content }]
        }
        NormalizedLog::ToolCall {
            name,
            args,
            status,
            action,
        } => map_tool_call(name, args, status, action),
        NormalizedLog::Thinking { content } => vec![
            AgentEvent::ThinkingStarted,
            AgentEvent::ThinkingCompleted { content },
        ],
        NormalizedLog::TokenUsage { total, limit } => {
            vec![AgentEvent::TokenUsageUpdated { total, limit }]
        }
        NormalizedLog::Error {
            error_type,
            message,
        } => vec![AgentEvent::ErrorOccurred {
            error: format!("{error_type}: {message}"),
        }],
    }
}

fn map_tool_call(
    name: String,
    args: Value,
    status: ToolStatus,
    action: ActionType,
) -> Vec<AgentEvent> {
    match status {
        ToolStatus::Started => vec![AgentEvent::ToolCallStarted { tool: name, args }],
        ToolStatus::Running => Vec::new(),
        ToolStatus::Completed => vec![AgentEvent::ToolCallCompleted {
            tool: name,
            result: completed_tool_result(args, action),
        }],
        ToolStatus::Failed => vec![AgentEvent::ToolCallFailed {
            tool: name,
            error: extract_tool_error(&args),
        }],
    }
}

fn completed_tool_result(args: Value, action: ActionType) -> Value {
    json!({
        "args": args,
        "action": action,
        "status": "completed",
    })
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

    use super::*;
    use crate::{
        log::{ActionType, NormalizedLog},
        types::{Role, ToolStatus},
    };

    #[test]
    fn maps_message_to_message_received() {
        let event = EventConverter::convert(NormalizedLog::Message {
            role: Role::Assistant,
            content: "hello".to_string(),
        });

        assert!(matches!(
            event,
            Some(AgentEvent::MessageReceived { role: Role::Assistant, content }) if content == "hello"
        ));
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

        assert!(matches!(
            started,
            Some(AgentEvent::ToolCallStarted { tool, args }) if tool == "read_file" && args == json!({ "path": "README.md" })
        ));
        assert!(matches!(
            completed,
            Some(AgentEvent::ToolCallCompleted { tool, result })
                if tool == "read_file"
                    && result == json!({
                        "args": { "content": "ok" },
                        "action": {
                            "action": "FileRead",
                            "path": "README.md"
                        },
                        "status": "completed"
                    })
        ));
        assert!(matches!(
            failed,
            Some(AgentEvent::ToolCallFailed { tool, error }) if tool == "read_file" && error == "permission denied"
        ));
    }

    #[test]
    fn ignores_tool_call_running_state_in_one_to_one_converter() {
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

    #[test]
    fn converts_message_log_for_stream_pipeline() {
        let events = from_normalized_log(NormalizedLog::Message {
            role: Role::Assistant,
            content: "done".to_string(),
        });
        assert!(matches!(
            events.as_slice(),
            [AgentEvent::MessageReceived { role: Role::Assistant, content }] if content == "done"
        ));
    }

    #[test]
    fn converts_tool_call_by_status_for_stream_pipeline() {
        let started = from_normalized_log(NormalizedLog::ToolCall {
            name: "bash".to_string(),
            args: json!({"cmd":"ls"}),
            status: ToolStatus::Started,
            action: ActionType::CommandRun {
                command: "ls".to_string(),
            },
        });
        assert!(matches!(
            started.as_slice(),
            [AgentEvent::ToolCallStarted { tool, .. }] if tool == "bash"
        ));

        let completed = from_normalized_log(NormalizedLog::ToolCall {
            name: "bash".to_string(),
            args: json!({"cmd":"ls"}),
            status: ToolStatus::Completed,
            action: ActionType::CommandRun {
                command: "ls".to_string(),
            },
        });
        assert!(matches!(
            completed.as_slice(),
            [AgentEvent::ToolCallCompleted { tool, .. }] if tool == "bash"
        ));

        let failed = from_normalized_log(NormalizedLog::ToolCall {
            name: "bash".to_string(),
            args: json!({"cmd":"ls"}),
            status: ToolStatus::Failed,
            action: ActionType::CommandRun {
                command: "ls".to_string(),
            },
        });
        assert!(matches!(
            failed.as_slice(),
            [AgentEvent::ToolCallFailed { tool, error }] if tool == "bash" && error == "tool call failed"
        ));

        let running = from_normalized_log(NormalizedLog::ToolCall {
            name: "bash".to_string(),
            args: json!({"cmd":"ls"}),
            status: ToolStatus::Running,
            action: ActionType::CommandRun {
                command: "ls".to_string(),
            },
        });
        assert!(running.is_empty());
    }
}
