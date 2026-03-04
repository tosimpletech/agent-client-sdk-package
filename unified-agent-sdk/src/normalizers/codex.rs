//! Codex JSONL log normalizer.

use codex::{CommandExecutionStatus, McpToolCallStatus, PatchApplyStatus, ThreadEvent, ThreadItem};
use serde_json::json;

use crate::{
    error::{ExecutorError, Result},
    log::{ActionType, LogNormalizer, NormalizedLog},
    types::{Role, ToolStatus},
};

/// Normalizes Codex `ThreadEvent` JSONL chunks to [`NormalizedLog`] entries.
#[derive(Debug, Default)]
pub struct CodexLogNormalizer {
    buffer: Vec<u8>,
}

impl CodexLogNormalizer {
    /// Creates a new Codex log normalizer.
    pub fn new() -> Self {
        Self::default()
    }

    fn consume_lines(&mut self) -> Vec<NormalizedLog> {
        let mut output = Vec::new();

        while let Some(newline_idx) = self.buffer.iter().position(|&byte| byte == b'\n') {
            let mut line = self.buffer.drain(..=newline_idx).collect::<Vec<_>>();
            if matches!(line.last(), Some(b'\n')) {
                line.pop();
            }
            if matches!(line.last(), Some(b'\r')) {
                line.pop();
            }

            if line.is_empty() {
                continue;
            }

            output.extend(self.normalize_line(&line));
        }

        output
    }

    fn normalize_line(&self, line: &[u8]) -> Vec<NormalizedLog> {
        match self.try_normalize_line(line) {
            Ok(logs) => logs,
            Err(error) => vec![Self::error_from_executor_error(error)],
        }
    }

    fn try_normalize_line(&self, line: &[u8]) -> Result<Vec<NormalizedLog>> {
        let event: ThreadEvent = serde_json::from_slice(line)?;
        Ok(Self::map_event(event))
    }

    fn map_event(event: ThreadEvent) -> Vec<NormalizedLog> {
        match event {
            ThreadEvent::ThreadStarted { .. } | ThreadEvent::TurnStarted => Vec::new(),
            ThreadEvent::TurnCompleted { usage } => {
                let total_u64 = usage
                    .input_tokens
                    .saturating_add(usage.cached_input_tokens)
                    .saturating_add(usage.output_tokens);

                vec![NormalizedLog::TokenUsage {
                    total: total_u64.min(u32::MAX as u64) as u32,
                    // Codex events do not expose a token limit.
                    limit: 0,
                }]
            }
            ThreadEvent::TurnFailed { error } => vec![NormalizedLog::Error {
                error_type: "turn_failed".to_string(),
                message: error.message,
            }],
            ThreadEvent::Error { message } => vec![NormalizedLog::Error {
                error_type: "stream_error".to_string(),
                message,
            }],
            ThreadEvent::ItemStarted { item } => Self::map_item(item, ItemPhase::Started),
            ThreadEvent::ItemUpdated { item } => Self::map_item(item, ItemPhase::Updated),
            ThreadEvent::ItemCompleted { item } => Self::map_item(item, ItemPhase::Completed),
        }
    }

