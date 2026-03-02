use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use claude_code::{
    AgentDefinition, ClaudeAgentOptions, Error, McpServerConfig, McpServersOption,
    McpStdioServerConfig, PermissionMode, Prompt, SandboxNetworkConfig, SandboxSettings,
    SettingSource, SubprocessCliTransport, SystemPrompt, SystemPromptPreset, ThinkingConfig,
    ToolsOption, ToolsPreset, Transport,
};
use serde_json::Value;
use tokio::sync::Mutex;

fn make_options() -> ClaudeAgentOptions {
    ClaudeAgentOptions {
        cli_path: Some(PathBuf::from("/usr/bin/claude")),
        ..Default::default()
    }
}

fn fixture_cli_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock_claude_cli.py")
}

fn make_connect_options() -> ClaudeAgentOptions {
    ClaudeAgentOptions {
        cli_path: Some(fixture_cli_path()),
        ..Default::default()
    }
}

fn flag_value(cmd: &[String], flag: &str) -> String {
    let idx = cmd
        .iter()
        .position(|arg| arg == flag)
        .expect("flag present");
    cmd[idx + 1].clone()
}

fn parse_settings_flag(cmd: &[String]) -> Value {
    serde_json::from_str(&flag_value(cmd, "--settings")).expect("settings json")
}

#[tokio::test]
async fn test_find_cli_not_found() {
    let mut options = make_options();
    options.cli_path = Some(PathBuf::from(
        "/definitely/not/a/real/claude-code-binary-transport-test",
    ));

    let mut transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let err = transport.connect().await.expect_err("must fail");

    assert!(matches!(err, Error::CLINotFound(_)));
    assert!(err.to_string().contains("Claude Code not found"));
}

#[test]
fn test_build_command_basic() {
    let options = make_options();
    let transport =
        SubprocessCliTransport::new(Prompt::Text("Hello".to_string()), options).expect("transport");
    let cmd = transport.build_command().expect("command");

    assert_eq!(cmd[0], "/usr/bin/claude");
    assert!(cmd.contains(&"--output-format".to_string()));
    assert!(cmd.contains(&"stream-json".to_string()));
    assert!(cmd.contains(&"--input-format".to_string()));
    assert!(!cmd.contains(&"--print".to_string()));
    assert!(cmd.contains(&"--system-prompt".to_string()));
    let idx = cmd
        .iter()
        .position(|x| x == "--system-prompt")
        .expect("idx");
    assert_eq!(cmd[idx + 1], "");
}

#[test]
fn test_cli_path_accepts_pathbuf_path() {
    let options = ClaudeAgentOptions {
        cli_path: Some(PathBuf::from("/usr/bin/claude")),
        ..Default::default()
    };
    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    assert_eq!(transport.cli_path, "/usr/bin/claude");
}

#[test]
fn test_build_command_with_system_prompt_string() {
    let mut options = make_options();
    options.system_prompt = Some(SystemPrompt::Text("Be helpful".to_string()));
    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");

    assert!(cmd.contains(&"--system-prompt".to_string()));
    assert!(cmd.contains(&"Be helpful".to_string()));
}

#[test]
fn test_build_command_with_system_prompt_preset() {
    let mut options = make_options();
    options.system_prompt = Some(SystemPrompt::Preset(SystemPromptPreset::default()));
    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");

    assert!(!cmd.contains(&"--system-prompt".to_string()));
    assert!(!cmd.contains(&"--append-system-prompt".to_string()));
}

#[test]
fn test_build_command_with_system_prompt_preset_and_append() {
    let mut options = make_options();
    options.system_prompt = Some(SystemPrompt::Preset(SystemPromptPreset {
        append: Some("Be concise.".to_string()),
        ..Default::default()
    }));
    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");

    assert!(!cmd.contains(&"--system-prompt".to_string()));
    assert!(cmd.contains(&"--append-system-prompt".to_string()));
    assert!(cmd.contains(&"Be concise.".to_string()));
}

#[test]
fn test_build_command_with_options() {
    let mut options = make_options();
    options.allowed_tools = vec!["Read".to_string(), "Write".to_string()];
    options.disallowed_tools = vec!["Bash".to_string()];
    options.model = Some("claude-sonnet-4-5".to_string());
    options.permission_mode = Some(PermissionMode::AcceptEdits);
    options.max_turns = Some(5);

    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");

    assert!(cmd.contains(&"--allowedTools".to_string()));
    assert!(cmd.contains(&"Read,Write".to_string()));
    assert!(cmd.contains(&"--disallowedTools".to_string()));
    assert!(cmd.contains(&"Bash".to_string()));
    assert!(cmd.contains(&"--model".to_string()));
    assert!(cmd.contains(&"claude-sonnet-4-5".to_string()));
    assert!(cmd.contains(&"--permission-mode".to_string()));
    assert!(cmd.contains(&"acceptEdits".to_string()));
    assert!(cmd.contains(&"--max-turns".to_string()));
    assert!(cmd.contains(&"5".to_string()));
}

