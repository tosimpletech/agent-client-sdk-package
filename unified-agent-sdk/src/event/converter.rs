//! Stateless conversion from normalized logs to agent events.

use serde_json::{Value, json};

use crate::{
    event::AgentEvent,
    log::{ActionType, NormalizedLog},
    types::{ContextUsage, ContextUsageSource, ToolStatus},
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
            NormalizedLog::Error {
                error_type,
                message,
            } => Some(AgentEvent::ErrorOccurred {
                error: format!("{error_type}: {message}"),
            }),
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
    from_normalized_log_with_context_override(log, None)
}

/// Convert one normalized log entry into one or more [`AgentEvent`] values, with
/// an optional context window override used when source logs do not provide a limit.
pub fn from_normalized_log_with_context_override(
    log: NormalizedLog,
    context_window_override_tokens: Option<u32>,
) -> Vec<AgentEvent> {
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
        NormalizedLog::TokenUsage { total, limit } => vec![
            AgentEvent::TokenUsageUpdated { total, limit },
            AgentEvent::ContextUsageUpdated {
                usage: context_usage_from_token_usage(total, limit, context_window_override_tokens),
            },
        ],
        NormalizedLog::Error {
            error_type,
            message,
        } => vec![AgentEvent::ErrorOccurred {
            error: format!("{error_type}: {message}"),
        }],
    }
}

fn context_usage_from_token_usage(
    total: u32,
    limit: u32,
    context_window_override_tokens: Option<u32>,
) -> ContextUsage {
    let (window_tokens, source) = if limit > 0 {
        (Some(limit), ContextUsageSource::ProviderReported)
    } else if let Some(override_tokens) = context_window_override_tokens {
        (Some(override_tokens), ContextUsageSource::ConfigOverride)
    } else {
        (None, ContextUsageSource::Unknown)
    };

    let remaining_tokens = window_tokens.map(|window| window.saturating_sub(total));
    let utilization = window_tokens.and_then(|window| {
        if window == 0 {
            None
        } else {
            Some((total as f32 / window as f32).clamp(0.0, 1.0))
        }
    });

    ContextUsage {
        used_tokens: total,
        window_tokens,
        remaining_tokens,
        utilization,
        source,
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
    if let Value::String(message) = args {
        let trimmed = message.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    args.get("error")
        .or_else(|| args.get("message"))
        .and_then(nonempty_message)
        .unwrap_or_else(|| "tool call failed".to_string())
}

fn nonempty_message(value: &Value) -> Option<String> {
    let message = match value {
        Value::String(message) => message.clone(),
        Value::Null => return None,
        other => other.to_string(),
    };
    let trimmed = message.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return None;
    }
    Some(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        log::{ActionType, NormalizedLog},
        types::{ContextUsageSource, Role, ToolStatus},
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
    fn preserves_string_payloads_for_failed_tool_calls() {
        let one_to_one = EventConverter::convert(NormalizedLog::ToolCall {
            name: "bash".to_string(),
            args: json!("permission denied"),
            status: ToolStatus::Failed,
            action: ActionType::CommandRun {
                command: "ls".to_string(),
            },
        });
        let fan_out = from_normalized_log(NormalizedLog::ToolCall {
            name: "bash".to_string(),
            args: json!("permission denied"),
            status: ToolStatus::Failed,
            action: ActionType::CommandRun {
                command: "ls".to_string(),
            },
        });

        assert!(matches!(
            one_to_one,
            Some(AgentEvent::ToolCallFailed { tool, error }) if tool == "bash" && error == "permission denied"
        ));
        assert!(matches!(
            fan_out.as_slice(),
            [AgentEvent::ToolCallFailed { tool, error }] if tool == "bash" && error == "permission denied"
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

    #[test]
    fn maps_error_consistently_across_conversion_helpers() {
        let one_to_one = EventConverter::convert(NormalizedLog::Error {
            error_type: "io".to_string(),
            message: "permission denied".to_string(),
        });
        let fan_out = from_normalized_log(NormalizedLog::Error {
            error_type: "io".to_string(),
            message: "permission denied".to_string(),
        });

        assert!(matches!(
            one_to_one,
            Some(AgentEvent::ErrorOccurred { error }) if error == "io: permission denied"
        ));
        assert!(matches!(
            fan_out.as_slice(),
            [AgentEvent::ErrorOccurred { error }] if error == "io: permission denied"
        ));
    }

    #[test]
    fn token_usage_fans_out_context_usage_with_provider_limit() {
        let events = from_normalized_log(NormalizedLog::TokenUsage {
            total: 40,
            limit: 100,
        });

        assert!(events
            .iter()
            .any(|event| matches!(event, AgentEvent::TokenUsageUpdated { total, limit } if *total == 40 && *limit == 100)));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::ContextUsageUpdated { usage }
                if usage.used_tokens == 40
                    && usage.window_tokens == Some(100)
                    && usage.remaining_tokens == Some(60)
                    && usage.source == ContextUsageSource::ProviderReported
        )));
    }

    #[test]
    fn token_usage_uses_context_window_override_when_limit_unknown() {
        let events = from_normalized_log_with_context_override(
            NormalizedLog::TokenUsage {
                total: 30,
                limit: 0,
            },
            Some(120),
        );

        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::ContextUsageUpdated { usage }
                if usage.used_tokens == 30
                    && usage.window_tokens == Some(120)
                    && usage.remaining_tokens == Some(90)
                    && usage.source == ContextUsageSource::ConfigOverride
        )));
    }
}
