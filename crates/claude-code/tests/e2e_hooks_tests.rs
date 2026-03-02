use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use claude_code::{
    ClaudeAgentOptions, ContentBlock, HookCallback, HookMatcher, InputPrompt, Message, query,
};
use futures::FutureExt;
use serde_json::{Value, json};

const HOOK_E2E_MOCK_CLI: &str = r#"#!/usr/bin/env python3
import json
import os
import sys

RESULT_MESSAGE = {
    "type": "result",
    "subtype": "success",
    "duration_ms": 10,
    "duration_api_ms": 5,
    "is_error": False,
    "num_turns": 1,
    "session_id": "hook-e2e-session",
    "total_cost_usd": 0.0,
}


def emit(obj):
    print(json.dumps(obj), flush=True)


def parse_flags(argv):
    flags = set()
    values = {}
    i = 0
    while i < len(argv):
        token = argv[i]
        if token.startswith("--"):
            key = token[2:]
            if i + 1 < len(argv) and not argv[i + 1].startswith("--"):
                values[key] = argv[i + 1]
                i += 2
                continue
            flags.add(key)
        i += 1
    return flags, values


def first_callback_id(hooks, event_name):
    event_matchers = hooks.get(event_name, [])
    for matcher in event_matchers:
        callback_ids = matcher.get("hookCallbackIds", [])
        if callback_ids:
            return callback_ids[0]
    return None


def emit_tool_use_assistant():
    emit(
        {
            "type": "assistant",
            "message": {
                "content": [
                    {"type": "text", "text": "mock tool execution"},
                    {
                        "type": "tool_use",
                        "id": "toolu_mock_1",
                        "name": "Bash",
                        "input": {"command": "echo 'hook test'"},
                    },
                ],
                "model": "claude-sonnet-4-5",
            },
        }
    )


def build_requests_for_scenario(scenario):
    pre_tool_input = {
        "session_id": "sess-1",
        "transcript_path": "/tmp/transcript",
        "cwd": "/tmp",
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "echo pre"},
        "tool_use_id": "toolu-pre-1",
    }

    post_tool_input = {
        "session_id": "sess-1",
        "transcript_path": "/tmp/transcript",
        "cwd": "/tmp",
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "echo post"},
        "tool_response": {"stdout": "ok"},
        "tool_use_id": "toolu-post-1",
    }

    notification_input = {
        "session_id": "sess-1",
        "transcript_path": "/tmp/transcript",
        "cwd": "/tmp",
        "hook_event_name": "Notification",
        "message": "Task completed",
        "notification_type": "info",
    }

    if scenario == "permission_decision":
        return [("PreToolUse", pre_tool_input, "toolu-pre-1")]
    if scenario == "continue_stop":
        return [("PostToolUse", post_tool_input, "toolu-post-1")]
    if scenario == "additional_context":
        return [("PostToolUse", post_tool_input, "toolu-post-1")]
    if scenario == "pre_tool_use_tool_use_id":
        return [("PreToolUse", pre_tool_input, "toolu-pre-1")]
    if scenario == "post_tool_use_tool_use_id":
        return [("PostToolUse", post_tool_input, "toolu-post-1")]
    if scenario == "notification_hook":
        return [("Notification", notification_input, None)]
    if scenario == "multiple_hooks":
        return [
            ("PreToolUse", pre_tool_input, "toolu-pre-1"),
            ("PostToolUse", post_tool_input, "toolu-post-1"),
            ("Notification", notification_input, None),
        ]
    return []


def emit_summary_and_result(scenario, hooks_config, request_order, pending):
    requests = []
    for req_id in request_order:
        item = pending[req_id]
        requests.append(
            {
                "request_id": req_id,
                "event": item["event"],
                "input": item["input"],
                "tool_use_id": item["tool_use_id"],
                "response": item["response"],
            }
        )

    summary = {
        "scenario": scenario,
        "hooks_seen": sorted(list(hooks_config.keys())),
        "requests": requests,
    }
    emit(
        {
            "type": "assistant",
            "message": {
                "content": [
                    {"type": "text", "text": "HOOK_SUMMARY:" + json.dumps(summary, sort_keys=True)}
                ],
                "model": "claude-sonnet-4-5",
            },
        }
    )
    emit(RESULT_MESSAGE)


