mod helpers;

use std::collections::HashMap;

use codex::{CodexOptions, Input, UserInput};
use serde_json::json;

use crate::helpers::{MockCodexHarness, expect_pair};

#[tokio::test]
async fn rejects_when_process_exits_non_zero() {
    let harness = MockCodexHarness::new(vec![vec![json!({
        "type": "thread.started",
        "thread_id": "thread_1"
    })]]);

    let mut options = CodexOptions::default();
    let mut env = HashMap::new();
    env.insert("CODEX_MOCK_EXIT_CODE".to_string(), "2".to_string());
    options.env = Some(env);

    let codex = harness.codex(options).expect("codex");
    let thread = codex.start_thread(None);
    let err = thread.run("hi", None).await.expect_err("must fail");
    assert!(err.to_string().contains("code 2"));
}

#[tokio::test]
async fn places_resume_args_before_image_args() {
    let harness = MockCodexHarness::new(vec![
        vec![
            json!({ "type": "thread.started", "thread_id": "thread_1" }),
            json!({ "type": "turn.started" }),
            json!({
                "type": "item.completed",
                "item": { "id": "item_1", "type": "agent_message", "text": "First" }
            }),
            json!({
                "type": "turn.completed",
                "usage": { "input_tokens": 1, "cached_input_tokens": 0, "output_tokens": 1 }
            }),
        ],
        vec![
            json!({ "type": "turn.started" }),
            json!({
                "type": "item.completed",
                "item": { "id": "item_2", "type": "agent_message", "text": "Second" }
            }),
            json!({
                "type": "turn.completed",
                "usage": { "input_tokens": 1, "cached_input_tokens": 0, "output_tokens": 1 }
            }),
        ],
    ]);

    let codex = harness.codex(CodexOptions::default()).expect("codex");
    let thread = codex.start_thread(None);
    thread.run("first", None).await.expect("first run");

    let second_input = Input::Entries(vec![
        UserInput::Text {
            text: "second".to_string(),
        },
        UserInput::LocalImage {
            path: "img.png".into(),
        },
    ]);
    thread.run(second_input, None).await.expect("second run");

    let logs = harness.logs();
    let args = &logs[1].args;
    let resume_index = args
        .iter()
        .position(|arg| arg == "resume")
        .expect("resume arg");
    let image_index = args
        .iter()
        .position(|arg| arg == "--image")
        .expect("image arg");
    assert!(resume_index < image_index);
}

#[tokio::test]
async fn passes_base_url_via_config_instead_of_env() {
    let harness = MockCodexHarness::new(vec![vec![
        json!({ "type": "thread.started", "thread_id": "thread_1" }),
        json!({ "type": "turn.started" }),
        json!({
            "type": "item.completed",
            "item": { "id": "item_1", "type": "agent_message", "text": "ok" }
        }),
        json!({
            "type": "turn.completed",
            "usage": { "input_tokens": 1, "cached_input_tokens": 0, "output_tokens": 1 }
        }),
    ]]);

    let options = CodexOptions {
        base_url: Some("https://example.invalid/v1".to_string()),
        ..Default::default()
    };

    let codex = harness.codex(options).expect("codex");
    let thread = codex.start_thread(None);
    thread.run("hi", None).await.expect("run");

    let logs = harness.logs();
    let invocation = &logs[0];
    expect_pair(
        &invocation.args,
        ("--config", "openai_base_url=\"https://example.invalid/v1\""),
    );
    assert!(!invocation.env.contains_key("OPENAI_BASE_URL"));
}
