# Unified Agent SDK Roadmap

## Overview

Unified SDK for multiple AI coding agents (Claude Code, Codex, etc.) providing a common abstraction layer.

## Core Abstractions

### 1. Executor Abstraction (`executor.rs`)
- [x] `AgentExecutor` trait - unified interface for all agents
- [x] `AgentCapabilities` - capability declaration
- [x] `AvailabilityStatus` - runtime availability check
- [x] `SpawnConfig` - spawn configuration

**Status**: Framework complete, implementation pending

### 2. Profile System (`profile.rs`)
- [x] `ProfileId` - executor + variant identifier
- [x] `ExecutorConfig` - configuration with overrides
- [x] `ProfileManager` - configuration management with caching
- [x] `ResolvedConfig` - final resolved configuration

**Status**: Framework complete, implementation pending

### 3. Log System (`log.rs`)
- [x] `NormalizedLog` - unified log format
- [x] `ActionType` - tool action classification
- [x] `LogStorage` trait - pluggable storage abstraction
- [x] `LogNormalizer` trait - log normalization

**Status**: Framework complete, implementation pending

### 4. Session Management (`session.rs`)
- [x] `AgentSession` - active session handle
- [x] `SessionMetadata` - persistence metadata
- [x] `SessionResume` - resume information

**Status**: Framework complete, implementation pending

### 5. Event System (`event.rs`)
- [x] `AgentEvent` - unified event types
- [x] `EventType` - event type enum
- [x] `EventStream` - event stream abstraction
- [x] `HookManager` - hook registration and triggering

**Status**: Framework complete, implementation pending

## Implementation Phases

### Phase 1: Core Implementation
- [ ] Implement `ProfileManager` with file-based storage
- [ ] Implement basic `LogNormalizer` for Codex
- [ ] Implement basic `LogNormalizer` for Claude Code
- [ ] Add process management to `AgentSession`

### Phase 2: Adapters
- [ ] Create `CodexAdapter` implementing `AgentExecutor`
- [ ] Create `ClaudeCodeAdapter` implementing `AgentExecutor`
- [ ] Implement log-to-event conversion
- [ ] Add integration tests

### Phase 3: Storage & Advanced Features
- [ ] Implement file-based `LogStorage`
- [ ] Implement memory-based `LogStorage`
- [ ] Add profile discovery mechanism
- [ ] Add capability detection

### Phase 4: Polish & Documentation
- [ ] Complete API documentation
- [ ] Add usage examples
- [ ] Performance optimization
- [ ] Release v0.1.0

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
- Database integration (provide trait only)

## Dependencies

- `async-trait` - async trait support
- `tokio` - async runtime
- `futures` - stream utilities
- `serde` / `serde_json` - serialization
- `thiserror` - error handling
- `chrono` - datetime handling

## Next Steps

1. Implement `ProfileManager` with JSON file loading
2. Create basic log normalizers for Codex and Claude Code
3. Implement process spawning in adapters
4. Add comprehensive tests
