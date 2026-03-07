# OpenCode SDK for Rust

Rust SDK aligned with the official OpenCode JavaScript SDK (`@opencode-ai/sdk` `v1.2.20`).

## Features

- Start/stop local OpenCode server: `create_opencode_server`
- Start/stop OpenCode TUI: `create_opencode_tui`
- Create OpenCode HTTP client: `create_opencode_client`
- One-shot helper: `create_opencode` (returns server + client)
- Session/global/project/event endpoint wrappers
- Generic operation dispatcher by `operationId`: `call_operation` / `call_operation_sse`

## Installation

```toml
[dependencies]
opencode-client-sdk = "1.2.20"
```

## Quickstart

```rust,no_run
use opencode::{create_opencode, RequestOptions};
use serde_json::json;

# async fn run() -> opencode::Result<()> {
let mut app = create_opencode(None).await?;

let session = app
    .client
    .session()
    .create(RequestOptions::default())
    .await?;

let session_id = session.data["id"].as_str().unwrap();

let response = app
    .client
    .session()
    .prompt(
        RequestOptions::default()
            .with_path("id", session_id)
            .with_body(json!({
                "parts": [
                    { "type": "text", "text": "Summarize this repo" }
                ]
            })),
    )
    .await?;

println!("status={} data={}", response.status, response.data);

app.server.close().await?;
# Ok(())
# }
```

## Notes

- The client uses JSON `Value` payloads for forward compatibility with frequently changing OpenCode schemas.
- `x-opencode-directory` is applied automatically when `OpencodeClientConfig.directory` is set.
