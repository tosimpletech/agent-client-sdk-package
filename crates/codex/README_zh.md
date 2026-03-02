# Codex Rust SDK

[English](README.md) | [中文](README_zh.md)

通过 `codex` CLI（`codex exec --experimental-json`）的 JSONL 通道，将 Codex agent 以 Rust SDK 方式集成到应用中。

## 概览

该 crate 是一个以能力对齐为目标的 Rust 实现，语义上与官方 Codex TypeScript SDK 保持一致。

支持能力：

- 基于线程的多轮会话（`start_thread`、`resume_thread`）
- 缓冲与流式两种执行方式（`run`、`run_streamed`）
- 基于 JSON Schema 的结构化输出（`--output-schema`）
- 多模态输入（文本 + 本地图片）
- 取消机制与线程恢复
- CLI 配置与环境透传（`--config`、API 地址/密钥、sandbox/approval/web-search）

## 状态

- 范围：覆盖 Codex 核心工作流的对齐实现
- 验证：测试通过（`cargo test -p codex-client-sdk`）
- 文档：公开 API 已补齐 rustdoc，并可通过 `missing_docs` 检查

## 安装

当前仓库采用 workspace / 本地路径依赖方式。

```toml
[dependencies]
codex = { package = "codex-client-sdk", path = "../../crates/codex" }
```

运行前提：

- Rust 1.85+（edition 2024）
- 已安装并可访问 Codex CLI（`codex`，通常来自 `@openai/codex`）

## 快速开始

```rust,no_run
use codex::Codex;

# async fn example() -> codex::Result<()> {
let codex = Codex::new(None)?;
let thread = codex.start_thread(None);

let turn = thread
    .run("Diagnose the test failure and propose a fix", None)
    .await?;

println!("final response: {}", turn.final_response);
println!("items: {}", turn.items.len());
# Ok(())
# }
```

### 在同一线程继续对话

```rust,no_run
# use codex::Codex;
# async fn example() -> codex::Result<()> {
# let codex = Codex::new(None)?;
# let thread = codex.start_thread(None);
let _first = thread.run("Diagnose failure", None).await?;
let second = thread.run("Implement the fix", None).await?;
println!("{}", second.final_response);
# Ok(())
# }
```

### 流式消费事件

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

### 结构化输出

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

## 核心 API

- `Codex`
  - `new`
  - `start_thread`
  - `resume_thread`
- `Thread`
  - `id`
  - `run`
  - `run_streamed`
- 输入类型
  - `Input`（`Text`、`Entries`）
  - `UserInput`（`Text`、`LocalImage`）
- 配置
  - `CodexOptions`
  - `ThreadOptions`
  - `TurnOptions`
- 事件与条目模型
  - `ThreadEvent` 及其类型化 payload
  - `ThreadItem` 及其类型化 payload
- 底层执行
  - `CodexExec`
  - `CodexExecArgs`

## 关键实现点

- CLI 路径发现较健壮（PATH、本地 `node_modules`、vendor、常见全局目录）
- JSONL 事件/条目均采用强类型反序列化
- `--output-schema` 临时文件生命周期自动管理
- `config` 对象可展开为重复的 TOML 兼容 `--config` 参数
- 对重叠选项有明确优先级（例如 `web_search_mode` 高于 `web_search_enabled`）

## 测试与验证

参考对齐映射（TypeScript -> Rust）：

- `tests/run.test.ts` -> `tests/run_tests.rs`
- `tests/runStreamed.test.ts` -> `tests/run_streamed_tests.rs`
- `tests/exec.test.ts` -> `tests/exec_tests.rs`
- `tests/abort.test.ts` -> `tests/abort_tests.rs`

验证命令：

```bash
RUSTDOCFLAGS='-Dwarnings -Dmissing_docs' cargo doc -p codex-client-sdk --no-deps
cargo test -p codex-client-sdk
```

## 并发模型

- `run_streamed()` 返回 `Send` 的 `ThreadEvent` 流
- `run()` 通过消费事件流构建最终 `Turn`
- 取消机制基于 `tokio_util::sync::CancellationToken`

## 与 TypeScript SDK 的差异

| 维度 | TypeScript SDK | Rust SDK |
| --- | --- | --- |
| 流式返回类型 | `AsyncGenerator<ThreadEvent>` | `Stream<Item = Result<ThreadEvent>>` |
| 取消原语 | `AbortSignal` | `CancellationToken` |
| 输入形态 | `string | UserInput[]` | `Input` enum |
| 线程 ID 访问 | `thread.id` getter | `thread.id()` method |

## 开发

```bash
cargo test -p codex-client-sdk
cargo clippy -p codex-client-sdk --all-targets --all-features -- -D warnings
```
