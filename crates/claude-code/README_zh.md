# Claude Code Rust SDK

[English](README.md) | [中文](README_zh.md)

通过类型化 Rust API 将 Claude Code 以子进程方式集成到应用中。

## 概览

该 crate 是一个以能力对齐为目标的 Rust 实现，语义上与 Python Claude Agent SDK 保持一致。

支持能力：

- 一次性查询 API（`query`、`query_stream` 及流输入变体）
- 基于 `ClaudeSdkClient` 的多轮会话
- 类型化消息解析与传输层抽象
- 工具权限回调、hooks 与 SDK MCP 集成
- CLI 传输命令构建与稳健的流缓冲处理

## 状态

- 范围：覆盖 Python SDK 核心工作流的对齐实现
- 验证：包含对齐测试与 subprocess/e2e 风格测试
- 文档：公开 API 已通过 `claude_code` 导出并附带文档

## 安装

当前仓库采用 workspace / 本地路径依赖方式。

```toml
[dependencies]
claude_code = { package = "claude-code-client-sdk", path = "../../crates/claude-code" }
```

运行前提：

- Rust 1.85+（edition 2024）
- 运行环境可访问 Claude Code CLI

## 快速开始

### 一次性查询

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

### 会话式多轮对话

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

## 核心 API

- 一次性查询 API
  - `query`
  - `query_from_stream`
  - `query_stream`
  - `query_stream_from_stream`
- 会话客户端
  - `ClaudeSdkClient::connect`
  - `ClaudeSdkClient::connect_with_messages`
  - `ClaudeSdkClient::query`
  - `ClaudeSdkClient::receive_message`
  - `ClaudeSdkClient::receive_response`
  - `ClaudeSdkClient::interrupt`
  - `ClaudeSdkClient::disconnect`
  - 控制类方法：`set_permission_mode`、`set_model`、`rewind_files`、`get_mcp_status`、`get_server_info`
- 传输层
  - `SubprocessCliTransport`
  - `Transport` trait 及 split 辅助
- SDK MCP 原语
  - `SdkMcpTool`、`tool`、`create_sdk_mcp_server`、`ToolAnnotations`
- 消息与配置类型
  - `ClaudeAgentOptions`、`Message` 以及相关类型化内容结构

## 关键实现点

- 带容量保护的 JSON 流缓冲解析
- 覆盖传输/进程/消息解析的结构化错误体系
- 回调协议支持（`can_use_tool`、hooks）
- 面向进程内工具调用的 SDK MCP 路由
- 使用 `TransportFactory` 时支持断开后重连

## 测试与验证

参考对齐映射（Python -> Rust）：

- `test_errors.py` -> `tests/errors_tests.rs`
- `test_types.py` -> `tests/types_tests.rs`
- `test_message_parser.py` -> `tests/message_parser_tests.rs`
- `test_transport.py`（命令构建子集）-> `tests/transport_command_tests.rs`
- `test_subprocess_buffering.py` -> `tests/buffering_tests.rs`
- `test_tool_callbacks.py`（回调子集）-> `tests/query_callbacks_tests.rs`
- `test_sdk_mcp_integration.py`（核心子集）-> `tests/sdk_mcp_tests.rs`
- `test_streaming_client.py` / `test_client.py`（核心流程子集）-> `tests/client_tests.rs`
- 流 API 覆盖 -> `tests/query_stream_api_tests.rs`
- subprocess/e2e 协议覆盖 -> `tests/e2e_subprocess_mock_tests.rs`

验证命令：

```bash
cargo test -p claude-code-client-sdk
cargo clippy -p claude-code-client-sdk --all-targets --all-features -- -D warnings
```

## 并发模型

- `Query::start()` 会启动后台任务，负责：
  - 读取传输消息
  - 路由控制响应
  - 处理 callback / MCP 请求
  - 通过 channel 输出类型化 SDK 消息
- 一次性流式 API 返回 `Send` 流
- `ClaudeSdkClient` 连接后支持并发控制/查询调用（`&self` 方法）

## 与 Python SDK 的差异

| 维度 | Python SDK | Rust SDK |
| --- | --- | --- |
| 一次性流式接口 | async iterables | `futures::Stream`（`BoxStream`） |
| 运行时模型 | Python async runtimes | Tokio |
| 客户端流输入连接 | `connect(AsyncIterable)` | `connect_with_messages(Stream<Item = Value>)` |
| 传输对象模型 | 动态协议对象 | 强类型 Rust enum/struct |

## 开发

```bash
cargo test -p claude-code-client-sdk
cargo clippy -p claude-code-client-sdk --all-targets --all-features -- -D warnings
```
