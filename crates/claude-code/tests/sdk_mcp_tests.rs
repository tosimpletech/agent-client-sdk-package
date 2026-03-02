use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use claude_code::{
    ClaudeAgentOptions, McpServerConfig, McpServersOption, McpStdioServerConfig, ToolAnnotations,
    create_sdk_mcp_server, handle_sdk_mcp_request, tool,
};
use serde_json::{Value, json};

#[tokio::test]
async fn test_sdk_mcp_server_handlers() {
    let executions = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));

    let greet_exec = Arc::clone(&executions);
    let greet_user = tool(
        "greet_user",
        "Greets a user by name",
        json!({"type": "object", "properties": {"name": {"type": "string"}}}),
        move |args| {
            let greet_exec = Arc::clone(&greet_exec);
            async move {
                greet_exec
                    .lock()
                    .expect("lock")
                    .push(("greet_user".to_string(), args.clone()));
                let name = args["name"].as_str().unwrap_or_default();
                Ok(json!({
                    "content": [{"type": "text", "text": format!("Hello, {name}!")}]
                }))
            }
        },
    );

    let add_exec = Arc::clone(&executions);
    let add_numbers = tool(
        "add_numbers",
        "Adds two numbers",
        json!({"type": "object", "properties": {"a": {"type": "number"}, "b": {"type": "number"}}}),
        move |args| {
            let add_exec = Arc::clone(&add_exec);
            async move {
                add_exec
                    .lock()
                    .expect("lock")
                    .push(("add_numbers".to_string(), args.clone()));
                let a = args["a"].as_f64().unwrap_or_default();
                let b = args["b"].as_f64().unwrap_or_default();
                Ok(json!({
                    "content": [{"type": "text", "text": format!("The sum is {}", a + b)}]
                }))
            }
        },
    );

    let server = create_sdk_mcp_server("test-sdk-server", "1.0.0", vec![greet_user, add_numbers]);
    assert_eq!(server.type_, "sdk");
    assert_eq!(server.name, "test-sdk-server");
    assert!(server.instance.has_tools());

    let tools = server.instance.list_tools_json();
    assert_eq!(tools.len(), 2);
    let tool_names: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .collect();
    assert!(tool_names.contains(&"greet_user"));
    assert!(tool_names.contains(&"add_numbers"));

    let greet_result = server
        .instance
        .call_tool_json("greet_user", json!({"name": "Alice"}))
        .await;
    assert_eq!(greet_result["content"][0]["text"], "Hello, Alice!");

    let add_result = server
        .instance
        .call_tool_json("add_numbers", json!({"a": 5, "b": 3}))
        .await;
    assert_eq!(add_result["content"][0]["text"], "The sum is 8");

    let executions = executions.lock().expect("lock");
    assert_eq!(executions.len(), 2);
    assert_eq!(executions[0].0, "greet_user");
    assert_eq!(executions[0].1["name"], "Alice");
    assert_eq!(executions[1].0, "add_numbers");
    assert_eq!(executions[1].1["a"], 5);
    assert_eq!(executions[1].1["b"], 3);
}

#[tokio::test]
async fn test_tool_creation() {
    let echo_tool = tool(
        "echo",
        "Echo input",
        json!({"type": "object", "properties": {"input": {"type": "string"}}}),
        |args| async move { Ok(json!({"output": args["input"]})) },
    );

    assert_eq!(echo_tool.name, "echo");
    assert_eq!(echo_tool.description, "Echo input");
    assert_eq!(
        echo_tool.input_schema,
        json!({"type": "object", "properties": {"input": {"type": "string"}}})
    );

    let result = (echo_tool.handler)(json!({"input": "test"}))
        .await
        .expect("handler result");
    assert_eq!(result, json!({"output": "test"}));
}

#[tokio::test]
async fn test_error_handling() {
    let fail_tool = tool(
        "fail",
        "Always fails",
        json!({"type": "object"}),
        |_args| async move { Err(claude_code::Error::Other("Expected error".to_string())) },
    );

    let err = (fail_tool.handler)(json!({}))
        .await
        .expect_err("direct call should fail");
    assert!(err.to_string().contains("Expected error"));

    let server = create_sdk_mcp_server("error-test", "1.0.0", vec![fail_tool]);
    let result = server.instance.call_tool_json("fail", json!({})).await;
    assert_eq!(result["isError"], true);
    assert_eq!(result["is_error"], true);
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("Expected error")
    );
}

#[tokio::test]
async fn test_mixed_servers() {
    let sdk_tool = tool(
        "sdk_tool",
        "SDK tool",
        json!({"type": "object", "properties": {}}),
        |_args| async move { Ok(json!({"result": "from SDK"})) },
    );
    let sdk_server = create_sdk_mcp_server("sdk-server", "1.0.0", vec![sdk_tool]);

    let external_server = McpServerConfig::Stdio(McpStdioServerConfig {
        type_: Some("stdio".to_string()),
        command: "echo".to_string(),
        args: Some(vec!["test".to_string()]),
        env: None,
    });

    let mut servers = HashMap::new();
    servers.insert("sdk".to_string(), McpServerConfig::Sdk(sdk_server));
    servers.insert("external".to_string(), external_server);

    let options = ClaudeAgentOptions {
        mcp_servers: McpServersOption::Servers(servers),
        ..Default::default()
    };

    let McpServersOption::Servers(servers) = options.mcp_servers else {
        panic!("expected servers map");
    };
    assert!(servers.contains_key("sdk"));
    assert!(servers.contains_key("external"));

    let sdk = servers.get("sdk").expect("sdk");
    let external = servers.get("external").expect("external");
    assert!(matches!(sdk, McpServerConfig::Sdk(_)));
    assert!(matches!(external, McpServerConfig::Stdio(_)));
}

