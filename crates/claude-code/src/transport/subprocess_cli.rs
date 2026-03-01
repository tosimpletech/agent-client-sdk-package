use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::errors::{
    CLIConnectionError, CLIJSONDecodeError, CLINotFoundError, Error, ProcessError, Result,
};
use crate::transport::Transport;
use crate::types::{
    ClaudeAgentOptions, McpServersOption, PermissionMode, SettingSource, SystemPrompt,
    ThinkingConfig, ToolsOption,
};

pub const DEFAULT_MAX_BUFFER_SIZE: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prompt {
    Text(String),
    Messages,
}

#[derive(Debug, Clone)]
pub struct JsonStreamBuffer {
    buffer: String,
    max_buffer_size: usize,
}

impl JsonStreamBuffer {
    pub fn new(max_buffer_size: usize) -> Self {
        Self {
            buffer: String::new(),
            max_buffer_size,
        }
    }

    pub fn push_chunk(
        &mut self,
        chunk: &str,
    ) -> std::result::Result<Vec<Value>, CLIJSONDecodeError> {
        let mut messages = Vec::new();

        for line in chunk.split('\n') {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            self.buffer.push_str(line);
            if self.buffer.len() > self.max_buffer_size {
                let current_size = self.buffer.len();
                self.buffer.clear();
                return Err(CLIJSONDecodeError::new(
                    format!(
                        "JSON message exceeded maximum buffer size of {} bytes",
                        self.max_buffer_size
                    ),
                    format!(
                        "Buffer size {current_size} exceeds limit {}",
                        self.max_buffer_size
                    ),
                ));
            }

            match serde_json::from_str::<Value>(&self.buffer) {
                Ok(value) => {
                    messages.push(value);
                    self.buffer.clear();
                }
                Err(_) => {
                    // Continue buffering partial JSON.
                }
            }
        }

        Ok(messages)
    }
}

pub struct SubprocessCliTransport {
    pub prompt: Prompt,
    pub options: ClaudeAgentOptions,
    pub cli_path: String,
    cwd: Option<PathBuf>,
    child: Option<Child>,
    stdout: Option<BufReader<ChildStdout>>,
    stdin: Option<ChildStdin>,
    ready: bool,
    write_lock: Arc<Mutex<()>>,
    parser: JsonStreamBuffer,
    pending_messages: VecDeque<Value>,
}

impl SubprocessCliTransport {
    pub fn new(prompt: Prompt, options: ClaudeAgentOptions) -> Result<Self> {
        let cli_path = match &options.cli_path {
            Some(path) => path.to_string_lossy().to_string(),
            None => Self::find_cli()?,
        };

        let cwd = options.cwd.clone();
        let max_buffer_size = options.max_buffer_size.unwrap_or(DEFAULT_MAX_BUFFER_SIZE);

        Ok(Self {
            prompt,
            options,
            cli_path,
            cwd,
            child: None,
            stdout: None,
            stdin: None,
            ready: false,
            write_lock: Arc::new(Mutex::new(())),
            parser: JsonStreamBuffer::new(max_buffer_size),
            pending_messages: VecDeque::new(),
        })
    }

    fn find_cli() -> std::result::Result<String, CLINotFoundError> {
        if let Ok(path) = which::which("claude") {
            return Ok(path.to_string_lossy().to_string());
        }

        let locations = vec![
            PathBuf::from(format!(
                "{}/.npm-global/bin/claude",
                std::env::var("HOME").unwrap_or_default()
            )),
            PathBuf::from("/usr/local/bin/claude"),
            PathBuf::from(format!(
                "{}/.local/bin/claude",
                std::env::var("HOME").unwrap_or_default()
            )),
            PathBuf::from(format!(
                "{}/node_modules/.bin/claude",
                std::env::var("HOME").unwrap_or_default()
            )),
            PathBuf::from(format!(
                "{}/.yarn/bin/claude",
                std::env::var("HOME").unwrap_or_default()
            )),
            PathBuf::from(format!(
                "{}/.claude/local/claude",
                std::env::var("HOME").unwrap_or_default()
            )),
        ];

        for path in locations {
            if path.exists() && path.is_file() {
                return Ok(path.to_string_lossy().to_string());
            }
        }

        Err(CLINotFoundError::new(
            "Claude Code not found. Install with:\n  npm install -g @anthropic-ai/claude-code\n\nIf already installed locally, try:\n  export PATH=\"$HOME/node_modules/.bin:$PATH\"\n\nOr provide the path via ClaudeAgentOptions",
            None,
        ))
    }

