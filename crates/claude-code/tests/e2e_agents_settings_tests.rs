use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use claude_code::{
    AgentDefinition, ClaudeAgentOptions, ClaudeSdkClient, InputPrompt, Message, SettingSource,
    query,
};
use serde_json::Value;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn fixture_cli_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock_claude_cli.py")
}

fn base_options() -> ClaudeAgentOptions {
    ClaudeAgentOptions {
        cli_path: Some(fixture_cli_path()),
        max_turns: Some(1),
        ..Default::default()
    }
}

fn unique_temp_path(prefix: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    let pid = std::process::id();
    let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("claude-code-{prefix}-{pid}-{timestamp}-{sequence}"))
}

struct TempProjectDir {
    path: PathBuf,
}

impl TempProjectDir {
    fn new(prefix: &str) -> Self {
        let path = unique_temp_path(prefix);
        fs::create_dir_all(&path).expect("create temp project directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write_file(&self, relative_path: &str, content: &str) {
        let file_path = self.path.join(relative_path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(file_path, content).expect("write test file");
    }
}

impl Drop for TempProjectDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn extract_init_message_data(messages: &[Message]) -> &Value {
    messages
        .iter()
        .find_map(|msg| match msg {
            Message::System(system) if system.subtype == "init" => Some(&system.data),
            _ => None,
        })
        .expect("missing system init message")
}

fn extract_string_list_field<'a>(init_data: &'a Value, field_name: &str) -> Vec<&'a str> {
    init_data
        .get(field_name)
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn generate_large_agents(num_agents: usize, prompt_size_kb: usize) -> HashMap<String, AgentDefinition> {
    let mut agents = HashMap::new();
    for i in 0..num_agents {
        let prompt_content = format!("You are test agent #{i}. ") + &"x".repeat(prompt_size_kb * 1024);
        agents.insert(
            format!("large-agent-{i}"),
            AgentDefinition {
                description: format!("Large test agent #{i} for stress testing"),
                prompt: prompt_content,
                tools: None,
                model: None,
            },
        );
    }
    agents
}

fn serialized_agents_size(agents: &HashMap<String, AgentDefinition>) -> usize {
    serde_json::to_vec(agents)
        .expect("serialize agents")
        .len()
}

#[tokio::test]
async fn test_e2e_agent_definition_in_init() {
    let mut agents = HashMap::new();
    agents.insert(
        "test-agent".to_string(),
        AgentDefinition {
            description: "A test agent for verification".to_string(),
            prompt: "You are a test agent.".to_string(),
            tools: Some(vec!["Read".to_string()]),
            model: Some("sonnet".to_string()),
        },
    );

    let mut options = base_options();
    options.agents = Some(agents);

    let mut client = ClaudeSdkClient::new(Some(options), None);
    client.connect(None).await.expect("connect");
    client
        .query(InputPrompt::Text("What is 2 + 2?".to_string()), "default")
        .await
        .expect("query");

    let messages = client.receive_response().await.expect("receive response");
    let init_data = extract_init_message_data(&messages);
    let init_agents = extract_string_list_field(init_data, "agents");

    assert!(init_agents.contains(&"test-agent"));
    client.disconnect().await.expect("disconnect");
}

#[tokio::test]
async fn test_e2e_agent_via_query_function() {
    let mut agents = HashMap::new();
    agents.insert(
        "test-agent-query".to_string(),
        AgentDefinition {
            description: "A test agent for query() verification".to_string(),
            prompt: "You are a query-function test agent.".to_string(),
            tools: None,
            model: None,
        },
    );

    let mut options = base_options();
    options.agents = Some(agents);

    let messages = query(
        InputPrompt::Text("What is 2 + 2?".to_string()),
        Some(options),
        None,
    )
    .await
    .expect("query");

    let init_data = extract_init_message_data(&messages);
    let init_agents = extract_string_list_field(init_data, "agents");
    assert!(init_agents.contains(&"test-agent-query"));
}

#[tokio::test]
async fn test_e2e_large_agents() {
    let agents = generate_large_agents(20, 13);
    assert!(
        serialized_agents_size(&agents) > 250_000,
        "test fixture must exceed 250KB"
    );

    let mut options = base_options();
    options.agents = Some(agents.clone());

    let messages = query(
        InputPrompt::Text("List available agents".to_string()),
        Some(options),
        None,
    )
    .await
    .expect("query");

    let init_data = extract_init_message_data(&messages);
    let init_agents = extract_string_list_field(init_data, "agents");
    for agent_name in agents.keys() {
        assert!(
            init_agents.contains(&agent_name.as_str()),
            "{agent_name} should be registered"
        );
    }

    assert_eq!(
        init_data
            .get("initialize_has_agents")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert!(
        init_data
            .get("initialize_agent_bytes")
            .and_then(Value::as_u64)
            .unwrap_or_default()
            > 250_000
    );
}

#[tokio::test]
async fn test_e2e_filesystem_agent_loading() {
    let temp_project = TempProjectDir::new("agents-filesystem");
    temp_project.write_file(
        ".claude/agents/fs-test-agent.md",
        r#"---
name: fs-test-agent
description: Filesystem test agent
tools: Read
---

You are a filesystem test agent.
"#,
    );

    let mut options = base_options();
    options.cwd = Some(temp_project.path().to_path_buf());
    options.setting_sources = Some(vec![SettingSource::Project]);

    let messages = query(
        InputPrompt::Text("Say hello in exactly 3 words".to_string()),
        Some(options),
        None,
    )
    .await
    .expect("query");

    let init_data = extract_init_message_data(&messages);
    let init_agents = extract_string_list_field(init_data, "agents");
    assert!(init_agents.contains(&"fs-test-agent"));
    assert!(messages.iter().any(|msg| matches!(msg, Message::Assistant(_))));
    assert!(messages.iter().any(|msg| matches!(msg, Message::Result(_))));
}

#[tokio::test]
async fn test_e2e_setting_sources_default() {
    let temp_project = TempProjectDir::new("settings-default");
    temp_project.write_file(
        ".claude/settings.local.json",
        r#"{"outputStyle":"local-test-style"}"#,
    );

    let mut options = base_options();
    options.cwd = Some(temp_project.path().to_path_buf());

    let messages = query(
        InputPrompt::Text("What is 2 + 2?".to_string()),
        Some(options),
        None,
    )
    .await
    .expect("query");

    let init_data = extract_init_message_data(&messages);
    let output_style = init_data
        .get("output_style")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert_eq!(output_style, "default");
    assert_ne!(output_style, "local-test-style");
}

#[tokio::test]
async fn test_e2e_setting_sources_user_only() {
    let temp_project = TempProjectDir::new("settings-user-only");
    temp_project.write_file(
        ".claude/commands/testcmd.md",
        r#"---
description: Test command
---

This is a test command.
"#,
    );

    let mut options = base_options();
    options.cwd = Some(temp_project.path().to_path_buf());
    options.setting_sources = Some(vec![SettingSource::User]);

    let messages = query(
        InputPrompt::Text("What is 2 + 2?".to_string()),
        Some(options),
        None,
    )
    .await
    .expect("query");

    let init_data = extract_init_message_data(&messages);
    let slash_commands = extract_string_list_field(init_data, "slash_commands");
    assert!(!slash_commands.contains(&"testcmd"));
}

#[tokio::test]
async fn test_e2e_setting_sources_project_included() {
    let temp_project = TempProjectDir::new("settings-project-included");
    temp_project.write_file(
        ".claude/settings.local.json",
        r#"{"outputStyle":"local-test-style"}"#,
    );

    let mut options = base_options();
    options.cwd = Some(temp_project.path().to_path_buf());
    options.setting_sources = Some(vec![
        SettingSource::User,
        SettingSource::Project,
        SettingSource::Local,
    ]);

    let messages = query(
        InputPrompt::Text("What is 2 + 2?".to_string()),
        Some(options),
        None,
    )
    .await
    .expect("query");

    let init_data = extract_init_message_data(&messages);
    let output_style = init_data
        .get("output_style")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert_eq!(output_style, "local-test-style");
}

#[tokio::test]
async fn test_e2e_large_agents_via_initialize() {
    let agents = generate_large_agents(20, 13);
    assert!(
        serialized_agents_size(&agents) > 250_000,
        "test fixture must exceed 250KB"
    );

    let mut options = base_options();
    options.agents = Some(agents.clone());

    let mut client = ClaudeSdkClient::new(Some(options), None);
    client.connect(None).await.expect("connect");
    client
        .query(InputPrompt::Text("List available agents".to_string()), "default")
        .await
        .expect("query");

    let messages = client.receive_response().await.expect("receive response");
    let init_data = extract_init_message_data(&messages);

    assert_eq!(
        init_data
            .get("initialize_has_agents")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        init_data
            .get("argv_has_agents_flag")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert!(
        init_data
            .get("initialize_agent_bytes")
            .and_then(Value::as_u64)
            .unwrap_or_default()
            > 250_000
    );

    let init_agents = extract_string_list_field(init_data, "agents");
    for agent_name in agents.keys() {
        assert!(
            init_agents.contains(&agent_name.as_str()),
            "{agent_name} should be registered"
        );
    }

    client.disconnect().await.expect("disconnect");
}
