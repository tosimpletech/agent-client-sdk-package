use std::path::PathBuf;

use claude_code::{
    ClaudeAgentOptions, ContentBlock, InputPrompt, Message, PermissionMode, UserContent, query,
};
use serde_json::{Value, json};

fn fixture_cli_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock_claude_cli.py")
}

fn base_options() -> ClaudeAgentOptions {
    ClaudeAgentOptions {
        cli_path: Some(fixture_cli_path()),
        permission_mode: Some(PermissionMode::AcceptEdits),
        max_turns: Some(1),
        ..Default::default()
    }
}

fn extract_result_structured_output(messages: &[Message]) -> &Value {
    messages
        .iter()
        .find_map(|message| match message {
            Message::Result(result) => result.structured_output.as_ref(),
            _ => None,
        })
        .expect("missing result.structured_output")
}

#[tokio::test]
async fn test_e2e_simple_structured_output() {
    let schema = json!({
        "type": "object",
        "properties": {
            "file_count": {"type": "number"},
            "has_tests": {"type": "boolean"},
            "test_file_count": {"type": "number"}
        },
        "required": ["file_count", "has_tests"]
    });

    let mut options = base_options();
    options.output_format = Some(json!({
        "type": "json_schema",
        "schema": schema
    }));

    let messages = query(
        InputPrompt::Text("Count files and detect tests.".to_string()),
        Some(options),
        None,
    )
    .await
    .expect("query");

    let output = extract_result_structured_output(&messages);
    assert!(output.get("file_count").is_some_and(Value::is_number));
    assert!(output.get("has_tests").is_some_and(Value::is_boolean));
}

#[tokio::test]
async fn test_e2e_nested_structured_output() {
    let schema = json!({
        "type": "object",
        "properties": {
            "analysis": {
                "type": "object",
                "properties": {
                    "word_count": {"type": "number"},
                    "character_count": {"type": "number"}
                },
                "required": ["word_count", "character_count"]
            },
            "words": {"type": "array", "items": {"type": "string"}}
        },
        "required": ["analysis", "words"]
    });

    let mut options = base_options();
    options.output_format = Some(json!({
        "type": "json_schema",
        "schema": schema
    }));

    let messages = query(
        InputPrompt::Text("Analyze 'Hello world'.".to_string()),
        Some(options),
        None,
    )
    .await
    .expect("query");

    let output = extract_result_structured_output(&messages);
    assert_eq!(output["analysis"]["word_count"], 2);
    assert_eq!(output["analysis"]["character_count"], 11);
    assert_eq!(output["words"].as_array().map_or(0, std::vec::Vec::len), 2);
}

#[tokio::test]
async fn test_e2e_structured_output_with_enum() {
    let schema = json!({
        "type": "object",
        "properties": {
            "has_tests": {"type": "boolean"},
            "test_framework": {
                "type": "string",
                "enum": ["pytest", "unittest", "nose", "unknown"]
            },
            "test_count": {"type": "number"}
        },
        "required": ["has_tests", "test_framework"]
    });

    let mut options = base_options();
    options.output_format = Some(json!({
        "type": "json_schema",
        "schema": schema
    }));

    let messages = query(
        InputPrompt::Text("Detect test framework.".to_string()),
        Some(options),
        None,
    )
    .await
    .expect("query");

    let output = extract_result_structured_output(&messages);
    let framework = output
        .get("test_framework")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(matches!(
        framework,
        "pytest" | "unittest" | "nose" | "unknown"
    ));
    assert_eq!(framework, "pytest");
    assert_eq!(output["has_tests"], true);
}

#[tokio::test]
async fn test_e2e_structured_output_with_tools() {
    let schema = json!({
        "type": "object",
        "properties": {
            "file_count": {"type": "number"},
            "has_readme": {"type": "boolean"}
        },
        "required": ["file_count", "has_readme"]
    });

    let mut options = base_options();
    options.output_format = Some(json!({
        "type": "json_schema",
        "schema": schema
    }));
    options.env.insert(
        "MOCK_CLAUDE_STRUCTURED_WITH_TOOLS".to_string(),
        "1".to_string(),
    );

    let messages = query(
        InputPrompt::Text("Count files with tool use.".to_string()),
        Some(options),
        None,
    )
    .await
    .expect("query");

    let saw_tool_use = messages.iter().any(|message| match message {
        Message::Assistant(assistant) => assistant
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolUse(_))),
        _ => false,
    });
    assert!(saw_tool_use, "expected assistant tool_use block");

    let saw_tool_result = messages.iter().any(|message| match message {
        Message::User(user) => match &user.content {
            UserContent::Blocks(blocks) => blocks
                .iter()
                .any(|block| matches!(block, ContentBlock::ToolResult(_))),
            UserContent::Text(_) => false,
        },
        _ => false,
    });
    assert!(saw_tool_result, "expected user tool_result block");

    let output = extract_result_structured_output(&messages);
    assert!(output.get("file_count").is_some_and(Value::is_number));
    assert!(output.get("has_readme").is_some_and(Value::is_boolean));
}
