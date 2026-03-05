use claude_code::{ContentBlock, Message, parse_message};
use serde_json::json;

#[test]
fn test_parse_valid_user_message() {
    let data = json!({
        "type": "user",
        "message": {"content": [{"type": "text", "text": "Hello"}]}
    });
    let message = parse_message(&data).expect("parse").expect("message");
    match message {
        Message::User(msg) => match msg.content {
            claude_code::UserContent::Blocks(blocks) => {
                assert_eq!(blocks.len(), 1);
                match &blocks[0] {
                    ContentBlock::Text(block) => assert_eq!(block.text, "Hello"),
                    _ => panic!("expected text block"),
                }
            }
            _ => panic!("expected blocks"),
        },
        _ => panic!("expected user message"),
    }
}

#[test]
fn test_parse_user_message_with_uuid_and_tool_result() {
    let data = json!({
        "type": "user",
        "uuid": "msg-abc123-def456",
        "tool_use_result": {"filePath": "/tmp/a.py"},
        "message": {"content": "Simple string content"}
    });
    let message = parse_message(&data).expect("parse").expect("message");
    match message {
        Message::User(msg) => {
            assert_eq!(msg.uuid.as_deref(), Some("msg-abc123-def456"));
            assert_eq!(msg.tool_use_result, Some(json!({"filePath": "/tmp/a.py"})));
            assert_eq!(
                msg.content,
                claude_code::UserContent::Text("Simple string content".to_string())
            );
        }
        _ => panic!("expected user message"),
    }
}

#[test]
fn test_parse_user_message_with_mixed_blocks() {
    let data = json!({
        "type": "user",
        "message": {
            "content": [
                {"type": "text", "text": "A"},
                {"type": "tool_use", "id": "tool_1", "name": "Read", "input": {"file_path": "/x"}},
                {"type": "tool_result", "tool_use_id": "tool_1", "content": "OK", "is_error": true}
            ]
        }
    });
    let message = parse_message(&data).expect("parse").expect("message");
    match message {
        Message::User(msg) => match msg.content {
            claude_code::UserContent::Blocks(blocks) => {
                assert_eq!(blocks.len(), 3);
                assert!(matches!(&blocks[0], ContentBlock::Text(_)));
                assert!(matches!(&blocks[1], ContentBlock::ToolUse(_)));
                assert!(matches!(&blocks[2], ContentBlock::ToolResult(_)));
            }
            _ => panic!("expected blocks"),
        },
        _ => panic!("expected user message"),
    }
}

#[test]
fn test_parse_valid_assistant_message_with_thinking_and_error() {
    let data = json!({
        "type": "assistant",
        "message": {
            "content": [
                {"type": "thinking", "thinking": "I'm thinking", "signature": "sig-123"},
                {"type": "text", "text": "Result"}
            ],
            "model": "claude-opus-4-1-20250805"
        },
        "error": "rate_limit",
        "parent_tool_use_id": "toolu_1"
    });
    let message = parse_message(&data).expect("parse").expect("message");
    match message {
        Message::Assistant(msg) => {
            assert_eq!(msg.content.len(), 2);
            assert_eq!(msg.error.as_deref(), Some("rate_limit"));
            assert_eq!(msg.parent_tool_use_id.as_deref(), Some("toolu_1"));
        }
        _ => panic!("expected assistant message"),
    }
}

#[test]
fn test_parse_valid_system_message() {
    let data = json!({"type": "system", "subtype": "start"});
    let message = parse_message(&data).expect("parse").expect("message");
    match message {
        Message::System(msg) => assert_eq!(msg.subtype, "start"),
        _ => panic!("expected system message"),
    }
}

