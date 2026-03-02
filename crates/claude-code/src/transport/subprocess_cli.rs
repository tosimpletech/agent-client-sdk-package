//! Subprocess-based transport for the Claude Code CLI.
//!
//! This module provides [`SubprocessCliTransport`], which spawns the Claude Code CLI
//! as a child process and communicates via stdin/stdout using newline-delimited JSON.
//!
//! It also provides [`JsonStreamBuffer`] for incrementally parsing JSON messages
//! from a byte stream.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

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
    ClaudeAgentOptions, McpServersOption, PermissionMode, SettingSource, StderrCallback,
    SystemPrompt, ThinkingConfig, ToolsOption,
};

/// Default maximum buffer size for JSON stream parsing (1 MB).
pub const DEFAULT_MAX_BUFFER_SIZE: usize = 1024 * 1024;
const MINIMUM_CLAUDE_CODE_VERSION: &str = "2.0.0";

/// Prompt type for the transport layer.
///
/// Determines whether the CLI is invoked with a text prompt on the command line
/// or in streaming message mode (input via stdin).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prompt {
    /// A text prompt passed as a CLI argument.
    Text(String),
    /// Streaming message mode — input is provided via stdin as JSON messages.
    Messages,
}

/// Incremental JSON stream parser for buffering and parsing newline-delimited JSON.
///
/// Accumulates input chunks and attempts to parse complete JSON values.
/// Handles cases where JSON objects span multiple lines or chunks.
///
/// # Buffer overflow protection
///
/// If the buffer exceeds `max_buffer_size` bytes, a [`CLIJSONDecodeError`] is returned
/// and the buffer is cleared.
#[derive(Debug, Clone)]
pub struct JsonStreamBuffer {
    buffer: String,
    max_buffer_size: usize,
}

impl JsonStreamBuffer {
    /// Creates a new `JsonStreamBuffer` with the given maximum buffer size.
    pub fn new(max_buffer_size: usize) -> Self {
        Self {
            buffer: String::new(),
            max_buffer_size,
        }
    }

