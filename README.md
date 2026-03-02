# Agent Client SDK Package (Rust)

[English](README.md) | [中文](README_zh.md)

## Overview

Rust workspace containing two parity-focused client SDK crates:

- `codex-client-sdk` (`codex`): Codex CLI SDK aligned with the official TypeScript SDK core workflow.
- `claude-code-client-sdk` (`claude_code`): Claude Code CLI SDK aligned with the official Python SDK core workflow.

| Crate | Library Name | Path | Upstream Alignment |
| --- | --- | --- | --- |
| `codex-client-sdk` | `codex` | `crates/codex` | Official Codex TypeScript SDK |
| `claude-code-client-sdk` | `claude_code` | `crates/claude-code` | Official Claude Agent Python SDK |

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
│   └── claude-code
│       ├── src/
│       ├── tests/
│       ├── README.md
│       └── README_zh.md
└── LICENSE
```

## License

Licensed under Apache-2.0. See `LICENSE`.
