use std::collections::HashMap;
use std::sync::Arc;

use claude_code::{ToolAnnotations, create_sdk_mcp_server, handle_sdk_mcp_request, tool};
use serde_json::{Value, json};

#[tokio::test]
async fn test_tool_creation_and_call() {
    let greet_tool = tool(
        "greet_user",
        "Greets a user by name",
        json!({"type": "object", "properties": {"name": {"type": "string"}}}),
        |args| async move {
            let name = args["name"].as_str().unwrap_or_default();
            Ok(json!({
                "content": [{"type": "text", "text": format!("Hello, {name}!")}]
            }))
        },
    );

    let server = create_sdk_mcp_server("test-sdk-server", "1.0.0", vec![greet_tool]);
    assert_eq!(server.type_, "sdk");
    assert_eq!(server.name, "test-sdk-server");

    let tools = server.instance.list_tools_json();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "greet_user");

    let result = server
        .instance
        .call_tool_json("greet_user", json!({"name": "Alice"}))
        .await;
    assert_eq!(result["content"][0]["text"], "Hello, Alice!");
}

#[tokio::test]
async fn test_tool_error_handling() {
    let fail_tool = tool("fail", "Always fails", json!({}), |_args| async move {
        Err(claude_code::Error::Other("Expected error".to_string()))
    });

    let server = create_sdk_mcp_server("error-test", "1.0.0", vec![fail_tool]);
    let result = server.instance.call_tool_json("fail", json!({})).await;
    assert_eq!(result["is_error"], true);
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("Expected error")
    );
}

#[tokio::test]
async fn test_tool_annotations_in_list_and_jsonrpc() {
    let read_only = tool(
        "read_only_tool",
        "A read-only tool",
        json!({"type": "object", "properties": {"input": {"type": "string"}}}),
        |args| async move { Ok(json!({"content": [{"type": "text", "text": args["input"]}]})) },
    )
    .with_annotations(ToolAnnotations {
        read_only_hint: Some(true),
        open_world_hint: Some(false),
        ..Default::default()
    });

    let plain_tool = tool(
        "plain_tool",
        "A tool without annotations",
        json!({"type": "object", "properties": {"input": {"type": "string"}}}),
        |args| async move { Ok(json!({"content": [{"type": "text", "text": args["input"]}]})) },
    );

    let server = create_sdk_mcp_server("annotations-test", "1.0.0", vec![read_only, plain_tool]);
    let list = server.instance.list_tools_json();
    let by_name: HashMap<String, Value> = list
        .into_iter()
        .map(|item| {
            (
                item["name"].as_str().unwrap_or_default().to_string(),
                item.clone(),
            )
        })
        .collect();

    assert_eq!(
        by_name["read_only_tool"]["annotations"]["readOnlyHint"],
        true
    );
    assert!(
        by_name["plain_tool"]["annotations"].is_null()
            || by_name["plain_tool"].get("annotations").is_none()
    );

    // Test through handle_sdk_mcp_request for JSON-RPC format verification.
    let mut servers = HashMap::new();
    servers.insert("test".to_string(), Arc::clone(&server.instance));

    let response = handle_sdk_mcp_request(
        &servers,
        "test",
        &json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}),
    )
    .await;
    let tools = response["result"]["tools"].as_array().expect("tools");
    let by_name: HashMap<String, Value> = tools
        .iter()
        .map(|item| {
            (
                item["name"].as_str().unwrap_or_default().to_string(),
                item.clone(),
            )
        })
        .collect();
    assert_eq!(
        by_name["read_only_tool"]["annotations"]["readOnlyHint"],
        true
    );
}
