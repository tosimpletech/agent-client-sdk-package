# OpenCode Rust SDK

[English](README.md) | [中文](README_zh.md)

Rust SDK for integrating OpenCode by launching the local OpenCode CLI server and calling its HTTP/SSE APIs.

## Table of Contents

- [Overview](#overview)
- [Status](#status)
- [Installation](#installation)
- [Authentication and Environment Setup](#authentication-and-environment-setup)
- [Quickstart](#quickstart)
- [API Selection Guide](#api-selection-guide)
- [Core API Surface](#core-api-surface)
- [Feature Highlights](#feature-highlights)
- [Feature Comparison with Official JavaScript SDK](#feature-comparison-with-official-javascript-sdk)
- [Compatibility Matrix](#compatibility-matrix)
- [Known Limitations](#known-limitations)
- [Testing and Validation](#testing-and-validation)
- [Concurrency Model](#concurrency-model)
- [Development](#development)
- [Contributing](#contributing)
- [License](#license)

## Overview

This crate is a parity-focused Rust implementation aligned with official OpenCode SDK behavior (`@opencode-ai/sdk` `v1.2.20`) while using Rust-idiomatic APIs.

It supports:

- Local server lifecycle (`create_opencode_server`, `create_opencode`)
- TUI process lifecycle (`create_opencode_tui`)
- Typed client entry with endpoint namespaces (`session`, `global`, `project`, `provider`, `mcp`, `tui`, etc.)
- Generic operation-id dispatch (`call_operation`, `call_operation_sse`)
- Robust SSE parsing (split UTF-8 chunks, trailing non-blank-terminated lines)

## Status

- Package version: `1.2.20` (`opencode-client-sdk`)
- Scope: parity-focused implementation of OpenCode CLI server + SDK HTTP workflows
- Validation: crate check/tests are maintained with fixture-based subprocess and HTTP tests
- Rust docs: public APIs are documented and exported through `opencode`

## Installation

This repository currently uses a workspace/local package layout.

```toml
[dependencies]
opencode = { package = "opencode-client-sdk", path = "../../crates/opencode" }
```

Runtime prerequisites:

- Rust 1.85+ (edition 2024)
- OpenCode CLI installed and accessible (`opencode`)

## Authentication and Environment Setup

This SDK calls the external OpenCode CLI/server. Authentication is typically handled by your OpenCode runtime configuration.

### Option A: existing CLI/runtime environment

Use your existing OpenCode environment setup and login/session state.

### Option B: programmatic headers and runtime options

```rust,no_run
use std::collections::HashMap;
use opencode::{create_opencode_client, OpencodeClientConfig};

# fn example() -> opencode::Result<()> {
let mut headers = HashMap::new();
headers.insert("x-custom-header".to_string(), "value".to_string());

let _client = create_opencode_client(Some(OpencodeClientConfig {
    base_url: "http://127.0.0.1:4096".to_string(),
    headers,
    bearer_token: None,
    directory: Some("/tmp/project".to_string()),
    ..Default::default()
}))?;
# Ok(())
# }
```

Security note: do not hard-code or commit secrets.

## Quickstart

### Start local server + create client

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
            .with_path("sessionID", session_id)
            .with_body(json!({
                "parts": [
                    { "type": "text", "text": "Summarize this repository" }
                ]
            })),
    )
    .await?;

println!("status={} data={}", response.status, response.data);
app.server.close().await?;
# Ok(())
# }
```

### Subscribe to SSE events

```rust,no_run
use futures::StreamExt;
use opencode::{create_opencode_client, RequestOptions};

# async fn run() -> opencode::Result<()> {
let client = create_opencode_client(None)?;
let mut stream = client.global().event(RequestOptions::default()).await?;

while let Some(event) = stream.next().await {
    let event = event?;
    println!("event={:?} data={}", event.event, event.data);
}
# Ok(())
# }
```

## API Selection Guide

| Use case | Recommended API | Why |
| --- | --- | --- |
| Launch local OpenCode and call APIs in one object | `create_opencode` | Returns `Opencode { server, client }` |
| Manage server lifecycle explicitly | `create_opencode_server` + `create_opencode_client` | Fine-grained process control |
| Use endpoint wrappers with readable names | `client.session()/global()/...` methods | Clear, typed namespace access |
| Call by official operation id | `call_operation` / `call_operation_sse` | Close parity with OpenAPI operation naming |
| Consume streaming events | `request_sse` / `global().event()` / `event().subscribe()` | SSE parser and typed `SseEvent` output |

## Core API Surface

- Server and process lifecycle
  - `create_opencode_server`
  - `create_opencode_tui`
  - `create_opencode`
  - `OpencodeServer::close`
  - `OpencodeTui::close`
- Client construction and request primitives
  - `create_opencode_client`
  - `OpencodeClientConfig`
  - `RequestOptions`
  - `ApiResponse`
  - `SseEvent`, `SseStream`
  - `OpencodeClient::request_json`
  - `OpencodeClient::request_sse`
  - `OpencodeClient::call_operation`
  - `OpencodeClient::call_operation_sse`
- Endpoint namespaces
  - `SessionApi`, `GlobalApi`, `AppApi`, `ProjectApi`, `ProviderApi`, `AuthApi`, `OauthApi`
  - `FindApi`, `FileApi`, `PathApi`, `LspApi`, `ToolApi`, `CommandApi`, `ConfigApi`, `FormatterApi`, `VcsApi`, `InstanceApi`
  - `McpApi`, `McpAuthApi`, `PtyApi`, `EventApi`, `TuiApi`, `TuiControlApi` (`ControlApi` alias)
- Input helper types
  - `SessionCreateInput`
  - `PromptInput`
  - `PartInput`

## Feature Highlights

- Path parameter resolution compatible with OpenCode naming variants (`sessionID` / `messageID` / snake_case alternatives)
- Safer multi-parameter route behavior (avoids accidental single `id` fallback substitution)
- SSE parser hardened for streaming edge cases
- Explicit typed error model (`thiserror`) with SDK/HTTP/process/CLI-not-found categories
- Fixture-based tests for server lifecycle, CLI args/env forwarding, HTTP paths, and SSE parsing

## Feature Comparison with Official JavaScript SDK

| Feature | Official JavaScript SDK | This Rust SDK | Notes |
| --- | --- | --- | --- |
| Local server helper (`createOpencode`) | ✅ | ✅ (`create_opencode`) | Same workflow, Rust async style |
| Endpoint namespace access | ✅ | ✅ | Similar surface with Rust naming |
| Operation-id based invocation | ✅ | ✅ | `call_operation` parity-oriented API |
| SSE event subscription | ✅ | ✅ | Rust returns `futures::Stream` of `SseEvent` |
| Directory header support | ✅ | ✅ | `x-opencode-directory` auto-applied |
| Typed error hierarchy | JS Error objects | ✅ (`Error` enum) | Rust uses explicit typed errors |
| Core SDK workflow | ✅ | ✅ | Parity on behavior, Rust-native structure |

> Note: this crate targets behavior parity rather than identical internal structure. Rust-specific API and error modeling are intentionally idiomatic.

## Compatibility Matrix

| Component | Requirement / Notes |
| --- | --- |
| Rust | `1.85+` |
| Edition | `2024` |
| OpenCode CLI | Required at runtime (`opencode`) |
| Runtime | Tokio async runtime |
| HTTP client | Reqwest (`rustls-tls`) |

## Known Limitations

- Behavior depends on installed OpenCode CLI/server version.
- API payloads intentionally use `serde_json::Value` at boundaries for schema-forward compatibility.
- The crate validates parity through fixture/mocked workflows; full production environment matrix is not bundled.

## Testing and Validation

Validation commands:

```bash
cargo fmt --all
cargo check -p opencode-client-sdk
cargo test -p opencode-client-sdk
cargo clippy -p opencode-client-sdk --all-targets --all-features -- -D warnings
```

## Concurrency Model

- Server/TUI lifecycle is process-based (`tokio::process`) and can be managed asynchronously.
- Request APIs are async and cloneable (`OpencodeClient` uses shared inner state).
- SSE APIs produce `Send` streams suitable for async task pipelines.

## Development

- Keep API behavior aligned with official OpenCode SDK semantics.
- Prefer typed errors and explicit request/response handling.
- Add fixture-based tests when changing CLI transport, path rendering, or SSE parsing behavior.

## Contributing

Please follow repository contribution and validation rules in `AGENTS.md`.

## License

Apache-2.0