#[test]
fn test_parse_task_system_messages_with_typed_accessors() {
    let started = json!({
        "type": "system",
        "subtype": "task_started",
        "task_id": "task-1",
        "description": "Start",
        "uuid": "uuid-1",
        "session_id": "session-1",
        "task_type": "background"
    });
    let message = parse_message(&started).expect("parse").expect("message");
    match message {
        Message::System(msg) => {
            let typed = msg.as_task_started().expect("task_started");
            assert_eq!(typed.task_id, "task-1");
            assert_eq!(typed.task_type.as_deref(), Some("background"));
        }
        _ => panic!("expected system message"),
    }

    let progress = json!({
        "type": "system",
        "subtype": "task_progress",
        "task_id": "task-1",
        "description": "Halfway",
        "usage": {"total_tokens": 10, "tool_uses": 2, "duration_ms": 33},
        "uuid": "uuid-2",
        "session_id": "session-1",
        "last_tool_name": "Read"
    });
    let message = parse_message(&progress).expect("parse").expect("message");
    match message {
        Message::System(msg) => {
            let typed = msg.as_task_progress().expect("task_progress");
            assert_eq!(typed.usage.total_tokens, 10);
            assert_eq!(typed.last_tool_name.as_deref(), Some("Read"));
        }
        _ => panic!("expected system message"),
    }

    let notification = json!({
        "type": "system",
        "subtype": "task_notification",
        "task_id": "task-1",
        "status": "completed",
        "output_file": "/tmp/out.txt",
        "summary": "Done",
        "uuid": "uuid-3",
        "session_id": "session-1"
    });
    let message = parse_message(&notification)
        .expect("parse")
        .expect("message");
    match message {
        Message::System(msg) => {
            let typed = msg.as_task_notification().expect("task_notification");
            assert_eq!(typed.output_file, "/tmp/out.txt");
            assert!(matches!(
                typed.status,
                claude_code::TaskNotificationStatus::Completed
            ));
        }
        _ => panic!("expected system message"),
    }
}

#[test]
fn test_parse_valid_result_message() {
    let data = json!({
        "type": "result",
        "subtype": "success",
        "duration_ms": 1000,
        "duration_api_ms": 500,
        "is_error": false,
        "num_turns": 2,
        "session_id": "session_123",
        "stop_reason": "end_turn"
    });
    let message = parse_message(&data).expect("parse").expect("message");
    match message {
        Message::Result(msg) => {
            assert_eq!(msg.subtype, "success");
            assert_eq!(msg.stop_reason.as_deref(), Some("end_turn"));
        }
        _ => panic!("expected result message"),
    }
}

#[test]
fn test_parse_invalid_data_type() {
    let data = json!("not a dict");
    let error = parse_message(&data).expect_err("should fail");
    assert!(error.to_string().contains("Invalid message data type"));
    assert!(error.to_string().contains("expected dict, got str"));
}

#[test]
fn test_parse_missing_type_field() {
    let data = json!({"message": {"content": []}});
    let error = parse_message(&data).expect_err("should fail");
    assert!(error.to_string().contains("Message missing 'type' field"));
}

#[test]
fn test_parse_unknown_message_type_returns_none() {
    let result = parse_message(&json!({"type": "unknown_type"})).expect("parse ok");
    assert!(result.is_none());
}

#[test]
fn test_rate_limit_event_returns_none() {
    let data = json!({
        "type": "rate_limit_event",
        "rate_limit_info": {
            "status": "allowed_warning",
            "resetsAt": 1700000000,
            "rateLimitType": "five_hour",
            "utilization": 0.85,
            "isUsingOverage": false
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "session_id": "test-session-id"
    });

    let result = parse_message(&data).expect("parse ok");
    assert!(result.is_none());
}

#[test]
fn test_rate_limit_event_rejected_returns_none() {
    let data = json!({
        "type": "rate_limit_event",
        "rate_limit_info": {
            "status": "rejected",
            "resetsAt": 1700003600,
            "rateLimitType": "seven_day",
            "isUsingOverage": false,
            "overageStatus": "rejected",
            "overageDisabledReason": "out_of_credits"
        },
        "uuid": "660e8400-e29b-41d4-a716-446655440001",
        "session_id": "test-session-id"
    });

    let result = parse_message(&data).expect("parse ok");
    assert!(result.is_none());
}

#[test]
fn test_known_message_types_still_parsed() {
    let data = json!({
        "type": "assistant",
        "message": {
            "content": [{"type": "text", "text": "hello"}],
            "model": "claude-sonnet-4-6-20250929"
        }
    });

    let result = parse_message(&data).expect("parse ok");
    match result {
        Some(Message::Assistant(message)) => match &message.content[0] {
            ContentBlock::Text(block) => assert_eq!(block.text, "hello"),
            _ => panic!("expected text block"),
        },
        _ => panic!("expected assistant message"),
    }
}

#[test]
fn test_parse_missing_fields_errors_contain_data() {
    let data = json!({"type": "assistant"});
    let error = parse_message(&data).expect_err("should fail");
    assert!(
        error
            .to_string()
            .contains("Missing required field in assistant message")
    );
    assert_eq!(error.data, Some(data));
}
