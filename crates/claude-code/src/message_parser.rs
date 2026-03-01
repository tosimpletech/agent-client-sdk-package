use serde_json::Value;

use crate::errors::MessageParseError;
use crate::types::{
    AssistantMessage, ContentBlock, Message, ResultMessage, StreamEvent, SystemMessage, TextBlock,
    ThinkingBlock, ToolResultBlock, ToolUseBlock, UserContent, UserMessage,
};

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
                uuid: obj.get("uuid").and_then(Value::as_str).map(ToString::to_string),
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
                total_cost_usd: obj.get("total_cost_usd").and_then(Value::as_f64),
                usage: obj.get("usage").cloned(),
                result: obj
                    .get("result")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                structured_output: obj.get("structured_output").cloned(),
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
        _ => Ok(None),
    }
}