#[test]
fn test_build_command_with_fallback_model() {
    let mut options = make_options();
    options.model = Some("opus".to_string());
    options.fallback_model = Some("sonnet".to_string());

    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");

    assert!(cmd.contains(&"--model".to_string()));
    assert!(cmd.contains(&"opus".to_string()));
    assert!(cmd.contains(&"--fallback-model".to_string()));
    assert!(cmd.contains(&"sonnet".to_string()));
}

#[test]
fn test_build_command_with_max_thinking_tokens() {
    let mut options = make_options();
    options.max_thinking_tokens = Some(5000);

    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");

    assert!(cmd.contains(&"--max-thinking-tokens".to_string()));
    assert!(cmd.contains(&"5000".to_string()));
}

#[test]
fn test_build_command_with_add_dirs() {
    let mut options = make_options();
    options.add_dirs = vec![
        PathBuf::from("/path/to/dir1"),
        PathBuf::from("/path/to/dir2"),
    ];

    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");

    let add_dir_indices: Vec<usize> = cmd
        .iter()
        .enumerate()
        .filter_map(|(i, arg)| (arg == "--add-dir").then_some(i))
        .collect();
    assert_eq!(add_dir_indices.len(), 2);

    let dirs_in_cmd: Vec<String> = add_dir_indices.iter().map(|i| cmd[i + 1].clone()).collect();
    assert!(dirs_in_cmd.contains(&"/path/to/dir1".to_string()));
    assert!(dirs_in_cmd.contains(&"/path/to/dir2".to_string()));
}

#[test]
fn test_session_continuation() {
    let mut options = make_options();
    options.continue_conversation = true;
    options.resume = Some("session-123".to_string());

    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");

    assert!(cmd.contains(&"--continue".to_string()));
    assert!(cmd.contains(&"--resume".to_string()));
    assert!(cmd.contains(&"session-123".to_string()));
}

#[tokio::test]
async fn test_connect_close() {
    let options = make_connect_options();
    let mut transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");

    transport.connect().await.expect("connect");
    assert!(transport.is_ready());

    transport.close().await.expect("close");
    assert!(!transport.is_ready());
}

#[test]
fn test_read_messages_transport_creation() {
    let transport = SubprocessCliTransport::new(Prompt::Text("test".to_string()), make_options())
        .expect("transport");
    assert_eq!(transport.cli_path, "/usr/bin/claude");
    assert!(matches!(transport.prompt, Prompt::Text(_)));
}

#[tokio::test]
async fn test_connect_with_nonexistent_cwd() {
    let mut options = make_connect_options();
    options.cwd = Some(PathBuf::from("/this/directory/does/not/exist"));

    let mut transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let err = transport.connect().await.expect_err("must fail");
    assert!(err.to_string().contains("/this/directory/does/not/exist"));
}

#[test]
fn test_build_command_with_settings_file() {
    let mut options = make_options();
    options.settings = Some("/path/to/settings.json".to_string());

    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");

    assert!(cmd.contains(&"--settings".to_string()));
    assert_eq!(flag_value(&cmd, "--settings"), "/path/to/settings.json");
}

#[test]
fn test_build_command_with_settings_json() {
    let settings_json = "{\"permissions\":{\"allow\":[\"Bash(ls:*)\"]}}".to_string();
    let mut options = make_options();
    options.settings = Some(settings_json.clone());

    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");

    assert!(cmd.contains(&"--settings".to_string()));
    assert_eq!(flag_value(&cmd, "--settings"), settings_json);
}

#[test]
fn test_build_command_with_extra_args() {
    let mut options = make_options();
    options
        .extra_args
        .insert("new-flag".to_string(), Some("value".to_string()));
    options.extra_args.insert("boolean-flag".to_string(), None);
    options
        .extra_args
        .insert("another-option".to_string(), Some("test-value".to_string()));

    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");

    assert!(cmd.contains(&"--new-flag".to_string()));
    assert!(cmd.contains(&"value".to_string()));
    assert!(cmd.contains(&"--another-option".to_string()));
    assert!(cmd.contains(&"test-value".to_string()));
    assert!(cmd.contains(&"--boolean-flag".to_string()));

    let boolean_idx = cmd
        .iter()
        .position(|arg| arg == "--boolean-flag")
        .expect("boolean flag");
    assert!(boolean_idx == cmd.len() - 1 || cmd[boolean_idx + 1].starts_with("--"));
}

