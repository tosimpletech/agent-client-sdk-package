# OpenCode Rust SDK

[English](README.md) | [中文](README_zh.md)

通过启动本地 OpenCode CLI 服务并调用其 HTTP/SSE 接口，将 OpenCode 集成到 Rust 应用中。

## 目录

- [概览](#概览)
- [状态](#状态)
- [安装](#安装)
- [认证与环境配置](#认证与环境配置)
- [快速开始](#快速开始)
- [API 选型指南](#api-选型指南)
- [核心 API](#核心-api)
- [关键实现点](#关键实现点)
- [与官方 JavaScript SDK 的特性对比](#与官方-javascript-sdk-的特性对比)
- [兼容性矩阵](#兼容性矩阵)
- [已知限制](#已知限制)
- [测试与验证](#测试与验证)
- [并发模型](#并发模型)
- [开发](#开发)
- [贡献](#贡献)
- [许可证](#许可证)

## 概览

该 crate 是一个以能力对齐为目标的 Rust 实现，行为上对齐官方 OpenCode SDK（`@opencode-ai/sdk` `v1.3.0`），同时保持 Rust 风格的 API 设计。

支持能力：

- 本地服务生命周期管理（`create_opencode_server`、`create_opencode`）
- TUI 进程生命周期管理（`create_opencode_tui`）
- 命名空间客户端入口（`session`、`global`、`project`、`workspace`、`worktree`、`question`、`provider`、`mcp`、`tui` 等）
- 按 `operationId` 的通用调用（`call_operation`、`call_operation_sse`）
- 稳健 SSE 解析（支持 UTF-8 跨 chunk 与尾行未空行终止场景）

## 状态

- 版本：`1.3.0`（`opencode-client-sdk`）
- 范围：覆盖 OpenCode CLI server + SDK HTTP 核心工作流的对齐实现
- 验证：维护了基于 fixture 的子进程与 HTTP/SSE 测试
- 文档：公开 API 已通过 `opencode` 导出并补充 rustdoc

## 安装

当前仓库采用 workspace / 本地路径依赖方式。

```toml
[dependencies]
opencode = { package = "opencode-client-sdk", path = "../../crates/opencode" }
```

运行前提：

- Rust 1.85+（edition 2024）
- 已安装并可访问 OpenCode CLI（`opencode`）

## 认证与环境配置

该 SDK 本质是调用外部 OpenCode CLI/server。认证通常由 OpenCode 运行环境配置负责。

### 方式 A：沿用 CLI/运行环境

直接使用已有 OpenCode 环境配置和登录状态。

### 方式 B：代码中传入 header 与运行参数

```rust,no_run
use std::collections::HashMap;
use opencode::{create_opencode_client, OpencodeClientConfig};

# fn example() -> opencode::Result<()> {
let mut headers = HashMap::new();
headers.insert("x-custom-header".to_string(), "value".to_string());

let _client = create_opencode_client(Some(
    OpencodeClientConfig {
        base_url: "http://127.0.0.1:4096".to_string(),
        headers,
        bearer_token: None,
        directory: Some("/tmp/project".to_string()),
        ..Default::default()
    }
    .with_workspace_id("ws_123"),
))?;
# Ok(())
# }
```

安全提示：不要将密钥硬编码或提交到代码仓库。

## 快速开始

### 启动本地服务并创建客户端

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
                    { "type": "text", "text": "总结这个仓库" }
                ]
            })),
    )
    .await?;

println!("status={} data={}", response.status, response.data);
app.server.close().await?;
# Ok(())
# }
```

### 订阅 SSE 事件

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

## API 选型指南

| 场景 | 推荐 API | 原因 |
| --- | --- | --- |
| 一次获得 server+client 并快速使用 | `create_opencode` | 直接返回 `Opencode { server, client }` |
| 需要细粒度控制服务生命周期 | `create_opencode_server` + `create_opencode_client` | 进程管理更可控 |
| 希望使用可读性更好的命名空间方法 | `client.session()/global()/...` | 类型化命名空间封装 |
| 需要按官方 operation id 调用 | `call_operation` / `call_operation_sse` | 与 OpenAPI 命名高度贴近 |
| 需要流式事件消费 | `request_sse` / `global().event()` / `event().subscribe()` | 返回类型化 `SseEvent` 流 |

## 核心 API

- 服务与进程生命周期
  - `create_opencode_server`
  - `create_opencode_tui`
  - `create_opencode`
  - `OpencodeServer::close`
  - `OpencodeTui::close`
- 客户端构建与请求原语
  - `create_opencode_client`
  - `OpencodeClientConfig`
  - `RequestOptions`
  - `ApiResponse`
  - `SseEvent`、`SseStream`
  - `OpencodeClient::request_json`
  - `OpencodeClient::request_sse`
  - `OpencodeClient::call_operation`
  - `OpencodeClient::call_operation_sse`
- 命名空间接口
  - `SessionApi`、`GlobalApi`、`AppApi`、`ProjectApi`、`ProviderApi`、`AuthApi`、`OauthApi`、`QuestionApi`
  - `FindApi`、`FileApi`、`PathApi`、`LspApi`、`ToolApi`、`CommandApi`、`ConfigApi`、`FormatterApi`、`VcsApi`、`InstanceApi`
  - `WorkspaceApi`、`ResourceApi`、`WorktreeApi`、`ExperimentalApi`、`ExperimentalSessionApi`
  - `PartApi`、`PermissionApi`
  - `McpApi`、`McpAuthApi`、`PtyApi`、`EventApi`、`TuiApi`、`TuiControlApi`（`ControlApi` 别名）
- 输入辅助类型
  - `SessionCreateInput`
  - `PromptInput`
  - `PartInput`

## 关键实现点

- 路径参数解析兼容 OpenCode 命名变体（`sessionID` / `messageID` / snake_case）
- 多参数路径的 `id` 回退更安全（避免误替换）
- SSE 解析覆盖常见流式边界问题
- client config 对齐 `x-opencode-directory` 与 `x-opencode-workspace`
- 补齐 `v1.3.0` 的 `global.upgrade`、`project.init_git`、`file.status`、`question` 以及实验性 workspace/resource/worktree 接口
- 结构化错误模型（SDK/HTTP/进程/CLI-not-found）
- 基于 fixture 的测试覆盖 CLI 参数/环境透传、HTTP 路径、SSE 解析等场景

## 与官方 JavaScript SDK 的特性对比

| 特性 | 官方 JavaScript SDK | 本 Rust SDK | 说明 |
| --- | --- | --- | --- |
| 本地服务一体化启动（`createOpencode`） | ✅ | ✅（`create_opencode`） | 工作流一致，Rust 异步风格 |
| 命名空间接口调用 | ✅ | ✅ | 表面能力一致，命名为 Rust 风格 |
| operation-id 通用调用 | ✅ | ✅ | `call_operation` 以对齐为目标 |
| SSE 事件订阅 | ✅ | ✅ | Rust 返回 `futures::Stream<SseEvent>` |
| 目录/工作区请求头支持 | ✅ | ✅ | 自动附加 `x-opencode-directory` 与 `x-opencode-workspace` |
| 错误模型 | JS Error 对象 | ✅（`Error` enum） | Rust 采用显式类型错误 |
| 核心 SDK 工作流 | ✅ | ✅ | 核心行为已对齐 |

> 说明：本 crate 追求行为对齐，不追求内部结构完全一致。针对 Rust 的 API 组织和错误建模做了必要的语言化设计。

## 兼容性矩阵

| 组件 | 要求 / 说明 |
| --- | --- |
| Rust | `1.85+` |
| Edition | `2024` |
| OpenCode CLI | 运行时必需（`opencode`） |
| Runtime | Tokio 异步运行时 |
| HTTP 客户端 | Reqwest（`rustls-tls`） |

## 已知限制

- SDK 依赖外部 OpenCode CLI/server，行为会受运行时版本影响。
- 为适配 schema 快速演进，边界 payload 主要使用 `serde_json::Value`。
- 当前测试以 fixture/模拟链路为主，不包含完整生产环境矩阵。

## 测试与验证

验证命令：

```bash
cargo fmt --all
cargo check -p opencode-client-sdk
cargo test -p opencode-client-sdk
cargo clippy -p opencode-client-sdk --all-targets --all-features -- -D warnings
```

## 并发模型

- 服务/TUI 生命周期基于异步子进程（`tokio::process`）管理。
- 客户端请求 API 为异步且可 clone（共享内部状态）。
- SSE 接口返回 `Send` 流，适合接入异步任务管道。

## 开发

- 以官方 OpenCode SDK 语义为对齐目标。
- 优先 typed error 与显式请求/响应处理。
- 任何涉及 CLI transport、路径渲染、SSE 解析的改动应补充 fixture 测试。

## 贡献

请遵循仓库 `AGENTS.md` 中的贡献与验证规则。

## 许可证

Apache-2.0
