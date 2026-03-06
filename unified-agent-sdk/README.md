# Unified Agent SDK

Unified Rust SDK that provides one interface for multiple coding agents (currently Codex and Claude Code).

## Table of Contents

- [Overview](#overview)
- [Installation](#installation)
- [Quickstart](#quickstart)
- [API Overview](#api-overview)
- [Examples](#examples)
- [License](#license)

## Overview

`unified-agent-sdk` offers:

- A shared executor interface (`AgentExecutor`) across providers
- Provider adapters (`CodexExecutor`, `ClaudeCodeExecutor`)
- Profile/config resolution (`ProfileManager`)
- Unified event and log normalization pipeline (`AgentEvent`, `LogNormalizer`)
- Context usage signaling with optional capacity/remaining metadata (`ContextUsageUpdated`)

It is designed to keep integration code stable while switching agent backends.

## Installation

This repository currently uses a workspace/local package layout.

```toml
[dependencies]
unified-agent-sdk = { path = "../unified-agent-sdk" }
```

Runtime prerequisites:

- Rust 1.85+ (edition 2024)
- Installed agent CLIs (`codex` and/or `claude`)

## Quickstart

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
        .spawn(&working_dir, "Summarize this repository in 3 bullets.", &config)
        .await?;

    println!("session_id: {}", session.session_id);
    Ok(())
}
```

## API Overview

- Executors
  - `AgentExecutor` trait: `spawn`, `resume`, `capabilities`, `availability`
  - Implementations: `CodexExecutor`, `ClaudeCodeExecutor`
- Profiles
  - `ProfileManager`, `ProfileId`, `ExecutorConfig`
  - Runtime discovery via `discover`, merged config via `resolve`
- Sessions and events
  - `AgentSession` for session metadata and stream pipeline
  - `AgentEvent`, `EventStream`, `HookManager`
- Normalization
  - `CodexLogNormalizer`, `ClaudeCodeLogNormalizer`
  - `LogNormalizer` trait for custom adapters

## Examples

### Resume an existing session

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
        .spawn(&working_dir, "Create a TODO list for this codebase.", &config)
        .await?;

    let resumed = executor
        .resume(
            &working_dir,
            "Now prioritize the TODO list.",
            &first.session_id,
            None,
            &config,
        )
        .await?;

    println!("resumed session: {}", resumed.session_id);
    Ok(())
}
```

### Resolve profile + overrides

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

## License

Licensed under the [Apache License, Version 2.0](../LICENSE).
