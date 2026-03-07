use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::Stdio;

use opencode::{
    OpencodeServerOptions, OpencodeTuiOptions, create_opencode_server, create_opencode_tui,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct InvocationLog {
    args: Vec<String>,
    pid: Option<u32>,
    env: HashMap<String, String>,
}

fn fixture_script_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("mock_opencode_cli.py")
}

fn create_cli_wrapper(temp_dir: &Path) -> PathBuf {
    let python = resolve_python_command();
    let fixture = fixture_script_path();

    #[cfg(windows)]
    {
        let wrapper = temp_dir.join("mock_opencode_cli.cmd");
        let script = format!(
            "@echo off\r\n\"{}\" \"{}\" %*\r\n",
            python,
            fixture.to_string_lossy()
        );
        std::fs::write(&wrapper, script).expect("write windows cli wrapper");
        return wrapper;
    }

    #[cfg(not(windows))]
    {
        let wrapper = temp_dir.join("mock_opencode_cli.sh");
        let script = format!(
            "#!/usr/bin/env sh\nexec \"{}\" \"{}\" \"$@\"\n",
            python,
            fixture.to_string_lossy()
        );
        std::fs::write(&wrapper, script).expect("write unix cli wrapper");
        #[cfg(unix)]
        {
            let mut perms = std::fs::metadata(&wrapper)
                .expect("wrapper metadata")
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&wrapper, perms).expect("set wrapper permissions");
        }
        wrapper
    }
}

fn resolve_python_command() -> String {
    if let Ok(python) = std::env::var("PYTHON") {
        return python;
    }

    for candidate in ["python3", "python"] {
        if which::which(candidate).is_ok() {
            return candidate.to_string();
        }
    }

    "python3".to_string()
}

fn read_logs(path: &Path) -> Vec<InvocationLog> {
    let content = std::fs::read_to_string(path).expect("read logs");
    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<InvocationLog>(line).expect("parse log"))
        .collect()
}