    fn permission_mode_to_string(mode: &PermissionMode) -> &'static str {
        match mode {
            PermissionMode::Default => "default",
            PermissionMode::AcceptEdits => "acceptEdits",
            PermissionMode::Plan => "plan",
            PermissionMode::BypassPermissions => "bypassPermissions",
        }
    }

    fn setting_source_to_string(source: &SettingSource) -> &'static str {
        match source {
            SettingSource::User => "user",
            SettingSource::Project => "project",
            SettingSource::Local => "local",
        }
    }

    fn build_settings_value(&self) -> Option<String> {
        let has_settings = self.options.settings.is_some();
        let has_sandbox = self.options.sandbox.is_some();

        if !has_settings && !has_sandbox {
            return None;
        }

        if has_settings && !has_sandbox {
            return self.options.settings.clone();
        }

        let mut settings_obj = serde_json::Map::new();

        if let Some(settings) = &self.options.settings {
            let settings_str = settings.trim();
            if settings_str.starts_with('{') && settings_str.ends_with('}') {
                if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(settings_str) {
                    settings_obj = obj;
                }
            } else if Path::new(settings_str).exists()
                && let Ok(content) = std::fs::read_to_string(settings_str)
                && let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(&content)
            {
                settings_obj = obj;
            }
        }

        if let Some(sandbox) = &self.options.sandbox {
            settings_obj.insert(
                "sandbox".to_string(),
                serde_json::to_value(sandbox).unwrap_or(Value::Null),
            );
        }

        Some(Value::Object(settings_obj).to_string())
    }

    pub fn build_command(&self) -> Result<Vec<String>> {
        let mut cmd = vec![
            self.cli_path.clone(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(),
        ];

        match &self.options.system_prompt {
            None => {
                cmd.push("--system-prompt".to_string());
                cmd.push(String::new());
            }
            Some(SystemPrompt::Text(prompt)) => {
                cmd.push("--system-prompt".to_string());
                cmd.push(prompt.clone());
            }
            Some(SystemPrompt::Preset(preset)) => {
                if let Some(append) = &preset.append {
                    cmd.push("--append-system-prompt".to_string());
                    cmd.push(append.clone());
                }
            }
        }

        if let Some(tools) = &self.options.tools {
            match tools {
                ToolsOption::List(list) => {
                    cmd.push("--tools".to_string());
                    if list.is_empty() {
                        cmd.push(String::new());
                    } else {
                        cmd.push(list.join(","));
                    }
                }
                ToolsOption::Preset(_) => {
                    cmd.push("--tools".to_string());
                    cmd.push("default".to_string());
                }
            }
        }

        if !self.options.allowed_tools.is_empty() {
            cmd.push("--allowedTools".to_string());
            cmd.push(self.options.allowed_tools.join(","));
        }

        if let Some(max_turns) = self.options.max_turns {
            cmd.push("--max-turns".to_string());
            cmd.push(max_turns.to_string());
        }

        if let Some(max_budget) = self.options.max_budget_usd {
            cmd.push("--max-budget-usd".to_string());
            cmd.push(max_budget.to_string());
        }

        if !self.options.disallowed_tools.is_empty() {
            cmd.push("--disallowedTools".to_string());
            cmd.push(self.options.disallowed_tools.join(","));
        }

        if let Some(model) = &self.options.model {
            cmd.push("--model".to_string());
            cmd.push(model.clone());
        }

        if let Some(model) = &self.options.fallback_model {
            cmd.push("--fallback-model".to_string());
            cmd.push(model.clone());
        }

        if !self.options.betas.is_empty() {
            cmd.push("--betas".to_string());
            cmd.push(self.options.betas.join(","));
        }

        if let Some(tool_name) = &self.options.permission_prompt_tool_name {
            cmd.push("--permission-prompt-tool".to_string());
            cmd.push(tool_name.clone());
        }

        if let Some(mode) = &self.options.permission_mode {
            cmd.push("--permission-mode".to_string());
            cmd.push(Self::permission_mode_to_string(mode).to_string());
        }

        if self.options.continue_conversation {
            cmd.push("--continue".to_string());
        }

        if let Some(resume) = &self.options.resume {
            cmd.push("--resume".to_string());
            cmd.push(resume.clone());
        }

        if let Some(settings) = self.build_settings_value() {
            cmd.push("--settings".to_string());
            cmd.push(settings);
        }

        for directory in &self.options.add_dirs {
            cmd.push("--add-dir".to_string());
            cmd.push(directory.to_string_lossy().to_string());
        }

        match &self.options.mcp_servers {
            McpServersOption::Servers(servers) => {
                let mut cli_servers = HashMap::new();
                for (name, config) in servers {
                    cli_servers.insert(name.clone(), config.to_cli_json());
                }
                if !cli_servers.is_empty() {
                    cmd.push("--mcp-config".to_string());
                    cmd.push(json!({ "mcpServers": cli_servers }).to_string());
                }
            }
            McpServersOption::Raw(raw) => {
                cmd.push("--mcp-config".to_string());
                cmd.push(raw.clone());
            }
            McpServersOption::None => {}
        }

        if self.options.include_partial_messages {
            cmd.push("--include-partial-messages".to_string());
        }

        if self.options.fork_session {
            cmd.push("--fork-session".to_string());
        }

        let setting_sources = self
            .options
            .setting_sources
            .as_ref()
            .map(|sources| {
                sources
                    .iter()
                    .map(Self::setting_source_to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        cmd.push("--setting-sources".to_string());
        cmd.push(setting_sources);

        for plugin in &self.options.plugins {
            if plugin.type_ != "local" {
                return Err(Error::Other(format!(
                    "Unsupported plugin type: {}",
                    plugin.type_
                )));
            }
            cmd.push("--plugin-dir".to_string());
            cmd.push(plugin.path.clone());
        }

        for (flag, value) in &self.options.extra_args {
            if let Some(v) = value {
                cmd.push(format!("--{flag}"));
                cmd.push(v.clone());
            } else {
                cmd.push(format!("--{flag}"));
            }
        }

        let mut resolved_max_thinking_tokens = self.options.max_thinking_tokens;
        if let Some(thinking) = &self.options.thinking {
            match thinking {
                ThinkingConfig::Adaptive => {
                    if resolved_max_thinking_tokens.is_none() {
                        resolved_max_thinking_tokens = Some(32_000);
                    }
                }
                ThinkingConfig::Enabled { budget_tokens } => {
                    resolved_max_thinking_tokens = Some(*budget_tokens);
                }
                ThinkingConfig::Disabled => {
                    resolved_max_thinking_tokens = Some(0);
                }
            }
        }

        if let Some(tokens) = resolved_max_thinking_tokens {
            cmd.push("--max-thinking-tokens".to_string());
            cmd.push(tokens.to_string());
        }

        if let Some(effort) = &self.options.effort {
            cmd.push("--effort".to_string());
            cmd.push(effort.clone());
        }

        if let Some(Value::Object(output_format)) = &self.options.output_format
            && output_format.get("type").and_then(Value::as_str) == Some("json_schema")
            && let Some(schema) = output_format.get("schema")
        {
            cmd.push("--json-schema".to_string());
            cmd.push(schema.to_string());
        }

        cmd.push("--input-format".to_string());
        cmd.push("stream-json".to_string());

        Ok(cmd)
    }
}

#[async_trait]
impl Transport for SubprocessCliTransport {
    async fn connect(&mut self) -> Result<()> {
        if self.child.is_some() {
            return Ok(());
        }

        if let Some(cwd) = &self.cwd
            && !cwd.exists()
        {
            return Err(CLIConnectionError::new(format!(
                "Working directory does not exist: {}",
                cwd.to_string_lossy()
            ))
            .into());
        }

        let cmd = self.build_command()?;
        let mut command = Command::new(&cmd[0]);
        command.args(&cmd[1..]);
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        if let Some(cwd) = &self.cwd {
            command.current_dir(cwd);
            command.env("PWD", cwd.to_string_lossy().to_string());
        }

        command.env("CLAUDE_CODE_ENTRYPOINT", "sdk-rust");
        command.env("CLAUDE_AGENT_SDK_VERSION", env!("CARGO_PKG_VERSION"));
        for (key, value) in &self.options.env {
            command.env(key, value);
        }

        let mut child = command.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::CLINotFound(CLINotFoundError::new(
                    "Claude Code not found",
                    Some(self.cli_path.clone()),
                ))
            } else {
                Error::CLIConnection(CLIConnectionError::new(format!(
                    "Failed to start Claude Code: {e}"
                )))
            }
        })?;

        let stdout = child.stdout.take().ok_or_else(|| {
            Error::CLIConnection(CLIConnectionError::new(
                "Failed to open stdout for Claude process",
            ))
        })?;

        self.stdin = child.stdin.take();
        self.stdout = Some(BufReader::new(stdout));
        self.child = Some(child);
        self.ready = true;
        Ok(())
    }

    async fn write(&mut self, data: &str) -> Result<()> {
        let _guard = self.write_lock.lock().await;

        if !self.ready {
            return Err(
                CLIConnectionError::new("ProcessTransport is not ready for writing").into(),
            );
        }

        if let Some(child) = &mut self.child
            && let Ok(Some(status)) = child.try_wait()
        {
            return Err(CLIConnectionError::new(format!(
                "Cannot write to terminated process (exit code: {:?})",
                status.code()
            ))
            .into());
        }

        let stdin = self.stdin.as_mut().ok_or_else(|| {
            Error::CLIConnection(CLIConnectionError::new(
                "ProcessTransport is not ready for writing",
            ))
        })?;

        stdin.write_all(data.as_bytes()).await.map_err(|e| {
            Error::CLIConnection(CLIConnectionError::new(format!(
                "Failed to write to process stdin: {e}"
            )))
        })?;
        stdin.flush().await.map_err(|e| {
            Error::CLIConnection(CLIConnectionError::new(format!(
                "Failed to flush process stdin: {e}"
            )))
        })?;

        Ok(())
    }

    async fn end_input(&mut self) -> Result<()> {
        let _guard = self.write_lock.lock().await;
        self.stdin.take();
        Ok(())
    }

    async fn read_next_message(&mut self) -> Result<Option<Value>> {
        if let Some(message) = self.pending_messages.pop_front() {
            return Ok(Some(message));
        }

        if self.child.is_none() || self.stdout.is_none() {
            return Err(CLIConnectionError::new("Not connected").into());
        }

        let stdout = self.stdout.as_mut().expect("checked is_some");

        loop {
            let mut line = String::new();
            let bytes_read = stdout.read_line(&mut line).await?;
            if bytes_read == 0 {
                break;
            }

            let parsed = self.parser.push_chunk(&line)?;
            for message in parsed {
                self.pending_messages.push_back(message);
            }
            if let Some(message) = self.pending_messages.pop_front() {
                return Ok(Some(message));
            }
        }

        self.ready = false;
        if let Some(child) = &mut self.child {
            let status = child.wait().await.map_err(|e| {
                Error::Process(ProcessError::new(
                    format!("Failed to wait for process completion: {e}"),
                    None,
                    None,
                ))
            })?;
            if !status.success() {
                return Err(ProcessError::new(
                    "Command failed",
                    status.code(),
                    Some("Check stderr output for details".to_string()),
                )
                .into());
            }
        }
        Ok(None)
    }

    async fn close(&mut self) -> Result<()> {
        self.ready = false;
        self.stdin.take();
        self.stdout.take();
        if let Some(child) = &mut self.child
            && child.try_wait()?.is_none()
        {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        self.child = None;
        Ok(())
    }

    fn is_ready(&self) -> bool {
        self.ready
    }
}
