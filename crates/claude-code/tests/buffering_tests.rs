use claude_code::{DEFAULT_MAX_BUFFER_SIZE, JsonStreamBuffer};
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
fn test_json_with_embedded_newlines() {
    let mut parser = JsonStreamBuffer::new(DEFAULT_MAX_BUFFER_SIZE);
    let buffered_line = format!(
        "{}\n{}",
        json!({"type": "message", "content": "Line 1\nLine 2\nLine 3"}),
        json!({"type": "result", "data": "Some\nMultiline\nContent"})
    );

    let messages = parser.push_chunk(&buffered_line).expect("parse");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["content"], "Line 1\nLine 2\nLine 3");
    assert_eq!(messages[1]["data"], "Some\nMultiline\nContent");
}

#[test]
fn test_multiple_newlines_between_objects() {
    let mut parser = JsonStreamBuffer::new(DEFAULT_MAX_BUFFER_SIZE);
    let buffered_line = format!(
        "{}\n\n\n{}",
        json!({"type": "message", "id": "msg1"}),
        json!({"type": "result", "id": "res1"})
    );

    let messages = parser.push_chunk(&buffered_line).expect("parse");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["id"], "msg1");
    assert_eq!(messages[1]["id"], "res1");
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
fn test_large_minified_json() {
    let mut parser = JsonStreamBuffer::new(DEFAULT_MAX_BUFFER_SIZE);
    let large_data = json!({
        "records": (0..1200)
            .map(|i| json!({
                "id": i,
                "payload": {
                    "text": "x".repeat(120),
                    "tags": [format!("tag-{}", i % 7), format!("bucket-{}", i % 13)],
                    "meta": {"group": i % 10, "active": i % 2 == 0}
                }
            }))
            .collect::<Vec<_>>(),
        "summary": {"count": 1200, "kind": "nested-large-payload"}
    });
    let complete = json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": [{
                "tool_use_id": "toolu_016fed1NhiaMLqnEvrj5NUaj",
                "type": "tool_result",
                "content": large_data.to_string()
            }]
        }
    })
    .to_string();

    assert!(complete.len() > 100 * 1024, "payload must be at least 100KB");

    let chunk_size = 64 * 1024;
    let mut messages = Vec::new();
    let mut start = 0;
    while start < complete.len() {
        let end = (start + chunk_size).min(complete.len());
        messages.extend(parser.push_chunk(&complete[start..end]).expect("chunk parse"));
        start = end;
    }

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["type"], "user");
    assert_eq!(
        messages[0]["message"]["content"][0]["tool_use_id"],
        "toolu_016fed1NhiaMLqnEvrj5NUaj"
    );
}

#[test]
fn test_buffer_size_exceeded() {
    let mut parser = JsonStreamBuffer::new(DEFAULT_MAX_BUFFER_SIZE);
    let huge_incomplete = format!(
        "{{\"data\": \"{}\"",
        "x".repeat(DEFAULT_MAX_BUFFER_SIZE + 1000)
    );
    let error = parser.push_chunk(&huge_incomplete).expect_err("must fail");
    assert!(
        error
            .to_string()
            .contains(&format!(
                "maximum buffer size of {} bytes",
                DEFAULT_MAX_BUFFER_SIZE
            ))
    );
}

#[test]
fn test_buffer_size_option() {
    let custom_limit = 512;
    let mut parser = JsonStreamBuffer::new(custom_limit);
    let huge_incomplete = format!("{{\"data\": \"{}\"", "x".repeat(custom_limit + 10));

    let error = parser.push_chunk(&huge_incomplete).expect_err("must fail");
    assert!(
        error
            .to_string()
            .contains(&format!("maximum buffer size of {custom_limit} bytes"))
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
