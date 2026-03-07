use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use crate::client::{OpencodeClient, OpencodeClientConfig, create_opencode_client};
use crate::errors::{CLINotFoundError, Error, OpencodeSDKError, ProcessError, Result};

/// Options for launching `opencode serve`.
#[derive(Debug, Clone)]
pub struct OpencodeServerOptions {
    /// Hostname passed to `--hostname`.
    pub hostname: String,
    /// Port passed to `--port`.
    pub port: u16,
    /// Startup timeout while waiting for server URL log line.
    pub timeout: Duration,
    /// Optional OpenCode config JSON forwarded via `OPENCODE_CONFIG_CONTENT`.
    pub config: Option<serde_json::Value>,
    /// Optional explicit CLI path. If omitted, resolved via `which opencode`.
    pub cli_path: Option<PathBuf>,
    /// Optional extra environment variables.
    pub env: HashMap<String, String>,
    /// Optional working directory.
    pub cwd: Option<PathBuf>,
}

impl Default for OpencodeServerOptions {
    fn default() -> Self {
        Self {
            hostname: "127.0.0.1".to_string(),
            port: 4096,
            timeout: Duration::from_millis(5_000),
            config: None,
            cli_path: None,
            env: HashMap::new(),
            cwd: None,
        }
    }
}

/// Options for launching `opencode` TUI.
#[derive(Debug, Clone, Default)]
pub struct OpencodeTuiOptions {
    pub project: Option<String>,
    pub model: Option<String>,
    pub session: Option<String>,
    pub agent: Option<String>,
    /// Optional OpenCode config JSON forwarded via `OPENCODE_CONFIG_CONTENT`.
    pub config: Option<serde_json::Value>,
    /// Optional explicit CLI path. If omitted, resolved via `which opencode`.
    pub cli_path: Option<PathBuf>,
    /// Optional extra environment variables.
    pub env: HashMap<String, String>,
    /// Optional working directory.
    pub cwd: Option<PathBuf>,
}

/// Running OpenCode local server process.
#[derive(Debug)]
pub struct OpencodeServer {
    /// Base URL parsed from OpenCode startup logs.
    pub url: String,
    child: Child,
}

impl OpencodeServer {
    /// Stop the server process.
    pub async fn close(&mut self) -> Result<()> {
        if self.child.id().is_some() {
            self.child.start_kill()?;
            let _ = self.child.wait().await;
        }
        Ok(())
    }
}

impl Drop for OpencodeServer {
    fn drop(&mut self) {
        if self.child.id().is_some() {
            let _ = self.child.start_kill();
        }
    }
}

/// Running OpenCode TUI process.
#[derive(Debug)]
pub struct OpencodeTui {
    child: Child,
}

impl OpencodeTui {
    /// Stop the TUI process.
    pub async fn close(&mut self) -> Result<()> {
        if self.child.id().is_some() {
            self.child.start_kill()?;
            let _ = self.child.wait().await;
        }
        Ok(())
    }
}

impl Drop for OpencodeTui {
    fn drop(&mut self) {
        if self.child.id().is_some() {
            let _ = self.child.start_kill();
        }
    }
}

/// Bundled OpenCode server + client (equivalent to JS `createOpencode`).
#[derive(Debug)]
pub struct Opencode {
    pub client: OpencodeClient,
    pub server: OpencodeServer,
}