async fn wait_for_log(path: &Path) {
    for _ in 0..20 {
        if path.exists() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

async fn wait_for_log_lines(path: &Path) -> Option<String> {
    for _ in 0..40 {
        if let Ok(content) = std::fs::read_to_string(path) {
            if !content.trim().is_empty() {
                return Some(content);
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    None
}

async fn wait_for_process_exit(pid: u32, exit_log_path: &Path) -> bool {
    for _ in 0..80 {
        if let Ok(content) = std::fs::read_to_string(exit_log_path) {
            if content.lines().any(|line| line.contains("\"serve-exit\"")) {
                return true;
            }
        }

        if !is_process_alive(pid) {
            return true;
        }

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    false
}

#[cfg(unix)]
fn is_process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
fn is_process_alive(pid: u32) -> bool {
    std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}")])
        .output()
        .map(|output| {
            let body = String::from_utf8_lossy(&output.stdout);
            body.contains(&pid.to_string())
        })
        .unwrap_or(false)
}

#[cfg(not(any(unix, windows)))]
fn is_process_alive(_pid: u32) -> bool {
    false
}

#[tokio::test]
async fn create_server_passes_args_and_config() {
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("opencode_invocations.jsonl");
    let cli_path = create_cli_wrapper(temp.path());

    let options = OpencodeServerOptions {
        hostname: "127.0.0.1".to_string(),
        port: 4199,
        timeout: std::time::Duration::from_millis(1_500),
        config: Some(serde_json::json!({
            "logLevel": "DEBUG",
            "theme": "dracula"
        })),
        cli_path: Some(cli_path),
        env: HashMap::from([(
            "OPENCODE_MOCK_LOG".to_string(),
            log_path.to_string_lossy().into_owned(),
        )]),
        cwd: None,
    };

    let mut server = create_opencode_server(Some(options))
        .await
        .expect("create server");
    assert_eq!(server.url, "http://127.0.0.1:4199");

    server.close().await.expect("close server");

    let logs = read_logs(&log_path);
    assert_eq!(logs.len(), 1);

    let args = &logs[0].args;
    assert!(args.contains(&"serve".to_string()));
    assert!(args.contains(&"--hostname=127.0.0.1".to_string()));
    assert!(args.contains(&"--port=4199".to_string()));
    assert!(args.contains(&"--log-level=DEBUG".to_string()));

    let cfg = logs[0]
        .env
        .get("OPENCODE_CONFIG_CONTENT")
        .expect("config env present");
    let cfg_json: Value = serde_json::from_str(cfg).expect("config json");
    assert_eq!(cfg_json["theme"], "dracula");
}

#[tokio::test]
async fn create_tui_passes_flags() {
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("opencode_tui_invocations.jsonl");
    let cli_path = create_cli_wrapper(temp.path());

    let options = OpencodeTuiOptions {
        project: Some("proj-1".to_string()),
        model: Some("gpt-5".to_string()),
        session: Some("ses_123".to_string()),
        agent: Some("code".to_string()),
        config: Some(serde_json::json!({ "logLevel": "INFO" })),
        cli_path: Some(cli_path),
        env: HashMap::from([(
            "OPENCODE_MOCK_LOG".to_string(),
            log_path.to_string_lossy().into_owned(),
        )]),
        cwd: None,
    };

    let mut tui = create_opencode_tui(Some(options)).expect("create tui");
    wait_for_log(&log_path).await;
    tui.close().await.expect("close tui");

    let logs = read_logs(&log_path);
    assert_eq!(logs.len(), 1);

    let args = &logs[0].args;
    assert!(args.contains(&"--project=proj-1".to_string()));
    assert!(args.contains(&"--model=gpt-5".to_string()));
    assert!(args.contains(&"--session=ses_123".to_string()));
    assert!(args.contains(&"--agent=code".to_string()));

    let cfg = logs[0]
        .env
        .get("OPENCODE_CONFIG_CONTENT")
        .expect("config env present");
    let cfg_json: Value = serde_json::from_str(cfg).expect("config json");
    assert_eq!(cfg_json["logLevel"], "INFO");
}

#[tokio::test]
async fn startup_timeout_kills_server_process() {
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("opencode_timeout_invocations.jsonl");
    let exit_log_path = temp.path().join("opencode_exit.jsonl");
    let cli_path = create_cli_wrapper(temp.path());

    let options = OpencodeServerOptions {
        hostname: "127.0.0.1".to_string(),
        port: 4200,
        timeout: std::time::Duration::from_millis(1200),
        config: None,
        cli_path: Some(cli_path),
        env: HashMap::from([
            (
                "OPENCODE_MOCK_LOG".to_string(),
                log_path.to_string_lossy().into_owned(),
            ),
            ("OPENCODE_MOCK_NO_LISTEN".to_string(), "1".to_string()),
            (
                "OPENCODE_MOCK_EXIT_LOG".to_string(),
                exit_log_path.to_string_lossy().into_owned(),
            ),
        ]),
        cwd: None,
    };

    let err = create_opencode_server(Some(options))
        .await
        .expect_err("must timeout");
    assert!(matches!(err, opencode::Error::ServerStartupTimeout { .. }));

    wait_for_log_lines(&log_path)
        .await
        .expect("invocation log should exist");
    let logs = read_logs(&log_path);
    let pid = logs
        .first()
        .and_then(|entry| entry.pid)
        .expect("pid should be logged");
    assert!(
        wait_for_process_exit(pid, &exit_log_path).await,
        "server process still alive"
    );
}

#[test]
fn explicit_missing_cli_path_returns_cli_not_found() {
    let missing = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("definitely-missing-opencode-cli");

    let options = OpencodeTuiOptions {
        cli_path: Some(missing),
        ..Default::default()
    };

    let err = create_opencode_tui(Some(options)).expect_err("must fail");
    assert!(matches!(err, opencode::Error::CLINotFound(_)));
}
