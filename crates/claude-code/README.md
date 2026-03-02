# Claude Code Rust SDK

[English](README.md) | [中文](README_zh.md)

Rust SDK for integrating Claude Code as a subprocess and interacting with it through typed Rust APIs.

## Table of Contents

- [Overview](#overview)
- [Status](#status)
- [Installation](#installation)
- [Authentication and Environment Setup](#authentication-and-environment-setup)
- [Quickstart](#quickstart)
- [API Selection Guide](#api-selection-guide)
- [Core API Surface](#core-api-surface)
- [Feature Highlights](#feature-highlights)
- [Feature Comparison with Official Python SDK](#feature-comparison-with-official-python-sdk)
- [Compatibility Matrix](#compatibility-matrix)
- [Known Limitations](#known-limitations)
- [Testing and Validation](#testing-and-validation)
- [Concurrency Model](#concurrency-model)
- [Development](#development)
- [Contributing](#contributing)
- [License](#license)

## Overview

This crate is a parity-focused Rust implementation aligned with the Python Claude Agent SDK semantics.

It supports:

- One-shot query APIs (`query`, `query_stream`, and stream-input variants)
- Session-based multi-turn workflows with `ClaudeSdkClient`
- Typed message parsing and transport abstractions
- Tool permission callbacks, hooks, and SDK MCP integration
- CLI transport command construction and robust stream buffering behavior

## Status

- Package version: `0.1.0` (`claude-code-client-sdk`)
- Scope: parity-focused implementation of the core Python SDK workflow
- Validation: parity-focused test suite and subprocess/e2e-style coverage in this crate
- Rust docs: public API is documented and exported through `claude_code`

## Installation

This repository currently uses a workspace/local package layout.

```toml
[dependencies]
claude_code = { package = "claude-code-client-sdk", path = "../../crates/claude-code" }
```

Runtime prerequisites:

- Rust 1.85+ (edition 2024)
- Claude Code CLI installed and accessible in the runtime environment

## Authentication and Environment Setup

This SDK invokes the Claude Code CLI. Authentication can come from existing CLI login/session or environment variables passed to the process.

### Option A: environment variables

```bash
# Example provider key variable used by Claude tooling
export ANTHROPIC_API_KEY="<your_api_key>"
```

### Option B: per-client environment overrides

```rust,no_run
use std::collections::HashMap;
use claude_code::{ClaudeAgentOptions, ClaudeSdkClient};

# fn example() {
let mut env = HashMap::new();
env.insert("ANTHROPIC_API_KEY".to_string(), "<your_api_key>".to_string());

let options = ClaudeAgentOptions {
    env,
    ..Default::default()
};

let _client = ClaudeSdkClient::new(Some(options), None);
# }
```

Security note: do not hard-code or commit secrets to source control.

## Quickstart

### One-shot query

```rust,no_run
use claude_code::{query, ClaudeAgentOptions, InputPrompt, Message, PermissionMode};

# async fn example() -> claude_code::Result<()> {
let messages = query(
    InputPrompt::Text("Explain Rust ownership in two bullets".to_string()),
    Some(ClaudeAgentOptions {
        permission_mode: Some(PermissionMode::BypassPermissions),
        max_turns: Some(1),
        ..Default::default()
    }),
    None,
)
.await?;

for msg in messages {
    if let Message::Assistant(assistant) = msg {
        println!("{:?}", assistant.content);
    }
}
# Ok(())
# }
```

### Session-based multi-turn conversation

```rust,no_run
use claude_code::{ClaudeSdkClient, InputPrompt};

# async fn example() -> claude_code::Result<()> {
let mut client = ClaudeSdkClient::new(None, None);
client.connect(None).await?;

client
    .query(InputPrompt::Text("What's the capital of France?".into()), "default")
    .await?;
let _first = client.receive_response().await?;

client
    .query(
        InputPrompt::Text("What's the population of that city?".into()),
        "default",
    )
    .await?;
let second = client.receive_response().await?;

println!("messages in second response: {}", second.len());
client.disconnect().await?;
# Ok(())
# }
```

## API Selection Guide

| Use case | Recommended API | Why |
| --- | --- | --- |
| One-off request, collect all messages | `query` | Simplest single-call API |
| One-off request, consume incrementally | `query_stream` | Stream responses as they arrive |
| Stream input messages to one-off query | `query_from_stream` / `query_stream_from_stream` | Rust equivalent of async iterable input |
| Multi-turn conversation with session control | `ClaudeSdkClient` | Explicit connect/query/receive/interrupt lifecycle |

## Core API Surface

- One-shot APIs
  - `query`
  - `query_from_stream`
  - `query_stream`
  - `query_stream_from_stream`
- Session client
  - `ClaudeSdkClient::connect`
  - `ClaudeSdkClient::connect_with_messages`
  - `ClaudeSdkClient::query`
  - `ClaudeSdkClient::receive_message`
  - `ClaudeSdkClient::receive_response`
  - `ClaudeSdkClient::interrupt`
  - `ClaudeSdkClient::disconnect`
  - control methods such as `set_permission_mode`, `set_model`, `rewind_files`, `get_mcp_status`, `get_server_info`
- Transport layer
  - `SubprocessCliTransport`
  - `Transport` trait and split helpers
- SDK MCP primitives
  - `SdkMcpTool`, `tool`, `create_sdk_mcp_server`, `ToolAnnotations`
- Message/config types
  - `ClaudeAgentOptions`, `Message`, and typed content/message variants

## Feature Highlights

- Buffered JSON stream parser with size-limit protection
- Structured error taxonomy for transport/process/message parsing
- Callback protocol support (`can_use_tool`, hooks)
- SDK MCP server routing for in-process tool invocation
- Reconnect-capable client flow when used with a `TransportFactory`

## Feature Comparison with Official Python SDK

| Feature | Official Python SDK | This Rust SDK | Notes |
| --- | --- | --- | --- |
| One-shot query API | ✅ | ✅ | `query` parity for core workflow |
| Stream input support | ✅ (`AsyncIterable`) | ✅ (`Stream<Item = Value>`) | Rust-idiomatic streaming input |
| Stream output support | ✅ | ✅ | `query_stream` / `query_stream_from_stream` |
| Session client | ✅ (`ClaudeSDKClient`) | ✅ (`ClaudeSdkClient`) | Connect/query/receive/interrupt lifecycle |
| Hook callbacks | ✅ | ✅ | Core hook callback protocol covered |
| Tool permission callback (`can_use_tool`) | ✅ | ✅ | Includes typed context/result conversion |
| SDK MCP integration | ✅ | ✅ | Full in-process server routing supported |
| All message types | ✅ | ✅ | User/Assistant/System/Result/StreamEvent |
| All content block types | ✅ | ✅ | Text/Thinking/ToolUse/ToolResult |
| Permission types | ✅ | ✅ | Full PermissionUpdate/PermissionResult types |
| Sandbox configuration | ✅ | ✅ | Full SandboxSettings/NetworkConfig types |
| Agent definitions | ✅ | ✅ | AgentDefinition with tools/model options |
| Hook input types | ✅ (TypedDict) | ✅ (`Value`) | Rust uses raw JSON for flexibility |
| Runtime model | Python async runtimes | Tokio | Runtime model differs by language |
| Core SDK workflow | ✅ | ✅ | Full parity for all core use cases |

> **Note**: This Rust SDK achieves full core parity with the official Python SDK. The design choice to use `Value` (raw JSON) for hook input types instead of strongly-typed discriminated unions provides flexibility while still allowing users to deserialize into their own types if needed. For strongly-typed hook inputs, users can define their own Rust types and use `serde_json::from_value()`.

## Compatibility Matrix

| Component | Requirement / Notes |
| --- | --- |
| Rust | `1.85+` |
| Edition | `2024` |
| Claude Code CLI | Required in runtime environment |
| Runtime | Tokio async runtime |
| OS support | Follows CLI support matrix |

## Known Limitations

- SDK behavior depends on the installed Claude Code CLI version.
- E2E coverage is parity-focused; not every upstream integration scenario is replicated.
- CLI-specific auth/deployment flows are managed by the underlying CLI, not by this crate.

## Testing and Validation

Reference alignment coverage (Python -> Rust):

- `test_errors.py` -> `tests/errors_tests.rs`
- `test_types.py` -> `tests/types_tests.rs`
- `test_message_parser.py` -> `tests/message_parser_tests.rs`
- `test_transport.py` (command-building subset) -> `tests/transport_command_tests.rs`
- `test_subprocess_buffering.py` -> `tests/buffering_tests.rs`
- `test_tool_callbacks.py` (callback subset) -> `tests/query_callbacks_tests.rs`
- `test_sdk_mcp_integration.py` (core subset) -> `tests/sdk_mcp_tests.rs`
- `test_streaming_client.py` / `test_client.py` (core flow subset) -> `tests/client_tests.rs`
- stream API coverage -> `tests/query_stream_api_tests.rs`
- subprocess/e2e protocol coverage -> `tests/e2e_subprocess_mock_tests.rs`

Validation commands:

```bash
cargo test -p claude-code-client-sdk
cargo clippy -p claude-code-client-sdk --all-targets --all-features -- -D warnings
```

## Concurrency Model

- `Query::start()` runs a background task that:
  - reads transport messages
  - routes control responses
  - handles callback/MCP requests
  - emits typed SDK messages through channels
- One-shot streaming APIs return `Send` streams
- `ClaudeSdkClient` supports concurrent control/query calls after connection (`&self` methods)

## Development

```bash
cargo test -p claude-code-client-sdk
cargo clippy -p claude-code-client-sdk --all-targets --all-features -- -D warnings
```

## Contributing

Pull requests are welcome. Before submitting, run:

```bash
cargo fmt
cargo clippy -p claude-code-client-sdk --all-targets --all-features -- -D warnings
cargo test -p claude-code-client-sdk
```

## License

License information has not been declared in this repository yet. Add a root `LICENSE` file before external distribution.
