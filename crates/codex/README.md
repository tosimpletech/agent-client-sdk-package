# Codex Rust SDK (Initial Parity Build)

This crate is a Rust implementation aligned with the official Codex TypeScript SDK shape and behavior (`sdk/typescript`), focused on:

- Public API parity for core thread/turn workflows.
- JSONL event streaming over `codex exec --experimental-json`.
- Structured output support (`--output-schema`).
- Multimodal input support (text + local images).
- Cancellation, thread resume, and CLI option/config forwarding.

## Implemented Public API

- `Codex`
  - `new`
  - `start_thread`
  - `resume_thread`
- `Thread`
  - `id`
  - `run`
  - `run_streamed`
- Input types
  - `Input` (`Text`, `Entries`)
  - `UserInput` (`Text`, `LocalImage`)
- Event and item models
  - `ThreadEvent` + typed payload structs
  - `ThreadItem` + typed item structs
- Options and controls
  - `CodexOptions` (`codex_path_override`, `base_url`, `api_key`, `config`, `env`)
  - `ThreadOptions` (model/sandbox/workdir/reasoning/web-search/approval/additional-directories)
  - `TurnOptions` (`output_schema`, `cancellation_token`)
- Low-level executor
  - `CodexExec`
  - `CodexExecArgs`

## Quickstart

```rust,no_run
use codex::Codex;

# async fn example() -> codex::Result<()> {
let codex = Codex::new(None)?;
let thread = codex.start_thread(None);
let turn = thread.run("Diagnose the test failure and propose a fix", None).await?;

println!("{}", turn.final_response);
println!("{:?}", turn.items);
# Ok(())
# }
```

Call `run()` repeatedly on the same `Thread` to continue the conversation.

```rust,no_run
# use codex::Codex;
# async fn example() -> codex::Result<()> {
# let codex = Codex::new(None)?;
# let thread = codex.start_thread(None);
let next_turn = thread.run("Implement the fix", None).await?;
println!("{}", next_turn.final_response);
# Ok(())
# }
```

### Streaming responses

Use `run_streamed()` when you need intermediate events (`item.started`, `item.updated`, `item.completed`, `turn.completed`, etc.).

```rust,no_run
use codex::{Codex, ThreadEvent};
use futures::StreamExt;

# async fn example() -> codex::Result<()> {
let codex = Codex::new(None)?;
let thread = codex.start_thread(None);
let streamed = thread.run_streamed("Diagnose the failure", None).await?;

let mut events = streamed.events;
while let Some(event) = events.next().await {
    match event? {
        ThreadEvent::ItemCompleted { item } => println!("item: {:?}", item),
        ThreadEvent::TurnCompleted { usage } => println!("usage: {:?}", usage),
        _ => {}
    }
}
# Ok(())
# }
```

### Structured output

Pass JSON Schema in `TurnOptions.output_schema`. The SDK writes a temporary schema file and forwards it via `--output-schema`.

```rust,no_run
use codex::{Codex, TurnOptions};
use serde_json::json;

# async fn example() -> codex::Result<()> {
let codex = Codex::new(None)?;
let thread = codex.start_thread(None);
let schema = json!({
    "type": "object",
    "properties": {
        "summary": { "type": "string" },
        "status": { "type": "string", "enum": ["ok", "action_required"] }
    },
    "required": ["summary", "status"],
    "additionalProperties": false
});

let turn = thread
    .run(
        "Summarize repository status",
        Some(TurnOptions {
            output_schema: Some(schema),
            ..Default::default()
        }),
    )
    .await?;
println!("{}", turn.final_response);
# Ok(())
# }
```

### Attaching images

```rust,no_run
use codex::{Codex, Input, UserInput};

# async fn example() -> codex::Result<()> {
let codex = Codex::new(None)?;
let thread = codex.start_thread(None);

let input = Input::Entries(vec![
    UserInput::Text {
        text: "Describe these screenshots".to_string(),
    },
    UserInput::LocalImage {
        path: "./ui.png".into(),
    },
    UserInput::LocalImage {
        path: "./diagram.jpg".into(),
    },
]);

let turn = thread.run(input, None).await?;
println!("{}", turn.final_response);
# Ok(())
# }
```

### Resuming an existing thread

```rust,no_run
# use codex::Codex;
# async fn example() -> codex::Result<()> {
let codex = Codex::new(None)?;
let thread = codex.resume_thread("thread_abc123", None);
thread.run("Implement the fix", None).await?;
# Ok(())
# }
```

## Test Coverage Mapping (TypeScript -> Rust)

- `tests/run.test.ts` -> `tests/run_tests.rs`
- `tests/runStreamed.test.ts` -> `tests/run_streamed_tests.rs`
- `tests/exec.test.ts` -> `tests/exec_tests.rs`
- `tests/abort.test.ts` -> `tests/abort_tests.rs`

Current status: all Rust tests pass with `cargo test -p codex-client-sdk`.

## Rust 2024 and Library Practices

- Crate uses `edition = "2024"` and `rust-version = "1.85"`.
- `tokio` features are explicitly scoped (`fs`, `io-util`, `macros`, `process`, `rt-multi-thread`, `sync`, `time`) instead of `full`.
- `serde` / `serde_json` are used for protocol-compatible event and item models.
- `thiserror` is used for structured SDK error handling.

## Concurrency Model

- `Thread::run_streamed()` returns a `Send` stream of typed `ThreadEvent` values.
- `Thread::run()` consumes the streamed events and materializes a completed `Turn`.
- Cancellation is driven by `tokio_util::sync::CancellationToken` in `TurnOptions`.

## Functional Differences vs TypeScript SDK

| Area | TypeScript SDK | Rust SDK (this crate) | Notes |
| --- | --- | --- | --- |
| Completion API | `run()` returns `Turn` | `run()` returns `Turn` | Equivalent behavior. |
| Streaming API | `runStreamed()` returns `AsyncGenerator<ThreadEvent>` | `run_streamed()` returns `Stream<Item = Result<ThreadEvent>>` | Rust exposes `futures::Stream`. |
| Cancellation | `AbortSignal` in `TurnOptions` | `CancellationToken` in `TurnOptions` | Rust-idiomatic cancellation primitive. |
| Input model | `string \| UserInput[]` | `Input` enum | Equivalent semantics with typed enums. |
| Env/config forwarding | `CodexOptions` fields | `CodexOptions` fields | Same core option behavior. |
| Thread id access | `thread.id` getter | `thread.id()` method | Rust method form. |

