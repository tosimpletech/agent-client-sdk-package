# Claude Code Rust SDK (Initial Parity Build)

This crate is a Rust implementation aligned to the Python Claude Agent SDK shape and behavior, focusing on:

- Public API surface equivalence for core use-cases.
- Message parsing and error semantics.
- CLI transport command construction and stream buffering behavior.
- Control protocol callbacks (`can_use_tool`, hooks) and SDK MCP server routing.

## Implemented Public API

- `query(...)` one-shot API
- `query_from_stream(...)` one-shot streamed-input API (`Stream<Item = Value>`)
- `query_stream(...)` one-shot streamed-output API (`Stream<Item = Result<Message>>`)
- `query_stream_from_stream(...)` one-shot streamed input/output API
- `ClaudeSdkClient`
  - `connect`
  - `query`
  - `query_stream`
  - `receive_message`
  - `receive_response`
  - `interrupt`
  - `set_permission_mode`
  - `set_model`
  - `rewind_files`
  - `get_mcp_status`
  - `get_server_info`
  - `disconnect`
- `SubprocessCliTransport` + `Transport` trait
- Message / content / option types (`ClaudeAgentOptions`, `Message`, `UserMessage`, etc.)
- Error types (`ClaudeSDKError`, `CLIConnectionError`, `CLINotFoundError`, `ProcessError`, `CLIJSONDecodeError`, `MessageParseError`)
- SDK MCP primitives (`SdkMcpTool`, `tool`, `create_sdk_mcp_server`, `ToolAnnotations`)

## Test Coverage Mapping (Python -> Rust)

- `test_errors.py` -> `tests/errors_tests.rs`
- `test_types.py` -> `tests/types_tests.rs`
- `test_message_parser.py` -> `tests/message_parser_tests.rs`
- `test_transport.py` (command building subset) -> `tests/transport_command_tests.rs`
- `test_subprocess_buffering.py` -> `tests/buffering_tests.rs`
- `test_tool_callbacks.py` (control callback subset) -> `tests/query_callbacks_tests.rs`
- `test_sdk_mcp_integration.py` (server/tool/annotation subset) -> `tests/sdk_mcp_tests.rs`
- `test_streaming_client.py` / `test_client.py` (core client flow subset) -> `tests/client_tests.rs`
- Streamed one-shot API coverage -> `tests/query_stream_api_tests.rs`
- Dynamic control / stderr / partial messages / SDK MCP subprocess e2e coverage -> `tests/e2e_subprocess_mock_tests.rs`

Current status: all tests pass with `cargo test`.

## Rust 2024 and Library Practices

- Crate uses `edition = "2024"` and sets `rust-version = "1.85"` (minimum stable supporting edition 2024).
- `tokio` uses explicit granular features (`io-util`, `macros`, `process`, `rt-multi-thread`, `sync`, `time`) instead of `full`, to keep dependency surface smaller while covering required SDK behavior.
- `serde` / `serde_json` are used with derive + explicit field attributes (`rename`, `rename_all`, optional fields) for protocol compatibility.
- `thiserror` is used for structured error types and transparent wrapper behavior where appropriate.
- Codebase is kept clippy-clean under strict mode: `cargo clippy --all-targets --all-features -- -D warnings`.

## Concurrency Model

- `Query::start()` spawns a background tokio task that reads from the transport, routes control responses via oneshot channels, handles incoming control requests (permissions, hooks, MCP), and delivers SDK messages via an mpsc channel. This mirrors the Python SDK's task-group model.
- One-shot streaming APIs (`query_stream`, `query_stream_from_stream`) return `BoxStream` which is `Send`, safe to consume from any tokio task.
- `ClaudeSdkClient` supports reconnect after disconnect when constructed with a `TransportFactory`. Methods like `query()`, `interrupt()`, `set_model()` take `&self` for concurrent use from different tasks.

## Functional Differences vs Python SDK

| Area | Python SDK | Rust SDK (this crate) | Notes |
| --- | --- | --- | --- |
| Public one-shot query API | `query(...)` async iterator + AsyncIterable input | `query(...)` (collected), `query_from_stream(...)`, `query_stream(...)`, `query_stream_from_stream(...)` | Supports Rust `Stream`-based input/output interfaces in addition to collected mode. |
| Interactive client | `ClaudeSDKClient` with connect/query/receive/interrupt/model/permission controls | `ClaudeSdkClient` with equivalent core methods | Core control flow is implemented and tested. |
| Message model parsing | Full typed parsing for user/assistant/system/result/stream_event | Same core message categories implemented | Unknown message type is skipped for forward compatibility. |
| CLI transport command composition | Rich options + settings/sandbox merge + stream-json | Same key behavior implemented | Matches major flags and merge behavior tested in Rust. |
| Stream buffering robustness | Extensive buffering tests | Equivalent split/concat/size-limit parser tests | Core failure mode (`max_buffer_size`) covered. |
| Hook callback protocol | Implemented | Implemented (including `async_`/`continue_` field conversion) | Conversion behavior covered by tests. |
| Tool permission callback | Implemented (`can_use_tool`) | Implemented | Allow/deny paths and payload conversion tested. |
| SDK MCP in-process server | Implemented | Implemented (tool/list/call + annotations) | JSON-RPC bridge behavior covered for core methods. |
| Python-specific runtime integration | Native async ecosystem (anyio/trio/asyncio nuances) | Tokio-based runtime integration | Rust version follows idiomatic Tokio model. |
| Full parity with Python e2e suite | Full in source repo | Partial (parity-focused subset + subprocess protocol e2e layer) | Remaining work is mainly real-CLI/live-model end-to-end parity expansion. |
