# Agent Client SDK Package (Rust)

[English](README.md) | [中文](README_zh.md)

## Overview

Rust workspace containing parity-focused client SDK crates and a unified abstraction crate:

- `codex-client-sdk` (`codex`): Codex CLI SDK aligned with the official TypeScript SDK core workflow.
- `claude-code-client-sdk` (`claude_code`): Claude Code CLI SDK aligned with the official Python SDK core workflow.
- `opencode-client-sdk` (`opencode`): OpenCode SDK aligned with the official JavaScript SDK core workflow.
- `unified-agent-sdk`: Unified executor/profile/event abstraction built on top of Codex and Claude Code SDKs.

| Crate | Library Name | Path | Upstream Alignment |
| --- | --- | --- | --- |
| `codex-client-sdk` | `codex` | `crates/codex` | Official Codex TypeScript SDK |
| `claude-code-client-sdk` | `claude_code` | `crates/claude-code` | Official Claude Agent Python SDK |
| `opencode-client-sdk` | `opencode` | `crates/opencode` | Official OpenCode JavaScript SDK |
| `unified-agent-sdk` | `unified_agent_sdk` | `unified-agent-sdk` | Unified abstraction over workspace SDKs |

## Repository Layout

```text
.
├── Cargo.toml
├── crates
│   ├── codex
│   │   ├── src/
│   │   ├── tests/
│   │   ├── README.md
│   │   └── README_zh.md
│   ├── claude-code
│   │   ├── src/
│   │   ├── tests/
│   │   ├── README.md
│   │   └── README_zh.md
│   └── opencode
│       ├── src/
│       ├── tests/
│       ├── README.md
│       └── README_zh.md
├── unified-agent-sdk
│   ├── src/
│   ├── README.md
│   └── ROADMAP.md
└── LICENSE
```

## License

Licensed under Apache-2.0. See `LICENSE`.