#[tokio::test]
async fn test_server_creation() {
    let server = create_sdk_mcp_server("test-server", "2.0.0", vec![]);

    assert_eq!(server.type_, "sdk");
    assert_eq!(server.name, "test-server");
    assert_eq!(server.instance.name, "test-server");
    assert_eq!(server.instance.version, "2.0.0");
    assert!(!server.instance.has_tools());
    assert!(server.instance.list_tools_json().is_empty());
}

#[tokio::test]
async fn test_image_content_support() {
    let png_data = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAACXBIWXMAAAsTAAALEwEAmpwYAAAADElEQVR4nGNgYGAAAAAEAAFdVSEcAAAAAElFTkSuQmCC";
    let executions = Arc::new(Mutex::new(Vec::<Value>::new()));

    let image_exec = Arc::clone(&executions);
    let generate_chart = tool(
        "generate_chart",
        "Generates a chart and returns it as an image",
        json!({"type": "object", "properties": {"title": {"type": "string"}}}),
        move |args| {
            let image_exec = Arc::clone(&image_exec);
            let png_data = png_data.to_string();
            async move {
                image_exec.lock().expect("lock").push(args.clone());
                let title = args["title"].as_str().unwrap_or_default();
                Ok(json!({
                    "content": [
                        {"type": "text", "text": format!("Generated chart: {title}")},
                        {"type": "image", "data": png_data, "mimeType": "image/png"}
                    ]
                }))
            }
        },
    );

    let server = create_sdk_mcp_server("image-test-server", "1.0.0", vec![generate_chart]);
    let result = server
        .instance
        .call_tool_json("generate_chart", json!({"title": "Sales Report"}))
        .await;

    assert_eq!(result["content"].as_array().map(Vec::len), Some(2));
    assert_eq!(result["content"][0]["type"], "text");
    assert_eq!(
        result["content"][0]["text"],
        "Generated chart: Sales Report"
    );
    assert_eq!(result["content"][1]["type"], "image");
    assert_eq!(result["content"][1]["data"], png_data);
    assert_eq!(result["content"][1]["mimeType"], "image/png");

    let executions = executions.lock().expect("lock");
    assert_eq!(executions.len(), 1);
    assert_eq!(executions[0]["title"], "Sales Report");
}

#[tokio::test]
async fn test_tool_annotations() {
    let read_data = tool(
        "read_data",
        "Read data from source",
        json!({"type": "object", "properties": {"source": {"type": "string"}}}),
        |args| async move { Ok(json!({"content": [{"type": "text", "text": args["source"]}]})) },
    )
    .with_annotations(ToolAnnotations {
        read_only_hint: Some(true),
        ..Default::default()
    });

    let delete_item = tool(
        "delete_item",
        "Delete an item",
        json!({"type": "object", "properties": {"id": {"type": "string"}}}),
        |args| async move { Ok(json!({"content": [{"type": "text", "text": args["id"]}]})) },
    )
    .with_annotations(ToolAnnotations {
        destructive_hint: Some(true),
        idempotent_hint: Some(true),
        ..Default::default()
    });

    let search = tool(
        "search",
        "Search the web",
        json!({"type": "object", "properties": {"query": {"type": "string"}}}),
        |args| async move { Ok(json!({"content": [{"type": "text", "text": args["query"]}]})) },
    )
    .with_annotations(ToolAnnotations {
        open_world_hint: Some(true),
        ..Default::default()
    });

    let no_annotations = tool(
        "no_annotations",
        "Tool without annotations",
        json!({"type": "object", "properties": {"x": {"type": "string"}}}),
        |args| async move { Ok(json!({"content": [{"type": "text", "text": args["x"]}]})) },
    );

    assert_eq!(
        read_data
            .annotations
            .as_ref()
            .and_then(|anno| anno.read_only_hint),
        Some(true)
    );
    assert_eq!(
        delete_item
            .annotations
            .as_ref()
            .and_then(|anno| anno.destructive_hint),
        Some(true)
    );
    assert_eq!(
        delete_item
            .annotations
            .as_ref()
            .and_then(|anno| anno.idempotent_hint),
        Some(true)
    );
    assert_eq!(
        search
            .annotations
            .as_ref()
            .and_then(|anno| anno.open_world_hint),
        Some(true)
    );
    assert!(no_annotations.annotations.is_none());

    let server = create_sdk_mcp_server(
        "annotations-test",
        "1.0.0",
        vec![read_data, delete_item, search, no_annotations],
    );
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

    assert_eq!(by_name["read_data"]["annotations"]["readOnlyHint"], true);
    assert_eq!(
        by_name["delete_item"]["annotations"]["destructiveHint"],
        true
    );
    assert_eq!(
        by_name["delete_item"]["annotations"]["idempotentHint"],
        true
    );
    assert_eq!(by_name["search"]["annotations"]["openWorldHint"], true);
    assert!(by_name["no_annotations"].get("annotations").is_none());
}

#[tokio::test]
async fn test_tool_annotations_in_jsonrpc() {
    let read_only_tool = tool(
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

    let server = create_sdk_mcp_server(
        "jsonrpc-annotations-test",
        "1.0.0",
        vec![read_only_tool, plain_tool],
    );

    let mut sdk_servers = HashMap::new();
    sdk_servers.insert("test".to_string(), Arc::clone(&server.instance));

    let response = handle_sdk_mcp_request(
        &sdk_servers,
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
    assert_eq!(
        by_name["read_only_tool"]["annotations"]["openWorldHint"],
        false
    );
    assert!(by_name["plain_tool"].get("annotations").is_none());
}
