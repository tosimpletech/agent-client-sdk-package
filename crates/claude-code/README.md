# Claude Code Rust SDK (Initial Parity Build)

This crate is a Rust implementation aligned to the Python Claude Agent SDK shape and behavior, focusing on:

- Public API surface equivalence for core use-cases.
- Message parsing and error semantics.
- CLI transport command construction and stream buffering behavior.
- Control protocol callbacks (`can_use_tool`, hooks) and SDK MCP server routing.

## Implemented Public API

- `query(...)` one-shot API
- `ClaudeSdkClient`
  - `connect`
  - `query`
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

Current status: all tests pass with `cargo test`.

