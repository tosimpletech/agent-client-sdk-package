use claude_code::{
    AssistantMessage, ClaudeAgentOptions, PermissionMode, ResultMessage, TextBlock, ThinkingBlock,
    ToolResultBlock, ToolUseBlock, UserContent, UserMessage,
};
use serde_json::json;

#[test]
fn test_user_message_creation() {
    let msg = UserMessage {
        content: UserContent::Text("Hello, Claude!".to_string()),
        uuid: None,
        parent_tool_use_id: None,
        tool_use_result: None,
    };
    assert_eq!(msg.content, UserContent::Text("Hello, Claude!".to_string()));
}

#[test]
fn test_assistant_message_with_text() {
    let text_block = TextBlock {
        text: "Hello, human!".to_string(),
    };
    let msg = AssistantMessage {
        content: vec![claude_code::ContentBlock::Text(text_block)],
        model: "claude-opus-4-1-20250805".to_string(),
        parent_tool_use_id: None,
        error: None,
    };
    assert_eq!(msg.content.len(), 1);
}

#[test]
fn test_assistant_message_with_thinking() {
    let thinking_block = ThinkingBlock {
        thinking: "I'm thinking...".to_string(),
        signature: "sig-123".to_string(),
    };
    let msg = AssistantMessage {
        content: vec![claude_code::ContentBlock::Thinking(
            thinking_block.clone(),
        )],
        model: "claude-opus-4-1-20250805".to_string(),
        parent_tool_use_id: None,
        error: None,
    };
    assert_eq!(msg.content.len(), 1);
    match &msg.content[0] {
        claude_code::ContentBlock::Thinking(block) => {
            assert_eq!(block.thinking, thinking_block.thinking);
            assert_eq!(block.signature, thinking_block.signature);
        }
        _ => panic!("expected thinking block"),
    }
}

#[test]
fn test_tool_use_and_result_block() {
    let block = ToolUseBlock {
        id: "tool-123".to_string(),
        name: "Read".to_string(),
        input: json!({"file_path": "/test.txt"}),
    };
    assert_eq!(block.id, "tool-123");
    assert_eq!(block.name, "Read");
    assert_eq!(block.input["file_path"], "/test.txt");

    let result_block = ToolResultBlock {
        tool_use_id: "tool-123".to_string(),
        content: Some(json!("File contents here")),
        is_error: Some(false),
    };
    assert_eq!(result_block.tool_use_id, "tool-123");
    assert_eq!(result_block.content, Some(json!("File contents here")));
    assert_eq!(result_block.is_error, Some(false));
}

#[test]
fn test_result_message() {
    let msg = ResultMessage {
        subtype: "success".to_string(),
        duration_ms: 1500,
        duration_api_ms: 1200,
        is_error: false,
        num_turns: 1,
        session_id: "session-123".to_string(),
        total_cost_usd: Some(0.01),
        usage: None,
        result: None,
        structured_output: None,
    };
    assert_eq!(msg.subtype, "success");
    assert_eq!(msg.total_cost_usd, Some(0.01));
    assert_eq!(msg.session_id, "session-123");
}

#[test]
fn test_default_options() {
    let options = ClaudeAgentOptions::default();
    assert!(options.allowed_tools.is_empty());
    assert!(options.system_prompt.is_none());
    assert!(options.permission_mode.is_none());
    assert!(!options.continue_conversation);
    assert!(options.disallowed_tools.is_empty());
}

#[test]
fn test_options_permission_modes() {
    let mut options = ClaudeAgentOptions {
        permission_mode: Some(PermissionMode::BypassPermissions),
        ..Default::default()
    };
    assert_eq!(
        options.permission_mode,
        Some(PermissionMode::BypassPermissions)
    );

    options.permission_mode = Some(PermissionMode::Plan);
    assert_eq!(options.permission_mode, Some(PermissionMode::Plan));

    options.permission_mode = Some(PermissionMode::Default);
    assert_eq!(options.permission_mode, Some(PermissionMode::Default));

    options.permission_mode = Some(PermissionMode::AcceptEdits);
    assert_eq!(options.permission_mode, Some(PermissionMode::AcceptEdits));
}