#[test]
fn test_build_command_with_mcp_servers() {
    let mut servers = HashMap::new();
    servers.insert(
        "test-server".to_string(),
        McpServerConfig::Stdio(McpStdioServerConfig {
            type_: Some("stdio".to_string()),
            command: "/path/to/server".to_string(),
            args: Some(vec!["--option".to_string(), "value".to_string()]),
            env: None,
        }),
    );

    let mut options = make_options();
    options.mcp_servers = McpServersOption::Servers(servers);
    options.setting_sources = Some(vec![SettingSource::User, SettingSource::Project]);
    options.thinking = Some(ThinkingConfig::Adaptive);

    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");

    assert!(cmd.contains(&"--mcp-config".to_string()));
    let parsed: Value = serde_json::from_str(&flag_value(&cmd, "--mcp-config")).expect("json");
    assert_eq!(
        parsed["mcpServers"]["test-server"]["command"],
        "/path/to/server"
    );
    assert!(cmd.contains(&"--setting-sources".to_string()));
    assert!(cmd.contains(&"user,project".to_string()));
    assert!(cmd.contains(&"--max-thinking-tokens".to_string()));
    assert!(cmd.contains(&"32000".to_string()));
}

#[test]
fn test_build_command_with_mcp_servers_as_file_path() {
    let mut options = make_options();
    options.mcp_servers = McpServersOption::Raw("/path/to/mcp-config.json".to_string());

    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");

    assert!(cmd.contains(&"--mcp-config".to_string()));
    assert_eq!(flag_value(&cmd, "--mcp-config"), "/path/to/mcp-config.json");
}

#[test]
fn test_build_command_with_mcp_servers_as_json_string() {
    let json_config =
        "{\"mcpServers\":{\"server\":{\"type\":\"stdio\",\"command\":\"test\"}}}".to_string();
    let mut options = make_options();
    options.mcp_servers = McpServersOption::Raw(json_config.clone());

    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");

    assert!(cmd.contains(&"--mcp-config".to_string()));
    assert_eq!(flag_value(&cmd, "--mcp-config"), json_config);
}

#[tokio::test]
async fn test_env_vars_passed_to_subprocess() {
    let mut options = make_connect_options();
    options
        .env
        .insert("MOCK_CLAUDE_TRIGGER_MCP".to_string(), "1".to_string());

    let mut transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    transport.connect().await.expect("connect");

    transport
        .write(
            r#"{"type":"control_request","request_id":"req-env","request":{"subtype":"initialize"}}"#
                .to_string()
                .as_str(),
        )
        .await
        .expect("write initialize");
    transport.write("\n").await.expect("write newline");

    let first = tokio::time::timeout(Duration::from_secs(1), transport.read_next_message())
        .await
        .expect("first message timeout")
        .expect("first message read")
        .expect("first message");
    assert_eq!(first["type"], "control_response");

    let second = tokio::time::timeout(Duration::from_secs(1), transport.read_next_message())
        .await
        .expect("second message timeout")
        .expect("second message read")
        .expect("second message");
    assert_eq!(second["type"], "control_request");
    assert_eq!(second["request"]["subtype"], "mcp_message");

    transport.close().await.expect("close");
}

#[test]
fn test_build_command_with_sandbox_only() {
    let mut options = make_options();
    options.sandbox = Some(SandboxSettings {
        enabled: Some(true),
        auto_allow_bash_if_sandboxed: Some(true),
        network: Some(SandboxNetworkConfig {
            allow_local_binding: Some(true),
            allow_unix_sockets: Some(vec!["/var/run/docker.sock".to_string()]),
            ..Default::default()
        }),
        ..Default::default()
    });

    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");
    let parsed = parse_settings_flag(&cmd);

    assert_eq!(parsed["sandbox"]["enabled"], true);
    assert_eq!(parsed["sandbox"]["autoAllowBashIfSandboxed"], true);
    assert_eq!(parsed["sandbox"]["network"]["allowLocalBinding"], true);
    assert_eq!(
        parsed["sandbox"]["network"]["allowUnixSockets"][0],
        "/var/run/docker.sock"
    );
}

