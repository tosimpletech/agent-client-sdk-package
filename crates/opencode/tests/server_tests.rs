use std::collections::HashMap;
use std::path::PathBuf;

use opencode::{
    OpencodeServerOptions, OpencodeTuiOptions, create_opencode_server, create_opencode_tui,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct InvocationLog {
    args: Vec<String>,
    env: HashMap<String, String>,
}

fn fixture_cli_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("mock_opencode_cli.py")
}

fn read_logs(path: &std::path::Path) -> Vec<InvocationLog> {
    let content = std::fs::read_to_string(path).expect("read logs");
    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<InvocationLog>(line).expect("parse log"))
        .collect()
}

async fn wait_for_log(path: &std::path::Path) {
    for _ in 0..20 {
        if path.exists() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn create_server_passes_args_and_config() {
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("opencode_invocations.jsonl");

    let options = OpencodeServerOptions {
        hostname: "127.0.0.1".to_string(),
        port: 4199,
        timeout: std::time::Duration::from_millis(1_500),
        config: Some(serde_json::json!({
            "logLevel": "DEBUG",
            "theme": "dracula"
        })),
        cli_path: Some(fixture_cli_path()),
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

    let options = OpencodeTuiOptions {
        project: Some("proj-1".to_string()),
        model: Some("gpt-5".to_string()),
        session: Some("ses_123".to_string()),
        agent: Some("code".to_string()),
        config: Some(serde_json::json!({ "logLevel": "INFO" })),
        cli_path: Some(fixture_cli_path()),
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
}
