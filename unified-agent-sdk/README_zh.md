# Unified Agent SDK

[English](README.md) | [中文](README_zh.md)

统一的 Rust SDK，为多个 coding agent 提供同一套接入接口。

## 目录

- [概览](#概览)
- [支持的 Provider](#支持的-provider)
- [安装](#安装)
- [快速开始](#快速开始)
- [API 概览](#api-概览)
- [示例](#示例)
- [许可证](#许可证)

## 概览

`unified-agent-sdk` 提供：

- 统一的执行器接口 `AgentExecutor`，屏蔽不同 provider 的调用差异
- 基于 provider 的模块化架构：每个 SDK 在独立模块中封装 executor 和 normalizer
- 配置与 profile 解析能力（`ProfileManager`）
- 统一事件与日志归一化流水线（`AgentEvent`、`LogNormalizer`）
- 上下文使用量信号，支持可选的总容量与剩余容量元数据（`ContextUsageUpdated`）

它的目标是在切换 agent 后端时，尽可能保持上层集成代码稳定。

## 支持的 Provider

| Provider | 执行器 | 依赖 CLI |
| --- | --- | --- |
| Codex | `CodexExecutor` | `codex` |
| Claude Code | `ClaudeCodeExecutor` | `claude` |

## 安装

```toml
[dependencies]
unified-agent-sdk = "0.1.0"
```

运行前提：

- Rust 1.85+（edition 2024）
- 已安装所需 agent CLI（`codex` 和/或 `claude`）

## 快速开始

```rust,no_run
use unified_agent_sdk::{
    AgentExecutor, CodexExecutor, PermissionPolicy, Result,
    executor::SpawnConfig,
};

#[tokio::main]
async fn main() -> Result<()> {
    let executor = CodexExecutor::default();
    let config = SpawnConfig {
        model: Some("gpt-5-codex".to_string()),
        reasoning: Some("medium".to_string()),
        permission_policy: Some(PermissionPolicy::Prompt),
        env: vec![],
        context_window_override_tokens: None,
    };

    let working_dir = std::env::current_dir()?;
    let session = executor
        .spawn(&working_dir, "用 3 个要点总结这个仓库。", &config)
        .await?;

    println!("session_id: {}", session.session_id);
    Ok(())
}
```

## API 概览

- 执行器
  - `AgentExecutor` trait：`spawn`、`resume`、`capabilities`、`availability`
  - 实现类型：`CodexExecutor`、`ClaudeCodeExecutor`
- Profiles
  - `ProfileManager`、`ProfileId`、`ExecutorConfig`
  - 通过 `discover` 发现运行时配置，通过 `resolve` 获取合并后的配置
- 会话与事件
  - `AgentSession`：封装会话元信息、生命周期控制（`wait` / `cancel`）和事件流
  - `AgentEvent`、`EventStream`、`HookManager`
- 归一化
  - `CodexLogNormalizer`、`ClaudeCodeLogNormalizer`
  - `LogNormalizer` trait，可扩展自定义适配器

## 示例

### 恢复已有会话

```rust,no_run
use unified_agent_sdk::{
    AgentExecutor, ClaudeCodeExecutor, Result,
    executor::SpawnConfig,
};

#[tokio::main]
async fn main() -> Result<()> {
    let executor = ClaudeCodeExecutor::new();
    let config = SpawnConfig {
        model: None,
        reasoning: None,
        permission_policy: None,
        env: vec![],
        context_window_override_tokens: None,
    };

    let working_dir = std::env::current_dir()?;
    let first = executor
        .spawn(&working_dir, "为这个代码库整理一份 TODO 列表。", &config)
        .await?;

    let resumed = executor
        .resume(
            &working_dir,
            "现在给这份 TODO 列表排优先级。",
            &first.session_id,
            None,
            &config,
        )
        .await?;

    println!("resumed session: {}", resumed.session_id);
    Ok(())
}
```

### 解析 profile 并应用覆盖项

```rust,no_run
use unified_agent_sdk::{
    ExecutorConfig, ExecutorType, PermissionPolicy, ProfileId, ProfileManager, Result,
};

#[tokio::main]
async fn main() -> Result<()> {
    let manager = ProfileManager::new();
    let config = ExecutorConfig {
        profile_id: ProfileId::new(ExecutorType::Codex, Some("plan".to_string())),
        model_override: None,
        reasoning_override: Some("low".to_string()),
        permission_policy: Some(PermissionPolicy::Prompt),
    };

    let resolved = manager.resolve(&config).await?;
    println!("model={:?}, reasoning={:?}", resolved.model, resolved.reasoning);
    Ok(())
}
```

## 许可证

采用 [Apache License, Version 2.0](../LICENSE) 许可证。