#[test]
fn test_build_command_with_sandbox_and_settings_json() {
    let mut options = make_options();
    options.settings =
        Some("{\"permissions\":{\"allow\":[\"Bash(ls:*)\"]},\"verbose\":true}".to_string());
    options.sandbox = Some(SandboxSettings {
        enabled: Some(true),
        excluded_commands: Some(vec!["git".to_string(), "docker".to_string()]),
        ..Default::default()
    });

    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");
    let parsed = parse_settings_flag(&cmd);

    assert_eq!(parsed["permissions"]["allow"][0], "Bash(ls:*)");
    assert_eq!(parsed["verbose"], true);
    assert_eq!(parsed["sandbox"]["enabled"], true);
    assert_eq!(parsed["sandbox"]["excludedCommands"][0], "git");
    assert_eq!(parsed["sandbox"]["excludedCommands"][1], "docker");
}

#[test]
fn test_build_command_with_settings_file_and_no_sandbox() {
    let mut options = make_options();
    options.settings = Some("/path/to/settings.json".to_string());

    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");

    assert!(cmd.contains(&"--settings".to_string()));
    assert_eq!(flag_value(&cmd, "--settings"), "/path/to/settings.json");
}

#[test]
fn test_build_command_with_sandbox_minimal() {
    let mut options = make_options();
    options.sandbox = Some(SandboxSettings {
        enabled: Some(true),
        ..Default::default()
    });

    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");
    let parsed = parse_settings_flag(&cmd);
    assert_eq!(parsed, serde_json::json!({"sandbox": {"enabled": true}}));
}

#[test]
fn test_sandbox_network_config() {
    let mut options = make_options();
    options.sandbox = Some(SandboxSettings {
        enabled: Some(true),
        network: Some(SandboxNetworkConfig {
            allow_unix_sockets: Some(vec!["/tmp/ssh-agent.sock".to_string()]),
            allow_all_unix_sockets: Some(false),
            allow_local_binding: Some(true),
            http_proxy_port: Some(8080),
            socks_proxy_port: Some(8081),
        }),
        ..Default::default()
    });

    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");
    let parsed = parse_settings_flag(&cmd);
    let network = &parsed["sandbox"]["network"];

    assert_eq!(network["allowUnixSockets"][0], "/tmp/ssh-agent.sock");
    assert_eq!(network["allowAllUnixSockets"], false);
    assert_eq!(network["allowLocalBinding"], true);
    assert_eq!(network["httpProxyPort"], 8080);
    assert_eq!(network["socksProxyPort"], 8081);
}

#[test]
fn test_build_command_with_tools_array() {
    let mut options = make_options();
    options.tools = Some(ToolsOption::List(vec![
        "Read".to_string(),
        "Edit".to_string(),
        "Bash".to_string(),
    ]));

    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");
    assert_eq!(flag_value(&cmd, "--tools"), "Read,Edit,Bash");
}

#[test]
fn test_build_command_with_tools_empty_array() {
    let mut options = make_options();
    options.tools = Some(ToolsOption::List(vec![]));

    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");
    assert_eq!(flag_value(&cmd, "--tools"), "");
}

#[test]
fn test_build_command_with_tools_preset() {
    let mut options = make_options();
    options.tools = Some(ToolsOption::Preset(ToolsPreset::default()));

    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");
    assert_eq!(flag_value(&cmd, "--tools"), "default");
}

#[test]
fn test_build_command_without_tools() {
    let transport =
        SubprocessCliTransport::new(Prompt::Messages, make_options()).expect("transport");
    let cmd = transport.build_command().expect("command");
    assert!(!cmd.contains(&"--tools".to_string()));
}

#[test]
fn test_settings_merge_failure_falls_back_when_not_strict() {
    let mut options = make_options();
    options.settings = Some("/not/found/settings.json".to_string());
    options.sandbox = Some(SandboxSettings {
        enabled: Some(true),
        ..Default::default()
    });
    options.strict_settings_merge = false;

    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");
    let parsed = parse_settings_flag(&cmd);
    assert_eq!(parsed, serde_json::json!({"sandbox": {"enabled": true}}));
}

#[test]
fn test_settings_merge_failure_errors_in_strict_mode() {
    let mut options = make_options();
    options.settings = Some("/not/found/settings.json".to_string());
    options.sandbox = Some(SandboxSettings {
        enabled: Some(true),
        ..Default::default()
    });
    options.strict_settings_merge = true;

    let transport = SubprocessCliTransport::new(Prompt::Messages, options).expect("transport");
    let err = transport
        .build_command()
        .expect_err("strict mode must fail");
    assert!(
        err.to_string()
            .contains("Failed to merge settings into sandbox config")
    );
}