def main():
    argv = sys.argv[1:]
    if "-v" in argv or "--version" in argv:
        print("2.1.0")
        return 0

    _, values = parse_flags(argv)
    scenario = values.get("hook-scenario", os.environ.get("MOCK_HOOK_SCENARIO", ""))

    hooks_config = {}
    pending = {}
    request_order = []

    for raw in sys.stdin:
        line = raw.strip()
        if not line:
            continue

        try:
            msg = json.loads(line)
        except Exception:
            continue

        msg_type = msg.get("type")

        if msg_type == "control_request":
            req = msg.get("request", {})
            subtype = req.get("subtype")
            req_id = msg.get("request_id", "")

            if subtype == "initialize":
                hooks_config = req.get("hooks", {})
                emit(
                    {
                        "type": "control_response",
                        "response": {
                            "subtype": "success",
                            "request_id": req_id,
                            "response": {"ok": True},
                        },
                    }
                )
                continue

            emit(
                {
                    "type": "control_response",
                    "response": {
                        "subtype": "success",
                        "request_id": req_id,
                        "response": {"ack": subtype},
                    },
                }
            )
            continue

        if msg_type == "user":
            emit_tool_use_assistant()

            for index, (event_name, input_payload, tool_use_id) in enumerate(
                build_requests_for_scenario(scenario), start=1
            ):
                callback_id = first_callback_id(hooks_config, event_name)
                if callback_id is None:
                    continue
                request_id = f"{scenario}-{index}"
                request_order.append(request_id)
                pending[request_id] = {
                    "event": event_name,
                    "input": input_payload,
                    "tool_use_id": tool_use_id,
                }
                emit(
                    {
                        "type": "control_request",
                        "request_id": request_id,
                        "request": {
                            "subtype": "hook_callback",
                            "callback_id": callback_id,
                            "input": input_payload,
                            "tool_use_id": tool_use_id,
                        },
                    }
                )

            if not request_order:
                emit_summary_and_result(scenario, hooks_config, request_order, pending)
                return 0
            continue

        if msg_type == "control_response":
            response = msg.get("response", {})
            request_id = response.get("request_id")
            if request_id not in pending:
                continue

            pending[request_id]["response"] = response
            if all("response" in pending[req_id] for req_id in request_order):
                emit_summary_and_result(scenario, hooks_config, request_order, pending)
                return 0

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
"#;

fn build_mock_hook_cli_path() -> PathBuf {
    static SCRIPT_COUNTER: AtomicUsize = AtomicUsize::new(0);

    let unique_id = SCRIPT_COUNTER.fetch_add(1, Ordering::SeqCst);
    let script_path = std::env::temp_dir().join(format!(
        "claude_hook_e2e_mock_{}_{}.py",
        std::process::id(),
        unique_id
    ));

    fs::write(&script_path, HOOK_E2E_MOCK_CLI).expect("write hook e2e mock cli script");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut perms = fs::metadata(&script_path).expect("script metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).expect("set executable permission");
    }

    script_path
}

fn hook_matcher(matcher: Option<&str>, hook: HookCallback) -> HookMatcher {
    let mut hook_matcher = HookMatcher {
        matcher: matcher.map(ToString::to_string),
        ..Default::default()
    };
    hook_matcher.hooks.push(hook);
    hook_matcher
}

