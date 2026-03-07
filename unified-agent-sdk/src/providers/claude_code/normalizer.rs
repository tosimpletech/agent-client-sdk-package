//! Claude Code log normalizer.

use std::collections::HashMap;

use claude_code::{ContentBlock, Message, ToolResultBlock, UserContent, parse_message};
use serde_json::Value;

use crate::error::ExecutorError;
use crate::log::{ActionType, LogNormalizer, NormalizedLog};
use crate::types::{Role, ToolStatus};

#[derive(Debug, Clone)]
struct PendingToolCall {
    name: String,
    args: Value,
    action: ActionType,
}

/// Normalizes Claude Code JSON stream messages into `NormalizedLog` entries.
#[derive(Default)]
pub struct ClaudeCodeLogNormalizer {
    line_buffer: Vec<u8>,
    json_buffer: String,
    pending_tools: HashMap<String, PendingToolCall>,
}

impl ClaudeCodeLogNormalizer {
    /// Creates a new stateful normalizer for Claude Code JSON stream chunks.
    pub fn new() -> Self {
        Self::default()
    }

    fn process_line(&mut self, line: &[u8]) -> Result<Vec<NormalizedLog>, ExecutorError> {
        let line = std::str::from_utf8(line).map_err(|err| {
            ExecutorError::execution_failed("failed to decode claude log chunk as UTF-8", err)
        })?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }

        self.json_buffer.push_str(trimmed);

        match serde_json::from_str::<Value>(&self.json_buffer) {
            Ok(value) => {
                self.json_buffer.clear();
                self.process_value(value)
            }
            Err(err) if err.is_eof() => Ok(Vec::new()),
            Err(err) => {
                self.json_buffer.clear();
                Err(ExecutorError::Serialization(err))
            }
        }
    }

    fn process_value(&mut self, value: Value) -> Result<Vec<NormalizedLog>, ExecutorError> {
        let message = parse_message(&value).map_err(|err| {
            ExecutorError::execution_failed("failed to parse claude sdk message", err)
        })?;
        let Some(message) = message else {
            return Ok(Vec::new());
        };

        Ok(self.normalize_message(message))
    }

    fn normalize_message(&mut self, message: Message) -> Vec<NormalizedLog> {
        match message {
            Message::User(user) => self.normalize_user_content(user.content),
            Message::Assistant(assistant) => self.normalize_assistant_content(assistant.content),
            Message::System(system) => vec![NormalizedLog::Message {
                role: Role::System,
                content: system.subtype,
            }],
            Message::Result(result) => {
                let mut logs = Vec::new();

                if let Some((total, limit)) = extract_token_usage(result.usage.as_ref()) {
                    logs.push(NormalizedLog::TokenUsage { total, limit });
                }

                if result.is_error {
                    logs.push(NormalizedLog::Error {
                        error_type: "result_error".to_string(),
                        message: result.result.unwrap_or_else(|| {
                            format!("Claude Code result subtype: {}", result.subtype)
                        }),
                    });
                }

                logs
            }
            Message::StreamEvent(_) => Vec::new(),
        }
    }

    fn normalize_user_content(&mut self, content: UserContent) -> Vec<NormalizedLog> {
        match content {
            UserContent::Text(text) => {
                if text.trim().is_empty() {
                    Vec::new()
                } else {
                    vec![NormalizedLog::Message {
                        role: Role::User,
                        content: text,
                    }]
                }
            }
            UserContent::Blocks(blocks) => {
                let mut logs = Vec::new();
                for block in blocks {
                    match block {
                        ContentBlock::Text(text) => {
                            if !text.text.trim().is_empty() {
                                logs.push(NormalizedLog::Message {
                                    role: Role::User,
                                    content: text.text,
                                });
                            }
                        }
                        ContentBlock::ToolResult(result) => {
                            self.push_tool_result_log(&mut logs, result);
                        }
                        _ => {}
                    }
                }
                logs
            }
        }
    }

    fn normalize_assistant_content(&mut self, blocks: Vec<ContentBlock>) -> Vec<NormalizedLog> {
        let mut logs = Vec::new();

        for block in blocks {
            match block {
                ContentBlock::Text(text) => logs.push(NormalizedLog::Message {
                    role: Role::Assistant,
                    content: text.text,
                }),
                ContentBlock::Thinking(thinking) => logs.push(NormalizedLog::Thinking {
                    content: thinking.thinking,
                }),
                ContentBlock::ToolUse(tool_use) => {
                    let action = infer_action(&tool_use.name, &tool_use.input);
                    self.pending_tools.insert(
                        tool_use.id.clone(),
                        PendingToolCall {
                            name: tool_use.name.clone(),
                            args: tool_use.input.clone(),
                            action: action.clone(),
                        },
                    );

                    logs.push(NormalizedLog::ToolCall {
                        name: tool_use.name,
                        args: tool_use.input,
                        status: ToolStatus::Started,
                        action,
                    });
                }
                ContentBlock::ToolResult(result) => {
                    self.push_tool_result_log(&mut logs, result);
                }
            }
        }

        logs
    }

    fn push_tool_result_log(&mut self, logs: &mut Vec<NormalizedLog>, result: ToolResultBlock) {
        let ToolResultBlock {
            tool_use_id,
            content,
            is_error,
        } = result;
        if let Some(pending) = self.pending_tools.remove(&tool_use_id) {
            logs.push(NormalizedLog::ToolCall {
                name: pending.name,
                args: content.unwrap_or(pending.args),
                status: if is_error.unwrap_or(false) {
                    ToolStatus::Failed
                } else {
                    ToolStatus::Completed
                },
                action: pending.action,
            });
        }
    }

    fn error_to_log(error: ExecutorError) -> NormalizedLog {
        NormalizedLog::Error {
            error_type: error_type(&error).to_string(),
            message: error.to_string(),
        }
    }
}