#[tokio::test]
async fn test_concurrent_writes_are_serialized() {
    let mut transport =
        SubprocessCliTransport::new(Prompt::Messages, make_connect_options()).expect("transport");
    transport.connect().await.expect("connect");

    let (_reader, writer, close_handle) = Box::new(transport).into_split().expect("split");
    let writer = Arc::new(Mutex::new(writer));

    let mut tasks = Vec::new();
    for i in 0..10 {
        let writer = writer.clone();
        tasks.push(tokio::spawn(async move {
            let mut writer = writer.lock().await;
            writer
                .write(format!("{{\"type\":\"user\",\"message\":{i}}}\n").as_str())
                .await
        }));
    }

    for task in tasks {
        task.await.expect("task join").expect("write");
    }

    close_handle.close().await.expect("close");
}

#[tokio::test]
async fn test_concurrent_writes_fail_without_lock() {
    struct UnlockedWriteProbe {
        in_flight: AtomicBool,
    }

    impl UnlockedWriteProbe {
        async fn write(&self) -> std::result::Result<(), String> {
            if self.in_flight.swap(true, Ordering::SeqCst) {
                return Err("another task is already writing".to_string());
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
            self.in_flight.store(false, Ordering::SeqCst);
            Ok(())
        }
    }

    let probe = Arc::new(UnlockedWriteProbe {
        in_flight: AtomicBool::new(false),
    });
    let errors = Arc::new(Mutex::new(Vec::<String>::new()));

    let mut tasks = Vec::new();
    for _ in 0..20 {
        let probe = probe.clone();
        let errors = errors.clone();
        tasks.push(tokio::spawn(async move {
            if let Err(err) = probe.write().await {
                errors.lock().await.push(err);
            }
        }));
    }

    for task in tasks {
        task.await.expect("task join");
    }

    let errors = errors.lock().await;
    assert!(!errors.is_empty(), "expected concurrent write errors");
    assert!(errors.iter().any(|msg| msg.contains("another task")));
}

#[test]
fn test_build_command_agents_always_via_initialize() {
    let mut agents = HashMap::new();
    agents.insert(
        "test-agent".to_string(),
        AgentDefinition {
            description: "A test agent".to_string(),
            prompt: "You are a test agent".to_string(),
            tools: None,
            model: None,
        },
    );

    let mut text_options = make_options();
    text_options.agents = Some(agents.clone());
    let text_transport =
        SubprocessCliTransport::new(Prompt::Text("Hello".to_string()), text_options)
            .expect("transport");
    let text_cmd = text_transport.build_command().expect("command");
    assert!(!text_cmd.contains(&"--agents".to_string()));
    assert!(text_cmd.contains(&"--input-format".to_string()));
    assert!(text_cmd.contains(&"stream-json".to_string()));

    let mut streaming_options = make_options();
    streaming_options.agents = Some(agents);
    let streaming_transport =
        SubprocessCliTransport::new(Prompt::Messages, streaming_options).expect("transport");
    let streaming_cmd = streaming_transport.build_command().expect("command");
    assert!(!streaming_cmd.contains(&"--agents".to_string()));
    assert!(streaming_cmd.contains(&"--input-format".to_string()));
    assert!(streaming_cmd.contains(&"stream-json".to_string()));
}

#[test]
fn test_build_command_always_uses_streaming() {
    let transport = SubprocessCliTransport::new(Prompt::Text("Hello".to_string()), make_options())
        .expect("transport");
    let cmd = transport.build_command().expect("command");
    assert!(cmd.contains(&"--input-format".to_string()));
    assert!(cmd.contains(&"stream-json".to_string()));
    assert!(!cmd.contains(&"--print".to_string()));
}

#[test]
fn test_build_command_large_agents_work() {
    let mut agents = HashMap::new();
    agents.insert(
        "large-agent".to_string(),
        AgentDefinition {
            description: "A large agent".to_string(),
            prompt: "x".repeat(50_000),
            tools: None,
            model: None,
        },
    );

    let mut options = make_options();
    options.agents = Some(agents);

    let transport =
        SubprocessCliTransport::new(Prompt::Text("Hello".to_string()), options).expect("transport");
    let cmd = transport.build_command().expect("command");

    assert!(!cmd.contains(&"--agents".to_string()));
    assert!(!cmd.join(" ").contains('@'));
}