async fn run_hook_scenario(
    scenario: &str,
    hooks: HashMap<String, Vec<HookMatcher>>,
) -> (Vec<Message>, Value) {
    let script_path = build_mock_hook_cli_path();

    let mut options = ClaudeAgentOptions {
        cli_path: Some(script_path),
        hooks: Some(hooks),
        ..Default::default()
    };
    options
        .extra_args
        .insert("hook-scenario".to_string(), Some(scenario.to_string()));

    let messages = query(
        InputPrompt::Text(format!("run scenario {scenario}")),
        Some(options),
        None,
    )
    .await
    .expect("query");

    assert!(messages.iter().any(|m| matches!(m, Message::Result(_))));
    assert!(messages.iter().any(|m| {
        matches!(
            m,
            Message::Assistant(assistant)
                if assistant
                    .content
                    .iter()
                    .any(|block| matches!(block, ContentBlock::ToolUse(_)))
        )
    }));

    (messages.clone(), extract_hook_summary(&messages))
}

fn extract_hook_summary(messages: &[Message]) -> Value {
    for message in messages {
        if let Message::Assistant(assistant) = message {
            for block in &assistant.content {
                if let ContentBlock::Text(text_block) = block {
                    if let Some(summary_str) = text_block.text.strip_prefix("HOOK_SUMMARY:") {
                        return serde_json::from_str(summary_str).expect("valid summary json");
                    }
                }
            }
        }
    }
    panic!("missing HOOK_SUMMARY assistant message");
}

#[tokio::test]
async fn test_e2e_hook_permission_decision() {
    let invocations = Arc::new(Mutex::new(Vec::<String>::new()));
    let invocations_clone = invocations.clone();
    let pre_tool_hook = Arc::new(move |input: Value, _tool_use_id: Option<String>, _ctx| {
        let tool_name = input
            .get("tool_name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        invocations_clone
            .lock()
            .expect("lock invocations")
            .push(tool_name.clone());

        async move {
            if tool_name == "Bash" {
                Ok(json!({
                    "reason": "Bash commands are blocked in this test for safety",
                    "systemMessage": "Command blocked by hook",
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "deny",
                        "permissionDecisionReason": "Security policy: Bash blocked"
                    }
                }))
            } else {
                Ok(json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "allow",
                        "permissionDecisionReason": "Tool passed security checks"
                    }
                }))
            }
        }
        .boxed()
    });

    let mut hooks = HashMap::new();
    hooks.insert(
        "PreToolUse".to_string(),
        vec![hook_matcher(Some("Bash"), pre_tool_hook)],
    );

    let (_messages, summary) = run_hook_scenario("permission_decision", hooks).await;

    let invocations = invocations.lock().expect("lock invocations");
    assert!(invocations.iter().any(|tool| tool == "Bash"));

    let request = &summary["requests"][0];
    assert_eq!(request["event"], "PreToolUse");
    assert_eq!(
        request["response"]["response"]["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );
    assert_eq!(
        request["response"]["response"]["hookSpecificOutput"]["permissionDecisionReason"],
        "Security policy: Bash blocked"
    );
}

#[tokio::test]
async fn test_e2e_hook_continue_and_stop() {
    let post_tool_hook = Arc::new(|_input: Value, _tool_use_id: Option<String>, _ctx| {
        async move {
            Ok(json!({
                "continue_": false,
                "stopReason": "Execution halted by test hook for validation",
                "reason": "Testing continue and stopReason fields",
                "systemMessage": "Test hook stopped execution"
            }))
        }
        .boxed()
    });

    let mut hooks = HashMap::new();
    hooks.insert(
        "PostToolUse".to_string(),
        vec![hook_matcher(Some("Bash"), post_tool_hook)],
    );

    let (_messages, summary) = run_hook_scenario("continue_stop", hooks).await;

    let request = &summary["requests"][0];
    assert_eq!(request["event"], "PostToolUse");
    assert_eq!(request["response"]["response"]["continue"], false);
    assert!(request["response"]["response"].get("continue_").is_none());
    assert_eq!(
        request["response"]["response"]["stopReason"],
        "Execution halted by test hook for validation"
    );
}

#[tokio::test]
async fn test_e2e_hook_additional_context() {
    let post_tool_hook = Arc::new(|_input: Value, _tool_use_id: Option<String>, _ctx| {
        async move {
            Ok(json!({
                "systemMessage": "Additional context provided by hook",
                "reason": "Hook providing monitoring feedback",
                "suppressOutput": false,
                "hookSpecificOutput": {
                    "hookEventName": "PostToolUse",
                    "additionalContext": "The command executed successfully with hook monitoring"
                }
            }))
        }
        .boxed()
    });

    let mut hooks = HashMap::new();
    hooks.insert(
        "PostToolUse".to_string(),
        vec![hook_matcher(Some("Bash"), post_tool_hook)],
    );

    let (_messages, summary) = run_hook_scenario("additional_context", hooks).await;

    let request = &summary["requests"][0];
    assert_eq!(request["event"], "PostToolUse");
    assert_eq!(
        request["response"]["response"]["hookSpecificOutput"]["additionalContext"],
        "The command executed successfully with hook monitoring"
    );
}

#[tokio::test]
async fn test_e2e_pre_tool_use_with_tool_use_id() {
    let invocations = Arc::new(Mutex::new(Vec::<(Value, Option<String>)>::new()));
    let invocations_clone = invocations.clone();

    let pre_tool_hook = Arc::new(move |input: Value, tool_use_id: Option<String>, _ctx| {
        invocations_clone
            .lock()
            .expect("lock invocations")
            .push((input.clone(), tool_use_id.clone()));

        async move {
            Ok(json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "allow",
                    "permissionDecisionReason": "Approved with context",
                    "additionalContext": "This command is running in a test environment"
                }
            }))
        }
        .boxed()
    });

    let mut hooks = HashMap::new();
    hooks.insert(
        "PreToolUse".to_string(),
        vec![hook_matcher(Some("Bash"), pre_tool_hook)],
    );

    let (_messages, summary) = run_hook_scenario("pre_tool_use_tool_use_id", hooks).await;

    let invocations = invocations.lock().expect("lock invocations");
    assert_eq!(invocations.len(), 1);
    assert_eq!(
        invocations[0]
            .0
            .get("tool_use_id")
            .and_then(Value::as_str)
            .expect("tool_use_id in input"),
        "toolu-pre-1"
    );
    assert_eq!(invocations[0].1.as_deref(), Some("toolu-pre-1"));

    let request = &summary["requests"][0];
    assert_eq!(
        request["response"]["response"]["hookSpecificOutput"]["additionalContext"],
        "This command is running in a test environment"
    );
}

