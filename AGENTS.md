# agent-client-sdk-package

In this repository (Rust workspace for agent SDKs), follow these rules.

## Repository scope

- Workspace members:
  - `crates/codex` -> package `codex-client-sdk`, lib `codex`
  - `crates/claude-code` -> package `claude-code-client-sdk`, lib `claude_code`
  - `unified-agent-sdk` -> package `unified-agent-sdk`
- Primary goal: maintain parity with official upstream SDK behavior.

## Development rules

- Use the Rust edition declared by each crate's own `Cargo.toml`.
- Keep public API changes minimal and intentional; avoid accidental breaking changes.
- Prefer explicit typed errors (`thiserror`) over ad-hoc string errors.
- Keep event/message schema compatibility when touching CLI transport/parsing code.
- Keep changes scoped; do not refactor unrelated modules in the same PR.
- Do not edit `target/` artifacts.

## Test and validation policy

Run formatting and tests for the crate you changed.

1. Always run formatting after Rust code changes:
   - `cargo fmt --all`
2. Run package-level checks/tests for touched crates:
   - `cargo check -p [package name]`
   - `cargo test -p [package name]`
   - package names: see `Repository scope` -> workspace members
3. For broad or cross-crate changes, also run:
   - `cargo check --workspace`
   - `cargo test --workspace`
4. Run clippy when behavior/API changes are non-trivial:
   - `cargo clippy --workspace --all-targets --all-features -- -D warnings`

## Testing notes by area

- CLI command construction / env forwarding changes:
  - add or update command/transport tests in the affected crate.
- Stream/event parsing changes:
  - cover both success and failure paths.
- Prefer fixture-based subprocess tests over live-network tests.
- Mock CLIs used in tests:
  - `crates/codex/tests/fixtures/mock_codex_cli.py`
  - `crates/claude-code/tests/fixtures/mock_claude_cli.py`

## Version alignment rules

When version bump/alignment is requested, update all related files in one change:

- crate `Cargo.toml` version
- `Cargo.lock` local package entries
- crate `README.md` and `README_zh.md` (or other languages) version/status text

## Documentation rules

- If API or behavior changes, update README examples and related docs in the same PR.
- Keep all language variants of README/docs/spec notes consistent (English, Chinese, and any future languages).
- Do not claim parity/features in docs unless tests or implementation actually support them.

## PR rules

- Use scoped conventional titles, for example:
  - `feat(codex): ...`
  - `fix(claude-code): ...`
  - `chore(workspace): ...`
- PR description should include:
  - what changed
  - why it changed
  - important implementation details
  - validation commands executed

## Security and secrets

- Never commit credentials or tokens.
- Do not hardcode secrets in code, tests, docs, or fixtures.
- Use environment variables (for example `CODEX_API_KEY`, `ANTHROPIC_API_KEY`).
