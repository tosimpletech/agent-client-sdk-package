use std::collections::HashMap;
use std::path::PathBuf;

use claude_code::{
    ClaudeAgentOptions, McpServerConfig, McpServersOption, McpStdioServerConfig, PermissionMode,
    SandboxNetworkConfig, SandboxSettings, SettingSource, SubprocessCliTransport, SystemPrompt,
    SystemPromptPreset, ThinkingConfig, ToolsOption, ToolsPreset,
};
use serde_json::Value;

fn make_options() -> ClaudeAgentOptions {
    ClaudeAgentOptions {
        cli_path: Some(PathBuf::from("/usr/bin/claude")),
        ..Default::default()
    }
}

#[test]
fn test_build_command_basic() {
    let options = make_options();
    let transport =
        SubprocessCliTransport::new(claude_code::Prompt::Text("Hello".to_string()), options)
            .expect("transport");
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
fn test_build_command_system_prompt_variants() {
    let mut options = make_options();
    options.system_prompt = Some(SystemPrompt::Text("Be helpful".to_string()));
    let transport =
        SubprocessCliTransport::new(claude_code::Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");
    assert!(cmd.contains(&"--system-prompt".to_string()));
    assert!(cmd.contains(&"Be helpful".to_string()));

    let mut options = make_options();
    options.system_prompt = Some(SystemPrompt::Preset(SystemPromptPreset::default()));
    let transport =
        SubprocessCliTransport::new(claude_code::Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");
    assert!(!cmd.contains(&"--append-system-prompt".to_string()));

    let mut options = make_options();
    options.system_prompt = Some(SystemPrompt::Preset(SystemPromptPreset {
        append: Some("Be concise.".to_string()),
        ..Default::default()
    }));
    let transport =
        SubprocessCliTransport::new(claude_code::Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");
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
    options.fallback_model = Some("sonnet".to_string());
    options.max_thinking_tokens = Some(5000);

    let transport =
        SubprocessCliTransport::new(claude_code::Prompt::Messages, options).expect("transport");
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
    assert!(cmd.contains(&"--fallback-model".to_string()));
    assert!(cmd.contains(&"sonnet".to_string()));
    assert!(cmd.contains(&"--max-thinking-tokens".to_string()));
    assert!(cmd.contains(&"5000".to_string()));
}

#[test]
fn test_build_command_tools_variants() {
    let mut options = make_options();
    options.tools = Some(ToolsOption::List(vec![
        "Read".to_string(),
        "Edit".to_string(),
        "Bash".to_string(),
    ]));
    let transport =
        SubprocessCliTransport::new(claude_code::Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");
    let idx = cmd.iter().position(|x| x == "--tools").expect("tools flag");
    assert_eq!(cmd[idx + 1], "Read,Edit,Bash");

    let mut options = make_options();
    options.tools = Some(ToolsOption::List(vec![]));
    let transport =
        SubprocessCliTransport::new(claude_code::Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");
    let idx = cmd.iter().position(|x| x == "--tools").expect("tools flag");
    assert_eq!(cmd[idx + 1], "");

    let mut options = make_options();
    options.tools = Some(ToolsOption::Preset(ToolsPreset::default()));
    let transport =
        SubprocessCliTransport::new(claude_code::Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");
    let idx = cmd.iter().position(|x| x == "--tools").expect("tools flag");
    assert_eq!(cmd[idx + 1], "default");
}

#[test]
fn test_build_command_with_sandbox_and_settings_merge() {
    let mut options = make_options();
    options.settings = Some("{\"permissions\": {\"allow\": [\"Bash(ls:*)\"]}}".to_string());
    options.sandbox = Some(SandboxSettings {
        enabled: Some(true),
        auto_allow_bash_if_sandboxed: Some(true),
        excluded_commands: Some(vec!["git".to_string(), "docker".to_string()]),
        allow_unsandboxed_commands: None,
        network: Some(SandboxNetworkConfig {
            allow_local_binding: Some(true),
            allow_unix_sockets: Some(vec!["/var/run/docker.sock".to_string()]),
            ..Default::default()
        }),
        ..Default::default()
    });

    let transport =
        SubprocessCliTransport::new(claude_code::Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");
    let idx = cmd
        .iter()
        .position(|x| x == "--settings")
        .expect("settings");
    let parsed: Value = serde_json::from_str(&cmd[idx + 1]).expect("json");

    assert_eq!(parsed["permissions"]["allow"][0], "Bash(ls:*)");
    assert_eq!(parsed["sandbox"]["enabled"], true);
    assert_eq!(parsed["sandbox"]["autoAllowBashIfSandboxed"], true);
    assert_eq!(parsed["sandbox"]["network"]["allowLocalBinding"], true);
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

    let transport =
        SubprocessCliTransport::new(claude_code::Prompt::Messages, options).expect("transport");
    let cmd = transport.build_command().expect("command");

    assert!(cmd.contains(&"--mcp-config".to_string()));
    let idx = cmd.iter().position(|x| x == "--mcp-config").expect("mcp");
    let parsed: Value = serde_json::from_str(&cmd[idx + 1]).expect("json");
    assert_eq!(
        parsed["mcpServers"]["test-server"]["command"],
        "/path/to/server"
    );
    assert!(cmd.contains(&"--setting-sources".to_string()));
    assert!(cmd.contains(&"user,project".to_string()));
    assert!(cmd.contains(&"--max-thinking-tokens".to_string()));
    assert!(cmd.contains(&"32000".to_string()));
}