#[tokio::test]
async fn test_e2e_post_tool_use_with_tool_use_id() {
    let invocations = Arc::new(Mutex::new(Vec::<(Value, Option<String>)>::new()));
    let invocations_clone = invocations.clone();

    let post_tool_hook = Arc::new(move |input: Value, tool_use_id: Option<String>, _ctx| {
        invocations_clone
            .lock()
            .expect("lock invocations")
            .push((input.clone(), tool_use_id.clone()));

        async move {
            Ok(json!({
                "hookSpecificOutput": {
                    "hookEventName": "PostToolUse",
                    "additionalContext": "Post-tool monitoring active"
                }
            }))
        }
        .boxed()
    });

    let mut hooks = HashMap::new();
    hooks.insert(
        "PostToolUse".to_string(),
        vec![hook_matcher(Some("Bash"), post_tool_hook)],
    );

    let (_messages, summary) = run_hook_scenario("post_tool_use_tool_use_id", hooks).await;

    let invocations = invocations.lock().expect("lock invocations");
    assert_eq!(invocations.len(), 1);
    assert_eq!(
        invocations[0]
            .0
            .get("tool_use_id")
            .and_then(Value::as_str)
            .expect("tool_use_id in input"),
        "toolu-post-1"
    );
    assert_eq!(invocations[0].1.as_deref(), Some("toolu-post-1"));

    let request = &summary["requests"][0];
    assert_eq!(
        request["response"]["response"]["hookSpecificOutput"]["additionalContext"],
        "Post-tool monitoring active"
    );
}

