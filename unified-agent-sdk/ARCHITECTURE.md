# Architecture

## Provider-Based Structure

Each SDK adapter is now organized as an independent provider module under `src/providers/`.

### Provider Structure

```
src/providers/
├── claude_code/
│   ├── mod.rs          # Provider module exports
│   ├── executor.rs     # AgentExecutor implementation
│   └── normalizer.rs   # LogNormalizer implementation
├── codex/
│   ├── mod.rs
│   ├── executor.rs
│   └── normalizer.rs
└── mod.rs              # All providers export
```

### Benefits

- **Isolation**: Each provider encapsulates all SDK-specific logic
- **Maintainability**: Changes to one provider don't affect others
- **Scalability**: New providers can be added without cross-file conflicts
- **Clarity**: All related code (executor, normalizer, events, profiles) lives together

### Adding a New Provider

1. Create `src/providers/<name>/` directory
2. Implement `executor.rs` with `AgentExecutor` trait
3. Implement `normalizer.rs` with `LogNormalizer` trait
4. Export in `src/providers/<name>/mod.rs`
5. Re-export in `src/providers/mod.rs`
6. Update `src/lib.rs` public exports
