use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use claude_code::{
    ClaudeAgentOptions, ContentBlock, InputPrompt, McpServerConfig, McpServersOption, Message,
    create_sdk_mcp_server, query, tool,
};
use serde_json::json;

fn fixture_cli_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock_claude_cli.py")
}

fn base_options() -> ClaudeAgentOptions {
    ClaudeAgentOptions {
        cli_path: Some(fixture_cli_path()),
        ..Default::default()
    }
}

fn contains_result(messages: &[Message]) -> bool {
    messages.iter().any(|msg| matches!(msg, Message::Result(_)))
}

fn assistant_text(messages: &[Message]) -> String {
    messages
        .iter()
        .find_map(|msg| match msg {
            Message::Assistant(assistant) => {
                assistant.content.iter().find_map(|block| match block {
                    ContentBlock::Text(text) => Some(text.text.clone()),
                    _ => None,
                })
            }
            _ => None,
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn test_sdk_mcp_tool_execution() {
    let executions = Arc::new(Mutex::new(Vec::<String>::new()));

    let echo_exec = Arc::clone(&executions);
    let echo_tool = tool(
        "echo",
        "Echo back the input text",
        json!({"type": "object", "properties": {"text": {"type": "string"}}}),
        move |args| {
            let echo_exec = Arc::clone(&echo_exec);
            async move {
                echo_exec.lock().expect("lock").push("echo".to_string());
                Ok(
                    json!({"content": [{"type": "text", "text": format!("Echo: {}", args["text"])}]}),
                )
            }
        },
    );

    let sdk_server = create_sdk_mcp_server("test", "1.0.0", vec![echo_tool]);
    let mut servers = HashMap::new();
    servers.insert("test".to_string(), McpServerConfig::Sdk(sdk_server));

    let mut options = base_options();
    options.mcp_servers = McpServersOption::Servers(servers);
    options.allowed_tools = vec!["mcp__test__echo".to_string()];
    options
        .env
        .insert("MOCK_CLAUDE_TRIGGER_MCP".to_string(), "1".to_string());
    options.env.insert(
        "MOCK_CLAUDE_MCP_SCENARIO".to_string(),
        "tool_execution".to_string(),
    );

    let messages = query(
        InputPrompt::Text("Call the mcp__test__echo tool".to_string()),
        Some(options),
        None,
    )
    .await
    .expect("query");

    assert!(contains_result(&messages));
    assert!(assistant_text(&messages).contains("tool_execution"));
    assert_eq!(executions.lock().expect("lock").as_slice(), &["echo"]);
}

#[tokio::test]
async fn test_sdk_mcp_permission_enforcement() {
    let executions = Arc::new(Mutex::new(Vec::<String>::new()));

    let echo_exec = Arc::clone(&executions);
    let echo_tool = tool(
        "echo",
        "Echo back the input text",
        json!({"type": "object", "properties": {"text": {"type": "string"}}}),
        move |args| {
            let echo_exec = Arc::clone(&echo_exec);
            async move {
                echo_exec.lock().expect("lock").push("echo".to_string());
                Ok(
                    json!({"content": [{"type": "text", "text": format!("Echo: {}", args["text"])}]}),
                )
            }
        },
    );

    let greet_exec = Arc::clone(&executions);
    let greet_tool = tool(
        "greet",
        "Greet a person by name",
        json!({"type": "object", "properties": {"name": {"type": "string"}}}),
        move |args| {
            let greet_exec = Arc::clone(&greet_exec);
            async move {
                greet_exec.lock().expect("lock").push("greet".to_string());
                Ok(
                    json!({"content": [{"type": "text", "text": format!("Hello, {}!", args["name"])}]}),
                )
            }
        },
    );

    let sdk_server = create_sdk_mcp_server("test", "1.0.0", vec![echo_tool, greet_tool]);
    let mut servers = HashMap::new();
    servers.insert("test".to_string(), McpServerConfig::Sdk(sdk_server));

    let mut options = base_options();
    options.mcp_servers = McpServersOption::Servers(servers);
    options.allowed_tools = vec!["mcp__test__greet".to_string()];
    options.disallowed_tools = vec!["mcp__test__echo".to_string()];
    options
        .env
        .insert("MOCK_CLAUDE_TRIGGER_MCP".to_string(), "1".to_string());
    options.env.insert(
        "MOCK_CLAUDE_MCP_SCENARIO".to_string(),
        "permission_enforcement".to_string(),
    );

    let messages = query(
        InputPrompt::Text("Use greet, then echo.".to_string()),
        Some(options),
        None,
    )
    .await
    .expect("query");

    assert!(contains_result(&messages));
    assert!(assistant_text(&messages).contains("permission_enforcement"));
    let executions = executions.lock().expect("lock");
    assert!(executions.contains(&"greet".to_string()));
    assert!(!executions.contains(&"echo".to_string()));
}

#[tokio::test]
async fn test_sdk_mcp_multiple_tools() {
    let executions = Arc::new(Mutex::new(Vec::<String>::new()));

    let echo_exec = Arc::clone(&executions);
    let echo_tool = tool(
        "echo",
        "Echo back the input text",
        json!({"type": "object", "properties": {"text": {"type": "string"}}}),
        move |args| {
            let echo_exec = Arc::clone(&echo_exec);
            async move {
                echo_exec.lock().expect("lock").push("echo".to_string());
                Ok(
                    json!({"content": [{"type": "text", "text": format!("Echo: {}", args["text"])}]}),
                )
            }
        },
    );

    let greet_exec = Arc::clone(&executions);
    let greet_tool = tool(
        "greet",
        "Greet a person by name",
        json!({"type": "object", "properties": {"name": {"type": "string"}}}),
        move |args| {
            let greet_exec = Arc::clone(&greet_exec);
            async move {
                greet_exec.lock().expect("lock").push("greet".to_string());
                Ok(
                    json!({"content": [{"type": "text", "text": format!("Hello, {}!", args["name"])}]}),
                )
            }
        },
    );

    let sdk_server = create_sdk_mcp_server("multi", "1.0.0", vec![echo_tool, greet_tool]);
    let mut servers = HashMap::new();
    servers.insert("multi".to_string(), McpServerConfig::Sdk(sdk_server));

    let mut options = base_options();
    options.mcp_servers = McpServersOption::Servers(servers);
    options.allowed_tools = vec![
        "mcp__multi__echo".to_string(),
        "mcp__multi__greet".to_string(),
    ];
    options
        .env
        .insert("MOCK_CLAUDE_TRIGGER_MCP".to_string(), "1".to_string());
    options.env.insert(
        "MOCK_CLAUDE_MCP_SCENARIO".to_string(),
        "multiple_tools".to_string(),
    );

    let messages = query(
        InputPrompt::Text("Call both tools.".to_string()),
        Some(options),
        None,
    )
    .await
    .expect("query");

    assert!(contains_result(&messages));
    assert!(assistant_text(&messages).contains("multiple_tools"));
    let executions = executions.lock().expect("lock");
    assert!(executions.contains(&"echo".to_string()));
    assert!(executions.contains(&"greet".to_string()));
}

#[tokio::test]
async fn test_sdk_mcp_without_permissions() {
    let executions = Arc::new(Mutex::new(Vec::<String>::new()));

    let echo_exec = Arc::clone(&executions);
    let echo_tool = tool(
        "echo",
        "Echo back the input text",
        json!({"type": "object", "properties": {"text": {"type": "string"}}}),
        move |args| {
            let echo_exec = Arc::clone(&echo_exec);
            async move {
                echo_exec.lock().expect("lock").push("echo".to_string());
                Ok(
                    json!({"content": [{"type": "text", "text": format!("Echo: {}", args["text"])}]}),
                )
            }
        },
    );

    let sdk_server = create_sdk_mcp_server("noperm", "1.0.0", vec![echo_tool]);
    let mut servers = HashMap::new();
    servers.insert("noperm".to_string(), McpServerConfig::Sdk(sdk_server));

    let mut options = base_options();
    options.mcp_servers = McpServersOption::Servers(servers);
    options
        .env
        .insert("MOCK_CLAUDE_TRIGGER_MCP".to_string(), "1".to_string());
    options.env.insert(
        "MOCK_CLAUDE_MCP_SCENARIO".to_string(),
        "without_permissions".to_string(),
    );

    let messages = query(
        InputPrompt::Text("Call the mcp__noperm__echo tool.".to_string()),
        Some(options),
        None,
    )
    .await
    .expect("query");

    assert!(contains_result(&messages));
    assert!(assistant_text(&messages).contains("without_permissions"));
    assert!(executions.lock().expect("lock").is_empty());
}