/// Launch `opencode serve` and wait for startup URL.
pub async fn create_opencode_server(
    options: Option<OpencodeServerOptions>,
) -> Result<OpencodeServer> {
    let options = options.unwrap_or_default();
    let cli_path = resolve_cli_path(options.cli_path.as_deref())?;

    let mut args = vec![
        "serve".to_string(),
        format!("--hostname={}", options.hostname),
        format!("--port={}", options.port),
    ];

    if let Some(log_level) = options
        .config
        .as_ref()
        .and_then(|cfg| cfg.get("logLevel"))
        .and_then(serde_json::Value::as_str)
    {
        args.push(format!("--log-level={log_level}"));
    }

    let mut cmd = Command::new(&cli_path);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(cwd) = &options.cwd {
        cmd.current_dir(cwd);
    }

    cmd.envs(std::env::vars());
    cmd.envs(options.env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    cmd.env(
        "OPENCODE_CONFIG_CONTENT",
        serde_json::to_string(&options.config.unwrap_or_else(|| serde_json::json!({})))?,
    );

    let mut child = cmd.spawn()?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| OpencodeSDKError::new("Failed to capture opencode stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| OpencodeSDKError::new("Failed to capture opencode stderr"))?;

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(read_lines(stdout, tx.clone()));
    tokio::spawn(read_lines(stderr, tx));

    let timeout_ms = options.timeout.as_millis() as u64;
    let sleeper = tokio::time::sleep(options.timeout);
    tokio::pin!(sleeper);

    let mut output = String::new();

    loop {
        tokio::select! {
            _ = &mut sleeper => {
                terminate_child(&mut child).await;
                return Err(Error::ServerStartupTimeout { timeout_ms });
            }
            maybe_line = rx.recv() => {
                match maybe_line {
                    Some(line) => {
                        output.push_str(&line);
                        output.push('\n');

                        if line.starts_with("opencode server listening") {
                            if let Some(url) = extract_url_from_line(&line) {
                                return Ok(OpencodeServer { url, child });
                            }

                            terminate_child(&mut child).await;
                            return Err(Error::OpencodeSDK(OpencodeSDKError::new(format!(
                                "Failed to parse server url from output: {line}"
                            ))));
                        }
                    }
                    None => {
                        if let Some(status) = child.try_wait()? {
                            return Err(Error::Process(ProcessError::new(
                                "Server exited before reporting a listening URL",
                                status.code(),
                                Some(output),
                            )));
                        }

                        terminate_child(&mut child).await;
                        return Err(Error::Process(ProcessError::new(
                            "Server log streams closed before reporting a listening URL",
                            None,
                            Some(output),
                        )));
                    }
                }
            }
            wait_result = child.wait() => {
                let status = wait_result?;
                return Err(Error::Process(ProcessError::new(
                    "Server exited before startup completed",
                    status.code(),
                    Some(output),
                )));
            }
        }
    }
}

/// Launch OpenCode TUI process.
pub fn create_opencode_tui(options: Option<OpencodeTuiOptions>) -> Result<OpencodeTui> {
    let options = options.unwrap_or_default();
    let cli_path = resolve_cli_path(options.cli_path.as_deref())?;

    let mut args = Vec::new();
    if let Some(project) = options.project {
        args.push(format!("--project={project}"));
    }
    if let Some(model) = options.model {
        args.push(format!("--model={model}"));
    }
    if let Some(session) = options.session {
        args.push(format!("--session={session}"));
    }
    if let Some(agent) = options.agent {
        args.push(format!("--agent={agent}"));
    }

    let mut cmd = Command::new(cli_path);
    cmd.args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    if let Some(cwd) = &options.cwd {
        cmd.current_dir(cwd);
    }

    cmd.envs(std::env::vars());
    cmd.envs(options.env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    cmd.env(
        "OPENCODE_CONFIG_CONTENT",
        serde_json::to_string(&options.config.unwrap_or_else(|| serde_json::json!({})))?,
    );

    let child = cmd.spawn()?;
    Ok(OpencodeTui { child })
}

/// Create local server + bound client together.
pub async fn create_opencode(options: Option<OpencodeServerOptions>) -> Result<Opencode> {
    let server = create_opencode_server(options).await?;
    let client = create_opencode_client(Some(OpencodeClientConfig {
        base_url: server.url.clone(),
        ..Default::default()
    }))?;

    Ok(Opencode { client, server })
}

async fn read_lines<R>(reader: R, tx: mpsc::UnboundedSender<String>)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let _ = tx.send(line);
    }
}

async fn terminate_child(child: &mut Child) {
    if child.id().is_some() {
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
}

fn resolve_cli_path(cli_path: Option<&Path>) -> Result<PathBuf> {
    match cli_path {
        Some(path) if is_bare_command(path) => which::which(path).map_err(|_| {
            Error::CLINotFound(CLINotFoundError::new(
                "OpenCode CLI not found in PATH",
                Some(path.to_string_lossy().into_owned()),
            ))
        }),
        Some(path) => {
            if path.is_file() {
                Ok(path.to_path_buf())
            } else {
                Err(Error::CLINotFound(CLINotFoundError::new(
                    "OpenCode CLI not found at configured path",
                    Some(path.to_string_lossy().into_owned()),
                )))
            }
        }
        None => which::which("opencode").map_err(|_| {
            Error::CLINotFound(CLINotFoundError::new(
                "OpenCode CLI not found in PATH",
                Some("opencode".to_string()),
            ))
        }),
    }
}

fn is_bare_command(path: &Path) -> bool {
    let mut components = path.components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

fn extract_url_from_line(line: &str) -> Option<String> {
    for prefix in ["http://", "https://"] {
        if let Some(start) = line.find(prefix) {
            let rest = &line[start..];
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            return Some(rest[..end].to_string());
        }
    }
    None
}
