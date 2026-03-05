use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use claude_code::{get_session_messages, list_sessions};
use serde_json::json;

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn sanitize_path(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    out
}

fn unique_temp_dir() -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "claude-code-sessions-test-{}-{ts}",
        std::process::id()
    ))
}

fn write_session_file(base: &Path, project_path: &str, session_id: &str) {
    let sanitized = sanitize_path(project_path);
    let project_dir = base.join("projects").join(sanitized);
    fs::create_dir_all(&project_dir).expect("create project dir");
    let session_file = project_dir.join(format!("{session_id}.jsonl"));

    let lines = vec![
        json!({
            "type": "user",
            "uuid": "11111111-1111-1111-1111-111111111111",
            "sessionId": session_id,
            "cwd": project_path,
            "gitBranch": "main",
            "message": {"role": "user", "content": "Hello from session"}
        }),
        json!({
            "type": "assistant",
            "uuid": "22222222-2222-2222-2222-222222222222",
            "parentUuid": "11111111-1111-1111-1111-111111111111",
            "sessionId": session_id,
            "message": {"role": "assistant", "content": [{"type":"text","text":"Hi!"}]}
        }),
    ];

    let mut text = String::new();
    for line in lines {
        text.push_str(&line.to_string());
        text.push('\n');
    }
    fs::write(session_file, text).expect("write session");
}

#[test]
fn test_list_sessions_and_get_session_messages() {
    let lock = ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("lock");
    let config_dir = unique_temp_dir().join("claude-config");
    let project_path = "/tmp/mock-project";
    let session_id = "12345678-1234-1234-1234-1234567890ab";

    write_session_file(&config_dir, project_path, session_id);
    // SAFETY: Serialized via ENV_LOCK in this test module.
    unsafe {
        std::env::set_var("CLAUDE_CONFIG_DIR", &config_dir);
    }

    let sessions = list_sessions(Some(project_path), None, false);
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, session_id);
    assert_eq!(sessions[0].git_branch.as_deref(), Some("main"));
    assert_eq!(sessions[0].summary, "Hello from session");

    let messages = get_session_messages(session_id, Some(project_path), None, 0);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].type_, "user");
    assert_eq!(messages[1].type_, "assistant");

    let paged = get_session_messages(session_id, Some(project_path), Some(1), 1);
    assert_eq!(paged.len(), 1);
    assert_eq!(paged[0].type_, "assistant");

    // SAFETY: Serialized via ENV_LOCK in this test module.
    unsafe {
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }
    drop(lock);
    let _ = fs::remove_dir_all(config_dir.parent().expect("parent"));
}

#[test]
fn test_get_session_messages_rejects_invalid_session_id() {
    let messages = get_session_messages("not-a-uuid", None, None, 0);
    assert!(messages.is_empty());
}

#[test]
fn test_list_sessions_handles_multibyte_prompt_truncation_safely() {
    let lock = ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("lock");
    let config_dir = unique_temp_dir().join("claude-config-multibyte");
    let project_path = "/tmp/mock-project-multibyte";
    let session_id = "12345678-1234-1234-1234-1234567890ac";

    let sanitized = sanitize_path(project_path);
    let project_dir = config_dir.join("projects").join(sanitized);
    fs::create_dir_all(&project_dir).expect("create project dir");
    let session_file = project_dir.join(format!("{session_id}.jsonl"));

    let long_prompt = "你".repeat(240);
    let lines = vec![
        json!({
            "type": "user",
            "uuid": "33333333-3333-3333-3333-333333333333",
            "sessionId": session_id,
            "message": {"role": "user", "content": long_prompt}
        }),
        json!({
            "type": "assistant",
            "uuid": "44444444-4444-4444-4444-444444444444",
            "parentUuid": "33333333-3333-3333-3333-333333333333",
            "sessionId": session_id,
            "message": {"role": "assistant", "content": [{"type":"text","text":"ok"}]}
        }),
    ];

    let mut text = String::new();
    for line in lines {
        text.push_str(&line.to_string());
        text.push('\n');
    }
    fs::write(session_file, text).expect("write session");

    // SAFETY: Serialized via ENV_LOCK in this test module.
    unsafe {
        std::env::set_var("CLAUDE_CONFIG_DIR", &config_dir);
    }

    let sessions = list_sessions(Some(project_path), None, false);
    assert_eq!(sessions.len(), 1);
    assert!(sessions[0].summary.ends_with("..."));
    assert_eq!(sessions[0].summary.chars().count(), 203);

    // SAFETY: Serialized via ENV_LOCK in this test module.
    unsafe {
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }
    drop(lock);
    let _ = fs::remove_dir_all(config_dir.parent().expect("parent"));
}
