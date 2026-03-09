# Unified Agent SDK Roadmap

## Overview

Unified SDK for multiple AI coding agents (Claude Code, Codex, etc.) providing a common abstraction layer.

## Core Abstractions

### 1. Executor Abstraction (`executor.rs`)
- [x] `AgentExecutor` trait - unified interface for all agents
- [x] `AgentCapabilities` - capability declaration
- [x] `AvailabilityStatus` - runtime availability check
- [x] `SpawnConfig` - spawn configuration

**Status**: Implemented and used by Codex/Claude Code adapters

### 2. Profile System (`profile.rs`)
- [x] `ProfileId` - executor + variant identifier
- [x] `ExecutorConfig` - configuration with overrides
- [x] `ProfileManager` - configuration management with caching
- [x] `ResolvedConfig` - final resolved configuration

**Status**: Implemented (file loading, cache reload, runtime discovery, config resolve)

### 3. Log System (`log.rs`)
- [x] `NormalizedLog` - unified log format
- [x] `ActionType` - tool action classification
- [x] `LogNormalizer` trait - log normalization

**Status**: Implemented with Codex and Claude Code normalizers

### 4. Session Management (`session.rs`)
- [x] `AgentSession` - active session handle
- [x] `SessionMetadata` - persistence metadata
- [x] `SessionResume` - resume information

**Status**: Implemented (metadata + event pipeline + lifecycle controls `wait`/`cancel`)

### 5. Event System (`event.rs`)
- [x] `AgentEvent` - unified event types
- [x] `EventType` - event type enum
- [x] `EventStream` - event stream abstraction
- [x] `HookManager` - hook registration and triggering

**Status**: Implemented (normalized log -> unified events pipeline available)

## Implementation Phases

### Phase 1: Core Implementation
- [x] Implement `ProfileManager` with file-based storage
- [x] Implement basic `LogNormalizer` for Codex
- [x] Implement basic `LogNormalizer` for Claude Code
- [x] Add process management to `AgentSession` (`wait`/`cancel`)

### Phase 2: Adapters
- [x] Create `CodexAdapter` implementing `AgentExecutor`
- [x] Create `ClaudeCodeAdapter` implementing `AgentExecutor`
- [x] Implement log-to-event conversion
- [x] Add integration tests

### Phase 3: Advanced Features
- [x] Add profile discovery mechanism
- [x] Add capability detection

### Phase 4: Polish & Documentation
- [x] Complete API documentation
- [x] Add usage examples
- [x] Release v0.1.0

## Architecture

```
Application Layer
       ↓
Unified Agent SDK (this crate)
  - AgentExecutor trait
  - ProfileManager
  - LogNormalizer
  - EventStream + HookManager
       ↓
Adapters
  - CodexAdapter (implemented)
  - ClaudeCodeAdapter (implemented)
       ↓
Base SDKs
  - codex-sdk
  - claude-code-sdk
```

## Design Principles

1. **Minimal Abstraction**: Only abstract what's necessary
2. **Pluggable**: Storage, normalization, and hooks are pluggable
3. **No Lifecycle Management**: SDK doesn't manage workspace or persistence
4. **Preserve Raw Data**: Always keep original logs accessible
5. **Type Safety**: Leverage Rust's type system for correctness

## Non-Goals

- Workspace management (Git worktree, etc.)
- Task orchestration
- Session persistence (only provide metadata)
- UI components
- Log storage (application layer responsibility)

## Dependencies

- `async-trait` - async trait support
- `tokio` - async runtime
- `futures` - stream utilities
- `serde` / `serde_json` - serialization
- `thiserror` - error handling
- `chrono` - datetime handling

## Current Gap Assessment (2026-03-08)

- `AgentSession::wait` and `AgentSession::cancel` now use per-session lifecycle controllers (instance-owned; no global registry).
- Integration tests now exist at crate level (`unified-agent-sdk/tests`) and cover end-to-end event flow for Codex/Claude normalizers.
- API docs are now complete for the unified public surface (public rustdoc + doctests validated).

