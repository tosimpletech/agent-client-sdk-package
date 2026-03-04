# Unified Agent SDK

[English](README.md)

Unified Rust SDK providing a common abstraction layer for multiple AI coding agents (Claude Code, Codex, etc.).

## Table of Contents

- [Overview](#overview)
- [Status](#status)
- [Installation](#installation)
- [Core Concepts](#core-concepts)
- [Architecture](#architecture)
- [Design Principles](#design-principles)
- [Development](#development)
- [Contributing](#contributing)
- [License](#license)

## Overview

This SDK provides a unified interface for interacting with different AI coding agents through common abstractions:

- **Unified Executor Interface** - Single trait for all agents
- **Profile System** - Configuration management with presets and overrides
- **Log Normalization** - Standardized log format across agents
- **Event System** - Unified events with hook support
- **Session Management** - Session metadata for persistence

## Status

- Package version: `0.1.0` (`unified-agent-sdk`)
- Scope: Framework complete, implementation in progress
- Validation: Core traits and types defined, adapters pending

## Installation

This repository currently uses a workspace/local package layout.

```toml
[dependencies]
unified-agent-sdk = { path = "../unified-agent-sdk" }
```

Runtime prerequisites:

- Rust 1.85+ (edition 2024)
- Underlying agent CLIs (Codex, Claude Code) as needed

## Core Concepts

### Executor

The `AgentExecutor` trait provides a unified interface:

```rust
use unified_agent_sdk::{AgentExecutor, SpawnConfig};
use std::path::Path;

async fn example(executor: &dyn AgentExecutor) {
    let config = SpawnConfig {
        model: Some("gpt-4".into()),
        reasoning: None,
        permission_policy: None,
        env: vec![],
    };

    let session = executor.spawn(
        Path::new("/workspace"),
        "Implement feature X",
        &config,
    ).await?;
}
```

### Profile System

Manage configurations with presets and runtime overrides:

```rust
use unified_agent_sdk::{ProfileManager, ExecutorConfig, ProfileId, ExecutorType};

let manager = ProfileManager::new();
let config = ExecutorConfig {
    profile_id: ProfileId::new(ExecutorType::Codex, Some("PLAN".into())),
    model_override: Some("gpt-4".into()),
    reasoning_override: None,
    permission_policy: None,
};

let resolved = manager.resolve(&config)?;
```

### Log System

Normalized logs with pluggable storage:

```rust
use unified_agent_sdk::{LogStorage, NormalizedLog};

#[async_trait]
impl LogStorage for MyStorage {
    async fn store_raw(&self, session_id: &str, chunk: &[u8]) -> Result<()> {
        // Store raw logs
    }

    async fn store_normalized(&self, session_id: &str, log: &NormalizedLog) -> Result<()> {
        // Store normalized logs
    }
}
```

### Event System

Subscribe to agent events with hooks:

```rust
use unified_agent_sdk::{HookManager, EventType, AgentEvent};
use std::sync::Arc;

let hooks = HookManager::new();
hooks.register(EventType::ToolCallStarted, Arc::new(|event| {
    Box::pin(async move {
        println!("Tool called: {:?}", event);
    })
}));

hooks.trigger(&event).await;
```

## Architecture

```
Application Layer
       ↓
Unified Agent SDK (this crate)
  - AgentExecutor trait
  - ProfileManager
  - LogNormalizer + LogStorage
  - EventStream + HookManager
       ↓
Adapters (to be implemented)
  - CodexAdapter
  - ClaudeCodeAdapter
       ↓
Base SDKs
  - codex-sdk
  - claude-code-sdk
```

## Design Principles

- **Minimal Abstraction**: Only abstract what's necessary
- **Pluggable**: Storage and normalization are pluggable
- **No Lifecycle Management**: SDK doesn't manage workspace or persistence
- **Preserve Raw Data**: Original logs always accessible
- **Type Safety**: Leverage Rust's type system

## Development

```bash
cargo test -p unified-agent-sdk
cargo clippy -p unified-agent-sdk --all-targets --all-features -- -D warnings
```

## Contributing

Pull requests are welcome. Before submitting, run:

```bash
cargo fmt
cargo clippy -p unified-agent-sdk --all-targets --all-features -- -D warnings
cargo test -p unified-agent-sdk
```

See [ROADMAP.md](./ROADMAP.md) for implementation plan.

## License

Licensed under the [Apache License, Version 2.0](../LICENSE).