#[tokio::test]
async fn test_e2e_notification_hook() {
    let invocations = Arc::new(Mutex::new(Vec::<Value>::new()));
    let invocations_clone = invocations.clone();

    let notification_hook = Arc::new(move |input: Value, _tool_use_id: Option<String>, _ctx| {
        invocations_clone
            .lock()
            .expect("lock invocations")
            .push(input.clone());

        async move {
            Ok(json!({
                "hookSpecificOutput": {
                    "hookEventName": "Notification",
                    "additionalContext": "Notification received"
                }
            }))
        }
        .boxed()
    });

    let mut hooks = HashMap::new();
    hooks.insert(
        "Notification".to_string(),
        vec![hook_matcher(None, notification_hook)],
    );

    let (_messages, summary) = run_hook_scenario("notification_hook", hooks).await;

    let invocations = invocations.lock().expect("lock invocations");
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0]["hook_event_name"], "Notification");
    assert_eq!(invocations[0]["notification_type"], "info");

    let request = &summary["requests"][0];
    assert_eq!(request["event"], "Notification");
    assert_eq!(
        request["response"]["response"]["hookSpecificOutput"]["additionalContext"],
        "Notification received"
    );
}

#[tokio::test]
async fn test_e2e_multiple_hooks() {
    let invocations = Arc::new(Mutex::new(Vec::<String>::new()));

    let mut hooks = HashMap::new();

    let pre_invocations = invocations.clone();
    let pre_hook = Arc::new(move |input: Value, _tool_use_id: Option<String>, _ctx| {
        let event = input
            .get("hook_event_name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        pre_invocations.lock().expect("lock invocations").push(event);
        async move {
            Ok(json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "allow"
                }
            }))
        }
        .boxed()
    });

    let post_invocations = invocations.clone();
    let post_hook = Arc::new(move |input: Value, _tool_use_id: Option<String>, _ctx| {
        let event = input
            .get("hook_event_name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        post_invocations.lock().expect("lock invocations").push(event);
        async move {
            Ok(json!({
                "hookSpecificOutput": {
                    "hookEventName": "PostToolUse",
                    "additionalContext": "Post hook processed"
                }
            }))
        }
        .boxed()
    });

    let notification_invocations = invocations.clone();
    let notification_hook = Arc::new(move |input: Value, _tool_use_id: Option<String>, _ctx| {
        let event = input
            .get("hook_event_name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        notification_invocations
            .lock()
            .expect("lock invocations")
            .push(event);
        async move {
            Ok(json!({
                "hookSpecificOutput": {
                    "hookEventName": "Notification",
                    "additionalContext": "Notification processed"
                }
            }))
        }
        .boxed()
    });

    hooks.insert(
        "PreToolUse".to_string(),
        vec![hook_matcher(Some("Bash"), pre_hook)],
    );
    hooks.insert(
        "PostToolUse".to_string(),
        vec![hook_matcher(Some("Bash"), post_hook)],
    );
    hooks.insert(
        "Notification".to_string(),
        vec![hook_matcher(None, notification_hook)],
    );

    let (_messages, summary) = run_hook_scenario("multiple_hooks", hooks).await;

    let invocations = invocations.lock().expect("lock invocations");
    assert!(invocations.iter().any(|event| event == "PreToolUse"));
    assert!(invocations.iter().any(|event| event == "PostToolUse"));
    assert!(invocations.iter().any(|event| event == "Notification"));

    let requests = summary["requests"].as_array().expect("summary requests array");
    assert_eq!(requests.len(), 3);
    assert!(requests.iter().any(|request| request["event"] == "PreToolUse"));
    assert!(
        requests
            .iter()
            .any(|request| request["event"] == "PostToolUse")
    );
    assert!(
        requests
            .iter()
            .any(|request| request["event"] == "Notification")
    );
    assert!(requests
        .iter()
        .all(|request| request["response"]["subtype"] == "success"));
}