    fn map_item(item: ThreadItem, phase: ItemPhase) -> Vec<NormalizedLog> {
        match item {
            ThreadItem::AgentMessage(message) => vec![NormalizedLog::Message {
                role: Role::Assistant,
                content: message.text,
            }],
            ThreadItem::CommandExecution(command) => vec![NormalizedLog::ToolCall {
                name: "command_execution".to_string(),
                args: json!({
                    "id": command.id,
                    "output": command.aggregated_output,
                    "exit_code": command.exit_code,
                }),
                status: Self::map_command_status(command.status, phase),
                action: ActionType::CommandRun {
                    command: command.command,
                },
            }],
            ThreadItem::FileChange(file_change) => {
                let status = Self::map_patch_status(file_change.status, phase);
                let changes_len = file_change.changes.len();

                file_change
                    .changes
                    .into_iter()
                    .map(|change| NormalizedLog::ToolCall {
                        name: "file_change".to_string(),
                        args: json!({
                            "id": file_change.id,
                            "kind": change.kind,
                            "status": file_change.status,
                            "change_count": changes_len,
                        }),
                        status,
                        action: ActionType::FileEdit { path: change.path },
                    })
                    .collect()
            }
            ThreadItem::McpToolCall(tool_call) => {
                let tool_name = format!("{}.{}", tool_call.server, tool_call.tool);
                vec![NormalizedLog::ToolCall {
                    name: tool_name,
                    args: json!({
                        "id": tool_call.id,
                        "server": tool_call.server,
                        "arguments": tool_call.arguments,
                        "result": tool_call.result,
                        "error": tool_call.error,
                    }),
                    status: Self::map_mcp_status(tool_call.status, phase),
                    action: ActionType::McpTool {
                        tool: tool_call.tool,
                    },
                }]
            }
            ThreadItem::Reasoning(reasoning) => vec![NormalizedLog::Thinking {
                content: reasoning.text,
            }],
            ThreadItem::WebSearch(search) => vec![NormalizedLog::ToolCall {
                name: "web_search".to_string(),
                args: json!({ "id": search.id }),
                status: Self::status_from_phase(phase, ToolStatus::Completed),
                action: ActionType::WebSearch {
                    query: search.query,
                },
            }],
            ThreadItem::Error(error_item) => vec![NormalizedLog::Error {
                error_type: "item_error".to_string(),
                message: error_item.message,
            }],
            ThreadItem::TodoList(_) => Vec::new(),
        }
    }

    fn map_command_status(status: CommandExecutionStatus, phase: ItemPhase) -> ToolStatus {
        match phase {
            ItemPhase::Started => ToolStatus::Started,
            ItemPhase::Updated | ItemPhase::Completed => match status {
                CommandExecutionStatus::InProgress => ToolStatus::Running,
                CommandExecutionStatus::Completed => ToolStatus::Completed,
                CommandExecutionStatus::Failed => ToolStatus::Failed,
            },
        }
    }

    fn map_patch_status(status: PatchApplyStatus, phase: ItemPhase) -> ToolStatus {
        match phase {
            ItemPhase::Started => ToolStatus::Started,
            ItemPhase::Updated | ItemPhase::Completed => match status {
                PatchApplyStatus::Completed => ToolStatus::Completed,
                PatchApplyStatus::Failed => ToolStatus::Failed,
            },
        }
    }

    fn map_mcp_status(status: McpToolCallStatus, phase: ItemPhase) -> ToolStatus {
        match phase {
            ItemPhase::Started => ToolStatus::Started,
            ItemPhase::Updated | ItemPhase::Completed => match status {
                McpToolCallStatus::InProgress => ToolStatus::Running,
                McpToolCallStatus::Completed => ToolStatus::Completed,
                McpToolCallStatus::Failed => ToolStatus::Failed,
            },
        }
    }

    fn status_from_phase(phase: ItemPhase, fallback: ToolStatus) -> ToolStatus {
        match phase {
            ItemPhase::Started => ToolStatus::Started,
            ItemPhase::Updated | ItemPhase::Completed => fallback,
        }
    }

    fn error_from_executor_error(error: ExecutorError) -> NormalizedLog {
        let error_type = error.error_type();

        NormalizedLog::Error {
            error_type: error_type.to_string(),
            message: error.to_string(),
        }
    }
}

impl LogNormalizer for CodexLogNormalizer {
    fn normalize(&mut self, chunk: &[u8]) -> Vec<NormalizedLog> {
        self.buffer.extend_from_slice(chunk);
        self.consume_lines()
    }

    fn flush(&mut self) -> Vec<NormalizedLog> {
        let remaining = std::mem::take(&mut self.buffer);
        if remaining.is_empty() || remaining.iter().all(u8::is_ascii_whitespace) {
            return Vec::new();
        }

        self.normalize_line(&remaining)
    }
}

