# Agent Client SDK Package（Rust）

[English](README.md) | [中文](README_zh.md)

## 概述

该仓库是一个 Rust workspace，包含两个以官方 SDK 核心能力对齐为目标的客户端 SDK：

- `codex-client-sdk`（库名 `codex`）：面向 Codex CLI，对齐官方 TypeScript SDK 核心工作流。
- `claude-code-client-sdk`（库名 `claude_code`）：面向 Claude Code CLI，对齐官方 Python SDK 核心工作流。

| Crate | 库名 | 路径 | 对齐目标 |
| --- | --- | --- | --- |
| `codex-client-sdk` | `codex` | `crates/codex` | 官方 Codex TypeScript SDK |
| `claude-code-client-sdk` | `claude_code` | `crates/claude-code` | 官方 Claude Agent Python SDK |

## 仓库结构

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

## 许可证

采用 Apache-2.0 许可证，详见 `LICENSE`。
