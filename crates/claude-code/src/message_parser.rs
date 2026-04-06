//! JSON message parser for the Claude Code CLI protocol.
//!
//! This module converts raw JSON values from the CLI output stream into
//! typed [`Message`] variants. It handles all message types: user, assistant,
//! system, result, and stream_event.
//!
//! The primary entry point is [`parse_message()`].

use serde_json::Value;

use crate::errors::MessageParseError;
use crate::types::{
    AssistantMessage, ContentBlock, Message, RateLimitEvent, RateLimitInfo, ResultMessage,
    StreamEvent, SystemMessage, TextBlock, ThinkingBlock, ToolResultBlock, ToolUseBlock,
    UserContent, UserMessage,
};

/// Returns a human-readable type name for a JSON value (for error messages).
fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

/// Extracts a required field from a JSON object, returning a descriptive error if missing.
fn get_required<'a>(
    data: &'a serde_json::Map<String, Value>,
    key: &str,
    context: &str,
    full_data: &Value,
) -> std::result::Result<&'a Value, MessageParseError> {
    data.get(key).ok_or_else(|| {
        MessageParseError::new(
            format!("Missing required field in {context} message: '{key}'"),
            Some(full_data.clone()),
        )
    })
}

/// Parses an array of JSON content blocks into typed [`ContentBlock`] variants.
///
/// Supports `text`, `thinking`, `tool_use`, and `tool_result` block types.
/// Unknown block types are silently skipped.
fn parse_content_blocks(blocks: &[Value], include_thinking: bool) -> Vec<ContentBlock> {
    let mut content_blocks = Vec::new();

    for block in blocks {
        let Some(block_obj) = block.as_object() else {
            continue;
        };

        let Some(block_type) = block_obj.get("type").and_then(Value::as_str) else {
            continue;
        };

        match block_type {
            "text" => {
                if let Some(text) = block_obj.get("text").and_then(Value::as_str) {
                    content_blocks.push(ContentBlock::Text(TextBlock {
                        text: text.to_string(),
                    }));
                }
            }
            "thinking" if include_thinking => {
                if let (Some(thinking), Some(signature)) = (
                    block_obj.get("thinking").and_then(Value::as_str),
                    block_obj.get("signature").and_then(Value::as_str),
                ) {
                    content_blocks.push(ContentBlock::Thinking(ThinkingBlock {
                        thinking: thinking.to_string(),
                        signature: signature.to_string(),
                    }));
                }
            }
            "tool_use" => {
                if let (Some(id), Some(name), Some(input)) = (
                    block_obj.get("id").and_then(Value::as_str),
                    block_obj.get("name").and_then(Value::as_str),
                    block_obj.get("input"),
                ) {
                    content_blocks.push(ContentBlock::ToolUse(ToolUseBlock {
                        id: id.to_string(),
                        name: name.to_string(),
                        input: input.clone(),
                    }));
                }
            }
            "tool_result" => {
                if let Some(tool_use_id) = block_obj.get("tool_use_id").and_then(Value::as_str) {
                    content_blocks.push(ContentBlock::ToolResult(ToolResultBlock {
                        tool_use_id: tool_use_id.to_string(),
                        content: block_obj.get("content").cloned(),
                        is_error: block_obj.get("is_error").and_then(Value::as_bool),
                    }));
                }
            }
            _ => {}
        }
    }

    content_blocks
}