    /// Pushes a chunk of data into the buffer and returns any complete JSON values.
    ///
    /// The chunk is split by newlines, and each line is appended to the internal buffer.
    /// After each line, the buffer is tested for valid JSON. If it parses successfully,
    /// the value is collected and the buffer is cleared for the next message.
    ///
    /// # Returns
    ///
    /// A `Vec<Value>` of all complete JSON values parsed from this chunk.
    ///
    /// # Errors
    ///
    /// Returns [`CLIJSONDecodeError`] if the buffer exceeds the maximum size.
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

/// Transport implementation that communicates with the Claude Code CLI via a subprocess.
///
/// Spawns the `claude` CLI as a child process, passing configuration via command-line
/// arguments and environment variables. Communication uses newline-delimited JSON
/// over stdin (input) and stdout (output).
///
/// # CLI discovery
///
/// The CLI binary is located by:
/// 1. Using `cli_path` from [`ClaudeAgentOptions`] if provided
/// 2. Searching `PATH` for `claude`
/// 3. Checking common installation locations (`~/.npm-global/bin/`, `/usr/local/bin/`, etc.)
pub struct SubprocessCliTransport {
    /// The prompt type for this transport session.
    pub prompt: Prompt,
    /// The agent options used to configure the CLI.
    pub options: ClaudeAgentOptions,
    /// The resolved path to the CLI executable.
    pub cli_path: String,
    cwd: Option<PathBuf>,
    child: Option<Child>,
    stdout: Option<BufReader<ChildStdout>>,
    stdin: Option<ChildStdin>,
    ready: bool,
    write_lock: Arc<Mutex<()>>,
    parser: JsonStreamBuffer,
    pending_messages: VecDeque<Value>,
    /// Handle for the background task that drains stderr to prevent pipe blocking.
    stderr_task: Option<tokio::task::JoinHandle<()>>,
    /// Optional stderr callback to receive line output.
    stderr_callback: Option<StderrCallback>,
}

impl SubprocessCliTransport {
    /// Creates a new `SubprocessCliTransport` with the given prompt and options.
    ///
    /// Resolves the CLI path immediately but does not start the subprocess.
    /// Call [`connect()`](Transport::connect) to spawn the process.
    ///
    /// # Errors
    ///
    /// Returns [`CLINotFoundError`] if the CLI executable cannot be located.
    pub fn new(prompt: Prompt, options: ClaudeAgentOptions) -> Result<Self> {
        let cli_path = match &options.cli_path {
            Some(path) => path.to_string_lossy().to_string(),
            None => Self::find_cli()?,
        };

        let cwd = options.cwd.clone();
        let max_buffer_size = options.max_buffer_size.unwrap_or(DEFAULT_MAX_BUFFER_SIZE);
        let stderr_callback = options.stderr.clone();

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
            stderr_task: None,
            stderr_callback,
        })
    }

    /// Locates the Claude Code CLI binary by searching PATH and common locations.
    fn find_cli() -> std::result::Result<String, CLINotFoundError> {
        if let Some(path) = Self::find_bundled_cli() {
            return Ok(path);
        }

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

    /// Attempts to locate a bundled Claude Code CLI binary.
    fn find_bundled_cli() -> Option<String> {
        if let Ok(path) = std::env::var("CLAUDE_CODE_BUNDLED_CLI") {
            let candidate = PathBuf::from(path);
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }

        let cli_name = if cfg!(windows) {
            "claude.exe"
        } else {
            "claude"
        };
        let mut candidates = Vec::new();
        if let Ok(current_exe) = std::env::current_exe()
            && let Some(exe_dir) = current_exe.parent()
        {
            candidates.push(exe_dir.join("_bundled").join(cli_name));
            candidates.push(exe_dir.join("..").join("_bundled").join(cli_name));
        }

        for candidate in candidates {
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
        None
    }

    /// Resolves a user identifier (username string or numeric UID) to a Unix UID.
    ///
    /// Supports both numeric UIDs (e.g., `"1000"`) and username strings (e.g., `"nobody"`).
    #[cfg(unix)]
    fn resolve_user_to_uid(user: &str) -> Result<u32> {
        // Try parsing as numeric UID first.
        if let Ok(uid) = user.parse::<u32>() {
            return Ok(uid);
        }

        // Look up username via thread-safe getpwnam_r.
        use std::ffi::CString;
        use std::ptr;

        let c_user = CString::new(user)
            .map_err(|_| Error::Other(format!("Invalid user name (contains null byte): {user}")))?;

        let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result: *mut libc::passwd = ptr::null_mut();
        let mut buffer_len = {
            let configured = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
            if configured <= 0 {
                16 * 1024
            } else {
                configured as usize
            }
        };

        loop {
            let mut buffer = vec![0u8; buffer_len];
            let rc = unsafe {
                libc::getpwnam_r(
                    c_user.as_ptr(),
                    &mut pwd,
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &mut result,
                )
            };

            if rc == 0 {
                if result.is_null() {
                    return Err(Error::Other(format!("User not found: {user}")));
                }
                return Ok(pwd.pw_uid);
            }

            if rc == libc::ERANGE {
                buffer_len = buffer_len.saturating_mul(2);
                if buffer_len > 1024 * 1024 {
                    return Err(Error::Other(format!(
                        "Failed to resolve user '{user}': lookup buffer exceeded 1MB"
                    )));
                }
                continue;
            }

            return Err(Error::Other(format!(
                "Failed to resolve user '{user}': {}",
                std::io::Error::from_raw_os_error(rc)
            )));
        }
    }

    fn parse_semver_prefix(version: &str) -> Option<[u32; 3]> {
        let mut parts = [0u32; 3];
        let mut iter = version.trim().split('.');
        for slot in &mut parts {
            let component = iter.next()?;
            let digits: String = component
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect();
            if digits.is_empty() {
                return None;
            }
            *slot = digits.parse().ok()?;
        }
        Some(parts)
    }

    async fn check_claude_version(&self) {
        if std::env::var("CLAUDE_AGENT_SDK_SKIP_VERSION_CHECK").is_ok() {
            return;
        }

        let mut command = Command::new(&self.cli_path);
        command.arg("-v");
        command.stdout(Stdio::piped());
        command.stderr(Stdio::null());

        let output = tokio::time::timeout(Duration::from_secs(2), command.output()).await;
        let Ok(Ok(output)) = output else {
            return;
        };
        if !output.status.success() {
            return;
        }

        let version_output = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let Some(version) = Self::parse_semver_prefix(&version_output) else {
            return;
        };
        let Some(minimum) = Self::parse_semver_prefix(MINIMUM_CLAUDE_CODE_VERSION) else {
            return;
        };

        if version < minimum {
            eprintln!(
                "Warning: Claude Code version {} is unsupported in the Agent SDK. Minimum required version is {}. Some features may not work correctly.",
                version_output, MINIMUM_CLAUDE_CODE_VERSION
            );
        }
    }

    /// Converts a `PermissionMode` enum variant to its CLI string representation.
    fn permission_mode_to_string(mode: &PermissionMode) -> &'static str {
        match mode {
            PermissionMode::Default => "default",
            PermissionMode::AcceptEdits => "acceptEdits",
            PermissionMode::Plan => "plan",
            PermissionMode::BypassPermissions => "bypassPermissions",
        }
    }

    /// Converts a `SettingSource` enum variant to its CLI string representation.
    fn setting_source_to_string(source: &SettingSource) -> &'static str {
        match source {
            SettingSource::User => "user",
            SettingSource::Project => "project",
            SettingSource::Local => "local",
        }
    }

    /// Builds the combined settings value from `options.settings` and `options.sandbox`.
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

    /// Builds the complete command-line arguments for spawning the CLI process.
    ///
    /// Translates all [`ClaudeAgentOptions`] fields into their corresponding CLI flags.
    ///
    /// # Returns
    ///
    /// A `Vec<String>` where the first element is the CLI path and the rest are arguments.
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

        self.check_claude_version().await;

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
        if self.options.enable_file_checkpointing {
            command.env("CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING", "true");
        }
        for (key, value) in &self.options.env {
            command.env(key, value);
        }

        // Set subprocess user identity on Unix systems.
        #[cfg(unix)]
        if let Some(user) = &self.options.user {
            let uid = Self::resolve_user_to_uid(user)?;
            command.uid(uid);
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

        // Spawn a background task to drain stderr and prevent pipe buffer blocking.
        // The task reads all stderr output and optionally invokes the user's callback.
        if let Some(stderr) = child.stderr.take() {
            let callback = self.stderr_callback.clone();
            self.stderr_task = Some(tokio::spawn(async move {
                let mut reader = BufReader::new(stderr);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line).await {
                        Ok(0) => break, // EOF
                        Ok(_) => {
                            let trimmed = line.trim_end().to_string();
                            if !trimmed.is_empty() {
                                if let Some(cb) = &callback {
                                    cb(trimmed);
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
            }));
        }

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
        // Abort the stderr drain task if still running.
        if let Some(task) = self.stderr_task.take() {
            task.abort();
        }
        Ok(())
    }

    fn is_ready(&self) -> bool {
        self.ready
    }
}

#[cfg(test)]
mod tests {
    use super::SubprocessCliTransport;

    #[test]
    fn parse_semver_prefix_supports_plain_version() {
        assert_eq!(
            SubprocessCliTransport::parse_semver_prefix("2.4.1"),
            Some([2, 4, 1])
        );
    }

    #[test]
    fn parse_semver_prefix_supports_prefixed_version() {
        assert_eq!(
            SubprocessCliTransport::parse_semver_prefix("2.4.1-beta.1"),
            Some([2, 4, 1])
        );
    }

    #[test]
    fn parse_semver_prefix_rejects_invalid_version() {
        assert_eq!(SubprocessCliTransport::parse_semver_prefix("invalid"), None);
    }

    #[cfg(unix)]
    #[test]
    fn resolve_user_to_uid_accepts_numeric_uid() {
        let uid = unsafe { libc::getuid() };
        let resolved = SubprocessCliTransport::resolve_user_to_uid(&uid.to_string())
            .expect("resolve numeric uid");
        assert_eq!(resolved, uid);
    }

    #[cfg(unix)]
    #[test]
    fn resolve_user_to_uid_rejects_unknown_user() {
        let user = format!("__claude_code_sdk_nonexistent_{}__", std::process::id());
        let err = SubprocessCliTransport::resolve_user_to_uid(&user).expect_err("must fail");
        assert!(err.to_string().contains("User not found"));
    }
}