impl LogNormalizer for ClaudeCodeLogNormalizer {
    fn normalize(&mut self, chunk: &[u8]) -> Vec<NormalizedLog> {
        let mut logs = Vec::new();

        for &byte in chunk {
            if byte == b'\n' {
                let line = std::mem::take(&mut self.line_buffer);
                match self.process_line(&line) {
                    Ok(mut parsed) => logs.append(&mut parsed),
                    Err(error) => logs.push(Self::error_to_log(error)),
                }
            } else {
                self.line_buffer.push(byte);
            }
        }

        logs
    }

    fn flush(&mut self) -> Vec<NormalizedLog> {
        let mut logs = Vec::new();

        if !self.line_buffer.is_empty() {
            let line = std::mem::take(&mut self.line_buffer);
            match self.process_line(&line) {
                Ok(mut parsed) => logs.append(&mut parsed),
                Err(error) => logs.push(Self::error_to_log(error)),
            }
        }

        if !self.json_buffer.trim().is_empty() {
            let buffer_len = self.json_buffer.len();
            let message = format!(
                "incomplete Claude Code JSON message buffered at flush: <redacted> (buffer_len={buffer_len})"
            );
            self.json_buffer.clear();
            logs.push(Self::error_to_log(ExecutorError::execution_failed(
                "failed to flush claude code log stream",
                message,
            )));
        }

        self.pending_tools.clear();

        logs
    }
}

fn infer_action(name: &str, args: &Value) -> ActionType {
    let lower = name.to_ascii_lowercase();

    if lower.starts_with("mcp__") {
        return ActionType::McpTool {
            tool: name.to_string(),
        };
    }

    if lower.contains("askuser") || lower.contains("ask_user") {
        return ActionType::AskUser;
    }

    if lower.contains("websearch") || lower.contains("web_search") || lower.contains("webfetch") {
        return ActionType::WebSearch {
            query: extract_first_string(args, &["query", "search_query", "url"])
                .unwrap_or_default(),
        };
    }

    if lower.contains("read") {
        return ActionType::FileRead {
            path: extract_first_string(args, &["file_path", "path", "target_file"])
                .unwrap_or_default(),
        };
    }

    if lower.contains("edit")
        || lower.contains("write")
        || lower.contains("patch")
        || lower.contains("multiedit")
    {
        return ActionType::FileEdit {
            path: extract_first_string(args, &["file_path", "path", "target_file"])
                .unwrap_or_default(),
        };
    }

    if lower.contains("bash") || lower.contains("command") || lower.contains("run") {
        return ActionType::CommandRun {
            command: extract_first_string(args, &["command", "cmd"]).unwrap_or_default(),
        };
    }

    ActionType::McpTool {
        tool: name.to_string(),
    }
}

fn extract_first_string(args: &Value, keys: &[&str]) -> Option<String> {
    let object = args.as_object()?;

    for key in keys {
        if let Some(value) = object.get(*key).and_then(Value::as_str) {
            return Some(value.to_string());
        }
    }

    None
}

fn extract_token_usage(usage: Option<&Value>) -> Option<(u32, u32)> {
    let usage = usage?;

    if let Some(total) = value_to_u64(Some(usage)) {
        let total = saturating_u64_to_u32(total);
        return Some((total, 0));
    }

    let object = usage.as_object()?;
    let total = value_to_u64(object.get("input_tokens"))
        .unwrap_or(0)
        .saturating_add(value_to_u64(object.get("output_tokens")).unwrap_or(0))
        .saturating_add(value_to_u64(object.get("cache_creation_input_tokens")).unwrap_or(0))
        .saturating_add(value_to_u64(object.get("cache_read_input_tokens")).unwrap_or(0));
    let limit = value_to_u64(object.get("limit"))
        .or_else(|| value_to_u64(object.get("max_tokens")))
        .unwrap_or(0);

    if total == 0 && limit == 0 {
        None
    } else {
        Some((saturating_u64_to_u32(total), saturating_u64_to_u32(limit)))
    }
}

fn value_to_u64(value: Option<&Value>) -> Option<u64> {
    match value {
        Some(Value::Number(number)) => number
            .as_u64()
            .or_else(|| number.as_i64().and_then(|v| u64::try_from(v).ok())),
        _ => None,
    }
}

fn saturating_u64_to_u32(value: u64) -> u32 {
    value.min(u64::from(u32::MAX)) as u32
}

