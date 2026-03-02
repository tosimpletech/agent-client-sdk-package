use claude_code::{ClaudeAgentOptions, Message, ResultMessage};
use serde_json::json;

#[test]
fn test_output_format_json_schema_config() {
    let options = ClaudeAgentOptions {
        output_format: Some(json!({
            "type": "json_schema",
            "schema": {
                "type": "object",
                "properties": {
                    "answer": {"type": "string"},
                    "confidence": {"type": "number"}
                },
                "required": ["answer", "confidence"]
            }
        })),
        ..Default::default()
    };

    let output_format = options.output_format.as_ref().unwrap();
    assert_eq!(output_format["type"], "json_schema");
    assert!(output_format["schema"]["properties"]["answer"].is_object());
    assert!(output_format["schema"]["properties"]["confidence"].is_object());
}

#[test]
fn test_result_message_with_structured_output() {
    let result_json = json!({
        "subtype": "success",
        "duration_ms": 1000,
        "duration_api_ms": 800,
        "is_error": false,
        "num_turns": 1,
        "session_id": "test-session",
        "total_cost_usd": 0.05,
        "result": null,
        "structured_output": {
            "answer": "Paris",
            "confidence": 0.95
        }
    });

    let result: ResultMessage = serde_json::from_value(result_json).expect("deserialize");
    assert_eq!(result.subtype, "success");
    assert!(!result.is_error);

    let structured = result.structured_output.expect("structured_output");
    assert_eq!(structured["answer"], "Paris");
    assert_eq!(structured["confidence"], 0.95);
}

#[test]
fn test_result_message_without_structured_output() {
    let result_json = json!({
        "subtype": "success",
        "duration_ms": 500,
        "duration_api_ms": 400,
        "is_error": false,
        "num_turns": 1,
        "session_id": "test-session",
        "total_cost_usd": 0.01,
        "result": "Plain text response"
    });

    let result: ResultMessage = serde_json::from_value(result_json).expect("deserialize");
    assert_eq!(result.result, Some("Plain text response".to_string()));
    assert!(result.structured_output.is_none());
}

#[test]
fn test_result_message_parsed_via_message_parser() {
    let raw = json!({
        "type": "result",
        "subtype": "success",
        "duration_ms": 1200,
        "duration_api_ms": 1000,
        "is_error": false,
        "num_turns": 2,
        "session_id": "structured-session",
        "total_cost_usd": 0.1,
        "structured_output": {
            "items": ["a", "b", "c"],
            "count": 3
        }
    });

    let message = claude_code::parse_message(&raw)
        .expect("parse ok")
        .expect("some message");

    if let Message::Result(result) = message {
        let structured = result.structured_output.expect("structured_output");
        assert_eq!(structured["count"], 3);
        let items = structured["items"].as_array().expect("array");
        assert_eq!(items.len(), 3);
    } else {
        panic!("Expected Message::Result");
    }
}
