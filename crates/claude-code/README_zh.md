# Claude Code Rust SDK

[English](README.md) | [中文](README_zh.md)

通过类型化 Rust API 将 Claude Code 以子进程方式集成到应用中。

## 目录

- [概览](#概览)
- [状态](#状态)
- [安装](#安装)
- [认证与环境配置](#认证与环境配置)
- [快速开始](#快速开始)
- [API 选型指南](#api-选型指南)
- [核心 API](#核心-api)
- [关键实现点](#关键实现点)
- [与官方 Python SDK 的特性对比](#与官方-python-sdk-的特性对比)
- [兼容性矩阵](#兼容性矩阵)
- [已知限制](#已知限制)
- [测试与验证](#测试与验证)
- [并发模型](#并发模型)
- [开发](#开发)
- [贡献](#贡献)
- [许可证](#许可证)

## 概览

该 crate 是一个以能力对齐为目标的 Rust 实现，语义上与 Python Claude Agent SDK 保持一致。

支持能力：

- 一次性查询 API（`query`、`query_stream` 及流输入变体）
- 基于 `ClaudeSdkClient` 的多轮会话
- 类型化消息解析与传输层抽象
- 工具权限回调、hooks 与 SDK MCP 集成
- CLI 传输命令构建与稳健的流缓冲处理

## 状态

- 版本：`0.1.0`（`claude-code-client-sdk`）
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

## 认证与环境配置

该 SDK 本质上是调用 Claude Code CLI。认证可来自 CLI 已登录状态，或通过环境变量传递给 CLI 进程。

### 方式 A：环境变量

```bash
# Claude 工具链常用密钥环境变量示例
export ANTHROPIC_API_KEY="<your_api_key>"
```

### 方式 B：在客户端配置中传入环境变量

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

安全提示：不要将密钥硬编码或提交到代码仓库。

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

## API 选型指南

| 场景 | 推荐 API | 原因 |
| --- | --- | --- |
| 一次请求并收集全部消息 | `query` | 最简单的一次性调用 |
| 一次请求并增量消费输出 | `query_stream` | 边到边处理消息 |
| 一次请求但输入本身是流 | `query_from_stream` / `query_stream_from_stream` | 对应 Python AsyncIterable 场景 |
| 需要多轮会话与中断控制 | `ClaudeSdkClient` | 显式 connect/query/receive/interrupt 生命周期 |

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

## 与官方 Python SDK 的特性对比

| 特性 | 官方 Python SDK | 本 Rust SDK | 说明 |
| --- | --- | --- | --- |
| 一次性查询 API | ✅ | ✅ | `query` 核心语义对齐 |
| 流输入支持 | ✅（`AsyncIterable`） | ✅（`Stream<Item = Value>`） | Rust 异步流等价形态 |
| 流输出支持 | ✅ | ✅ | `query_stream` / `query_stream_from_stream` |
| 会话客户端 | ✅（`ClaudeSDKClient`） | ✅（`ClaudeSdkClient`） | connect/query/receive/interrupt 生命周期 |
| Hook 回调 | ✅ | ✅ | 核心回调协议已覆盖 |
| 工具权限回调（`can_use_tool`） | ✅ | ✅ | 含上下文与结果类型转换 |
| SDK MCP 集成 | ✅ | ✅ | 进程内 server 路由完整支持 |
| 全部消息类型 | ✅ | ✅ | User/Assistant/System/Result/StreamEvent |
| 全部内容块类型 | ✅ | ✅ | Text/Thinking/ToolUse/ToolResult |
| 权限类型体系 | ✅ | ✅ | 完整 `PermissionUpdate` / `PermissionResult` |
| 沙箱配置 | ✅ | ✅ | 完整 `SandboxSettings` / `NetworkConfig` |
| Agent 定义 | ✅ | ✅ | `AgentDefinition`（含 tools/model） |
| Hook 输入类型 | ✅（TypedDict） | ✅（`Value`） | Rust 采用原始 JSON 以保持灵活性 |
| 运行时模型 | Python async runtimes | Tokio | 语言生态差异 |
| 核心 SDK 工作流 | ✅ | ✅ | 核心用例已实现完整对齐 |

> **说明**：该 Rust SDK 与官方 Python SDK 已实现核心能力完整对齐。Hook 输入类型使用 `Value`（原始 JSON）而非强类型判别联合体，是有意的设计选择：保持灵活性的同时，用户仍可按需通过 `serde_json::from_value()` 反序列化为自定义强类型。

## 兼容性矩阵

| 组件 | 要求 / 说明 |
| --- | --- |
| Rust | `1.85+` |
| Edition | `2024` |
| Claude Code CLI | 运行时必需 |
| Runtime | Tokio 异步运行时 |
| 操作系统 | 依赖 CLI 支持矩阵 |

## 已知限制

- SDK 依赖外部 CLI，最终行为会受安装的 CLI 版本影响。
- E2E 覆盖为对齐优先，并未复制全部上游集成场景。
- 认证/部署细节由底层 CLI 管理，不由本 crate 直接实现。

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

## 开发

```bash
cargo test -p claude-code-client-sdk
cargo clippy -p claude-code-client-sdk --all-targets --all-features -- -D warnings
```

## 贡献

欢迎提交 PR。提交前请执行：

```bash
cargo fmt
cargo clippy -p claude-code-client-sdk --all-targets --all-features -- -D warnings
cargo test -p claude-code-client-sdk
```

## 许可证

本项目采用 [Apache License, Version 2.0](../../LICENSE) 许可。
