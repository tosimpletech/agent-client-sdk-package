use claude_code_client_sdk::{DEFAULT_MAX_BUFFER_SIZE, JsonStreamBuffer};
use serde_json::json;

#[test]
fn test_multiple_json_objects_on_single_line() {
    let mut parser = JsonStreamBuffer::new(DEFAULT_MAX_BUFFER_SIZE);
    let buffered_line = format!(
        "{}\n{}",
        json!({"type": "message", "id": "msg1", "content": "First message"}),
        json!({"type": "result", "id": "res1", "status": "completed"})
    );

    let messages = parser.push_chunk(&buffered_line).expect("parse");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["type"], "message");
    assert_eq!(messages[1]["type"], "result");
}

#[test]
fn test_split_json_across_multiple_reads() {
    let mut parser = JsonStreamBuffer::new(DEFAULT_MAX_BUFFER_SIZE);
    let complete = json!({
        "type": "assistant",
        "message": {"content": [{"type": "text", "text": "x".repeat(1000)}]}
    })
    .to_string();

    let part1 = &complete[..100];
    let part2 = &complete[100..250];
    let part3 = &complete[250..];

    assert!(parser.push_chunk(part1).expect("part1").is_empty());
    assert!(parser.push_chunk(part2).expect("part2").is_empty());
    let messages = parser.push_chunk(part3).expect("part3");

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["type"], "assistant");
}

#[test]
fn test_buffer_size_exceeded() {
    let mut parser = JsonStreamBuffer::new(128);
    let huge_incomplete = format!("{{\"data\": \"{}\"", "x".repeat(200));
    let error = parser.push_chunk(&huge_incomplete).expect_err("must fail");
    assert!(
        error
            .to_string()
            .contains("maximum buffer size of 128 bytes")
    );
}

#[test]
fn test_mixed_complete_and_split_json() {
    let mut parser = JsonStreamBuffer::new(DEFAULT_MAX_BUFFER_SIZE);

    let msg1 = json!({"type": "system", "subtype": "start"}).to_string();
    let large = json!({
        "type": "assistant",
        "message": {"content": [{"type": "text", "text": "y".repeat(5000)}]}
    })
    .to_string();
    let msg3 = json!({"type": "system", "subtype": "end"}).to_string();

    let mut results = Vec::new();
    results.extend(parser.push_chunk(&(msg1 + "\n")).expect("msg1"));
    results.extend(parser.push_chunk(&large[..1000]).expect("part1"));
    results.extend(parser.push_chunk(&large[1000..3000]).expect("part2"));
    results.extend(
        parser
            .push_chunk(&(large[3000..].to_string() + "\n" + &msg3))
            .expect("part3"),
    );

    assert_eq!(results.len(), 3);
    assert_eq!(results[0]["subtype"], "start");
    assert_eq!(results[1]["type"], "assistant");
    assert_eq!(results[2]["subtype"], "end");
}