#[derive(Debug, Clone, Copy)]
enum ItemPhase {
    Started,
    Updated,
    Completed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_is_incremental() {
        let mut normalizer = CodexLogNormalizer::new();
        let line =
            r#"{"type":"item.completed","item":{"type":"agent_message","id":"a1","text":"done"}}"#;

        assert!(normalizer.normalize(line.as_bytes()).is_empty());

        let logs = normalizer.normalize(b"\n");
        assert_eq!(logs.len(), 1);
        match &logs[0] {
            NormalizedLog::Message { role, content } => {
                assert_eq!(*role, Role::Assistant);
                assert_eq!(content, "done");
            }
            other => panic!("unexpected log: {other:?}"),
        }
    }

    #[test]
    fn maps_required_codex_items() {
        let mut normalizer = CodexLogNormalizer::new();
        let jsonl = concat!(
            r#"{"type":"item.completed","item":{"type":"agent_message","id":"m1","text":"hello"}}"#,
            "\n",
            r#"{"type":"item.updated","item":{"type":"command_execution","id":"c1","command":"ls -la","aggregated_output":"ok","status":"in_progress"}}"#,
            "\n",
            r#"{"type":"item.completed","item":{"type":"file_change","id":"f1","changes":[{"path":"src/lib.rs","kind":"update"}],"status":"completed"}}"#,
            "\n",
            r#"{"type":"item.completed","item":{"type":"mcp_tool_call","id":"t1","server":"filesystem","tool":"read_file","arguments":{"path":"README.md"},"status":"completed"}}"#,
            "\n",
            r#"{"type":"item.updated","item":{"type":"reasoning","id":"r1","text":"analyzing..."}}"#,
            "\n"
        );

        let logs = normalizer.normalize(jsonl.as_bytes());
        assert_eq!(logs.len(), 5);

        match &logs[0] {
            NormalizedLog::Message { role, content } => {
                assert_eq!(*role, Role::Assistant);
                assert_eq!(content, "hello");
            }
            other => panic!("unexpected message mapping: {other:?}"),
        }

        match &logs[1] {
            NormalizedLog::ToolCall { action, status, .. } => {
                assert!(matches!(
                    action,
                    ActionType::CommandRun { command } if command == "ls -la"
                ));
                assert_eq!(*status, ToolStatus::Running);
            }
            other => panic!("unexpected command mapping: {other:?}"),
        }

        match &logs[2] {
            NormalizedLog::ToolCall { action, status, .. } => {
                assert!(matches!(
                    action,
                    ActionType::FileEdit { path } if path == "src/lib.rs"
                ));
                assert_eq!(*status, ToolStatus::Completed);
            }
            other => panic!("unexpected file change mapping: {other:?}"),
        }

        match &logs[3] {
            NormalizedLog::ToolCall { action, status, .. } => {
                assert!(matches!(
                    action,
                    ActionType::McpTool { tool } if tool == "read_file"
                ));
                assert_eq!(*status, ToolStatus::Completed);
            }
            other => panic!("unexpected mcp mapping: {other:?}"),
        }

        match &logs[4] {
            NormalizedLog::Thinking { content } => assert_eq!(content, "analyzing..."),
            other => panic!("unexpected thinking mapping: {other:?}"),
        }
    }

    #[test]
    fn flush_processes_trailing_data() {
        let mut normalizer = CodexLogNormalizer::new();
        let line =
            r#"{"type":"item.completed","item":{"type":"reasoning","id":"r1","text":"pending"}}"#;

        assert!(normalizer.normalize(line.as_bytes()).is_empty());

        let logs = normalizer.flush();
        assert_eq!(logs.len(), 1);
        match &logs[0] {
            NormalizedLog::Thinking { content } => assert_eq!(content, "pending"),
            other => panic!("unexpected flush mapping: {other:?}"),
        }
    }
}