/// Parses a raw JSON value from the CLI into a typed [`Message`].
///
/// # Arguments
///
/// * `data` — A JSON value representing a single message from the CLI output stream.
///
/// # Returns
///
/// - `Ok(Some(message))` — Successfully parsed into a known message type.
/// - `Ok(None)` — The message type is unrecognized (silently skipped).
/// - `Err(MessageParseError)` — The message is malformed or missing required fields.
///
/// # Supported message types
///
/// | `type` field | Parsed into |
/// |-------------|-------------|
/// | `"user"` | [`Message::User`] |
/// | `"assistant"` | [`Message::Assistant`] |
/// | `"system"` | [`Message::System`] |
/// | `"result"` | [`Message::Result`] |
/// | `"stream_event"` | [`Message::StreamEvent`] |
///
/// # Example
///
/// ```rust
/// use claude_code::{parse_message, Message};
/// use serde_json::json;
///
/// let raw = json!({
///     "type": "system",
///     "subtype": "initialized"
/// });
///
/// let parsed = parse_message(&raw).unwrap();
/// assert!(matches!(parsed, Some(Message::System(_))));
/// ```
pub fn parse_message(data: &Value) -> std::result::Result<Option<Message>, MessageParseError> {
    let Some(obj) = data.as_object() else {
        return Err(MessageParseError::new(
            format!(
                "Invalid message data type (expected dict, got {})",
                value_type_name(data)
            ),
            Some(data.clone()),
        ));
    };

    let Some(message_type) = obj.get("type").and_then(Value::as_str) else {
        return Err(MessageParseError::new(
            "Message missing 'type' field",
            Some(data.clone()),
        ));
    };

    match message_type {
        "user" => {
            let message = get_required(obj, "message", "user", data)?;
            let message_obj = message.as_object().ok_or_else(|| {
                MessageParseError::new(
                    "Missing required field in user message: 'message'",
                    Some(data.clone()),
                )
            })?;
            let content = get_required(message_obj, "content", "user", data)?;

            let user_content = if let Some(content_blocks) = content.as_array() {
                UserContent::Blocks(parse_content_blocks(content_blocks, false))
            } else if let Some(content_text) = content.as_str() {
                UserContent::Text(content_text.to_string())
            } else {
                UserContent::Text(content.to_string())
            };

            Ok(Some(Message::User(UserMessage {
                content: user_content,
                uuid: obj
                    .get("uuid")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                parent_tool_use_id: obj
                    .get("parent_tool_use_id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                tool_use_result: obj.get("tool_use_result").cloned(),
            })))
        }
        "assistant" => {
            let message = get_required(obj, "message", "assistant", data)?;
            let message_obj = message.as_object().ok_or_else(|| {
                MessageParseError::new(
                    "Missing required field in assistant message: 'message'",
                    Some(data.clone()),
                )
            })?;

            let content = get_required(message_obj, "content", "assistant", data)?;
            let model = get_required(message_obj, "model", "assistant", data)?
                .as_str()
                .ok_or_else(|| {
                    MessageParseError::new(
                        "Missing required field in assistant message: 'model'",
                        Some(data.clone()),
                    )
                })?;

            let blocks = content.as_array().ok_or_else(|| {
                MessageParseError::new(
                    "Missing required field in assistant message: 'content'",
                    Some(data.clone()),
                )
            })?;

            Ok(Some(Message::Assistant(AssistantMessage {
                content: parse_content_blocks(blocks, true),
                model: model.to_string(),
                parent_tool_use_id: obj
                    .get("parent_tool_use_id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                error: obj
                    .get("error")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                usage: message_obj.get("usage").cloned(),
                message_id: message_obj
                    .get("id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                stop_reason: message_obj
                    .get("stop_reason")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                session_id: obj
                    .get("session_id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                uuid: obj
                    .get("uuid")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
            })))
        }
        "system" => {
            let subtype = get_required(obj, "subtype", "system", data)?
                .as_str()
                .ok_or_else(|| {
                    MessageParseError::new(
                        "Missing required field in system message: 'subtype'",
                        Some(data.clone()),
                    )
                })?;

            Ok(Some(Message::System(SystemMessage {
                subtype: subtype.to_string(),
                data: data.clone(),
            })))
        }
        "result" => {
            let subtype = get_required(obj, "subtype", "result", data)?
                .as_str()
                .ok_or_else(|| {
                    MessageParseError::new(
                        "Missing required field in result message: 'subtype'",
                        Some(data.clone()),
                    )
                })?;
            let duration_ms = get_required(obj, "duration_ms", "result", data)?
                .as_i64()
                .ok_or_else(|| {
                    MessageParseError::new(
                        "Missing required field in result message: 'duration_ms'",
                        Some(data.clone()),
                    )
                })?;
            let duration_api_ms = get_required(obj, "duration_api_ms", "result", data)?
                .as_i64()
                .ok_or_else(|| {
                    MessageParseError::new(
                        "Missing required field in result message: 'duration_api_ms'",
                        Some(data.clone()),
                    )
                })?;
            let is_error = get_required(obj, "is_error", "result", data)?
                .as_bool()
                .ok_or_else(|| {
                    MessageParseError::new(
                        "Missing required field in result message: 'is_error'",
                        Some(data.clone()),
                    )
                })?;
            let num_turns = get_required(obj, "num_turns", "result", data)?
                .as_i64()
                .ok_or_else(|| {
                    MessageParseError::new(
                        "Missing required field in result message: 'num_turns'",
                        Some(data.clone()),
                    )
                })?;
            let session_id = get_required(obj, "session_id", "result", data)?
                .as_str()
                .ok_or_else(|| {
                    MessageParseError::new(
                        "Missing required field in result message: 'session_id'",
                        Some(data.clone()),
                    )
                })?;

            Ok(Some(Message::Result(ResultMessage {
                subtype: subtype.to_string(),
                duration_ms,
                duration_api_ms,
                is_error,
                num_turns,
                session_id: session_id.to_string(),
                stop_reason: obj
                    .get("stop_reason")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                total_cost_usd: obj.get("total_cost_usd").and_then(Value::as_f64),
                usage: obj.get("usage").cloned(),
                result: obj
                    .get("result")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                structured_output: obj.get("structured_output").cloned(),
                model_usage: obj.get("modelUsage").cloned(),
                permission_denials: obj
                    .get("permission_denials")
                    .and_then(Value::as_array)
                    .cloned(),
                errors: obj.get("errors").and_then(Value::as_array).map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(ToString::to_string))
                        .collect()
                }),
                uuid: obj
                    .get("uuid")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
            })))
        }
        "stream_event" => {
            let uuid = get_required(obj, "uuid", "stream_event", data)?
                .as_str()
                .ok_or_else(|| {
                    MessageParseError::new(
                        "Missing required field in stream_event message: 'uuid'",
                        Some(data.clone()),
                    )
                })?;
            let session_id = get_required(obj, "session_id", "stream_event", data)?
                .as_str()
                .ok_or_else(|| {
                    MessageParseError::new(
                        "Missing required field in stream_event message: 'session_id'",
                        Some(data.clone()),
                    )
                })?;
            let event = get_required(obj, "event", "stream_event", data)?;

            Ok(Some(Message::StreamEvent(StreamEvent {
                uuid: uuid.to_string(),
                session_id: session_id.to_string(),
                event: event.clone(),
                parent_tool_use_id: obj
                    .get("parent_tool_use_id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
            })))
        }
        "rate_limit_event" => {
            let info = get_required(obj, "rate_limit_info", "rate_limit_event", data)?
                .as_object()
                .ok_or_else(|| {
                    MessageParseError::new(
                        "Missing required field in rate_limit_event message: 'rate_limit_info'",
                        Some(data.clone()),
                    )
                })?;
            let uuid = get_required(obj, "uuid", "rate_limit_event", data)?
                .as_str()
                .ok_or_else(|| {
                    MessageParseError::new(
                        "Missing required field in rate_limit_event message: 'uuid'",
                        Some(data.clone()),
                    )
                })?;
            let session_id = get_required(obj, "session_id", "rate_limit_event", data)?
                .as_str()
                .ok_or_else(|| {
                    MessageParseError::new(
                        "Missing required field in rate_limit_event message: 'session_id'",
                        Some(data.clone()),
                    )
                })?;

            Ok(Some(Message::RateLimit(RateLimitEvent {
                rate_limit_info: RateLimitInfo {
                    status: serde_json::from_value(
                        info.get("status")
                            .cloned()
                            .unwrap_or(Value::String("allowed".to_string())),
                    )
                    .map_err(|err| {
                        MessageParseError::new(
                            format!("Invalid rate_limit_event status: {err}"),
                            Some(data.clone()),
                        )
                    })?,
                    resets_at: info.get("resetsAt").and_then(Value::as_i64),
                    rate_limit_type: info
                        .get("rateLimitType")
                        .cloned()
                        .map(serde_json::from_value)
                        .transpose()
                        .map_err(|err| {
                            MessageParseError::new(
                                format!("Invalid rate_limit_event rateLimitType: {err}"),
                                Some(data.clone()),
                            )
                        })?,
                    utilization: info.get("utilization").and_then(Value::as_f64),
                    overage_status: info
                        .get("overageStatus")
                        .cloned()
                        .map(serde_json::from_value)
                        .transpose()
                        .map_err(|err| {
                            MessageParseError::new(
                                format!("Invalid rate_limit_event overageStatus: {err}"),
                                Some(data.clone()),
                            )
                        })?,
                    overage_resets_at: info.get("overageResetsAt").and_then(Value::as_i64),
                    overage_disabled_reason: info
                        .get("overageDisabledReason")
                        .and_then(Value::as_str)
                        .map(ToString::to_string),
                    raw: Value::Object(info.clone()),
                },
                uuid: uuid.to_string(),
                session_id: session_id.to_string(),
            })))
        }
        _ => Ok(None),
    }
}