fn error_type(error: &ExecutorError) -> &'static str {
    error.error_type()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_claude_stream_incrementally() {
        let mut normalizer = ClaudeCodeLogNormalizer::new();

        let assistant = concat!(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hello"},{"type":"thinking","thinking":"analyzing","signature":"sig"},{"type":"tool_use","id":"toolu_1","name":"Read","input":{"file_path":"src/main.rs"}}],"model":"claude-3-7-sonnet"}}"#,
            "\n"
        );
        let split = assistant.len() / 2;

        let first = normalizer.normalize(&assistant.as_bytes()[..split]);
        assert!(first.is_empty());

        let second = normalizer.normalize(&assistant.as_bytes()[split..]);
        assert_eq!(second.len(), 3);
        assert!(matches!(
            &second[0],
            NormalizedLog::Message {
                role: Role::Assistant,
                content
            } if content == "hello"
        ));
        assert!(matches!(
            &second[1],
            NormalizedLog::Thinking { content } if content == "analyzing"
        ));
        assert!(matches!(
            &second[2],
            NormalizedLog::ToolCall {
                name,
                status: ToolStatus::Started,
                ..
            } if name == "Read"
        ));

        let tool_result = concat!(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":{"ok":true},"is_error":false}]}}"#,
            "\n"
        );
        let third = normalizer.normalize(tool_result.as_bytes());
        assert_eq!(third.len(), 1);
        assert!(matches!(
            &third[0],
            NormalizedLog::ToolCall {
                name,
                args,
                status: ToolStatus::Completed,
                ..
            } if name == "Read" && args == &serde_json::json!({"ok": true})
        ));

        let result = concat!(
            r#"{"type":"result","subtype":"success","duration_ms":1,"duration_api_ms":1,"is_error":false,"num_turns":1,"session_id":"s1","usage":{"input_tokens":10,"output_tokens":5,"cache_creation_input_tokens":2,"cache_read_input_tokens":3}}"#,
            "\n"
        );
        let fourth = normalizer.normalize(result.as_bytes());
        assert_eq!(fourth.len(), 1);
        assert!(matches!(
            &fourth[0],
            NormalizedLog::TokenUsage { total, limit } if *total == 20 && *limit == 0
        ));
    }

    #[test]
    fn extracts_limit_when_explicitly_present() {
        let usage = serde_json::json!({
            "input_tokens": 4,
            "output_tokens": 6,
            "limit": 100
        });

        let parsed = extract_token_usage(Some(&usage));
        assert_eq!(parsed, Some((10, 100)));
    }

    #[test]
    fn numeric_usage_keeps_unknown_limit() {
        let usage = serde_json::json!(42);
        let parsed = extract_token_usage(Some(&usage));
        assert_eq!(parsed, Some((42, 0)));
    }

    #[test]
    fn flush_emits_error_for_incomplete_json() {
        let mut normalizer = ClaudeCodeLogNormalizer::new();

        let partial = br#"{"type":"assistant","message":{"content":[{"type":"text","text":"hello"}],"model":"claude"}"#;
        let logs = normalizer.normalize(partial);
        assert!(logs.is_empty());

        let flushed = normalizer.flush();
        assert_eq!(flushed.len(), 1);
        assert!(matches!(
            &flushed[0],
            NormalizedLog::Error { error_type, .. } if error_type == "execution_failed"
        ));
    }

    #[test]
    fn invalid_utf8_is_reported_as_error_log() {
        let mut normalizer = ClaudeCodeLogNormalizer::new();
        let logs = normalizer.normalize(&[0xFF, b'\n']);

        assert_eq!(logs.len(), 1);
        assert!(matches!(
            &logs[0],
            NormalizedLog::Error { error_type, .. } if error_type == "execution_failed"
        ));
    }

    #[test]
    fn flush_clears_pending_tool_calls() {
        let mut normalizer = ClaudeCodeLogNormalizer::new();

        let tool_start = concat!(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"Read","input":{"file_path":"src/main.rs"}}],"model":"claude-3-7-sonnet"}}"#,
            "\n"
        );
        let started = normalizer.normalize(tool_start.as_bytes());
        assert!(matches!(
            started.as_slice(),
            [NormalizedLog::ToolCall {
                status: ToolStatus::Started,
                ..
            }]
        ));

        let _ = normalizer.flush();

        let tool_result = concat!(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":{"ok":true},"is_error":false}]}}"#,
            "\n"
        );
        let logs_after_flush = normalizer.normalize(tool_result.as_bytes());
        assert!(logs_after_flush.is_empty());
    }

    #[test]
    fn ignores_whitespace_only_user_text_blocks() {
        let mut normalizer = ClaudeCodeLogNormalizer::new();

        let user_message = concat!(
            r#"{"type":"user","message":{"content":[{"type":"text","text":"   "},{"type":"text","text":"hello"}]}}"#,
            "\n"
        );
        let logs = normalizer.normalize(user_message.as_bytes());

        assert_eq!(logs.len(), 1);
        assert!(matches!(
            &logs[0],
            NormalizedLog::Message {
                role: Role::User,
                content
            } if content == "hello"
        ));
    }
}
