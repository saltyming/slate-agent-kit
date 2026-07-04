# Repository Guidelines

## Project Structure & Module Organization

`slate-agent-kit` is the meta repository for the agent-kit family. Shared rule
sources live in `shared/rules/core/` and `shared/rules/mcp/`; palette workflow
source lives in `shared/workflows/palette/`. Portable Rust MCP servers are in
`shared/mcp-servers/{aside,dispatch,harness-log}`. Harness render mappings are
under `adapters/{claude,codex,kimi}/`, and `kits/*-agent-kit/` are git
submodules containing rendered kit outputs. Do not hand-edit rendered files in
`kits/`; edit shared sources and adapters, then render.

## Build, Test, and Development Commands

- `cargo build --release --workspace` builds all shared Rust MCP crates.
- `cargo test --release --workspace` runs the workspace test suite.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  matches CI lint strictness.
- `cargo fmt --all -- --check` verifies Rust formatting.
- `sh tooling/render-kit.sh codex` renders one kit; replace `codex` with
  `claude` or `kimi` as needed.
- `sh tooling/validate.sh` checks required paths, rendered outputs, inserts,
  invariant IDs, harness leaks, and size guards.

## Coding Style & Naming Conventions

Rust crates use edition 2024 and standard `rustfmt` formatting. Use `snake_case`
for functions and modules, `PascalCase` for types, and avoid `unwrap()` in
production paths. Shell tooling is POSIX `sh` with `set -eu`. For rule text,
define each `INV-*` or `GATE-*` ID exactly once and reference that ID elsewhere.

## Testing Guidelines

Rust unit tests live beside implementation modules with `#[test]`. Add focused
tests for parsers, transcript handling, rendering edge cases, and cross-platform
path behavior. Use synthetic JSONL fixtures only; never commit real session
data. Run `sh tooling/validate.sh` whenever `shared/`, `adapters/`, workflows,
or rendered kit files may be affected.

## Commit & Pull Request Guidelines

History follows Conventional Commits, for example `feat(rules): ...`,
`fix(tooling): ...`, and `chore(release): ...`. Keep commits scoped to the
source change plus any required render output. Use `--no-gpg-sign` for commits.
PRs should summarize source files changed, list verification commands, and note
whether kit submodule pins or release artifacts are affected.

## Security & Configuration Tips

Do not print, copy, or commit live harness credentials such as
`~/.codex/config.toml`, `~/.codex/auth.json`, or `~/.kimi-code/config.toml`.
Installer and preference files are signature-guarded; preserve user-owned custom
configuration when testing install flows.
