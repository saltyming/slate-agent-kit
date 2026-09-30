# Repository Guidelines

## Project Structure & Module Organization

`slate-agent-kit` is the meta repository for the agent-kit family. Shared rule
sources live in `shared/rules/core/` and `shared/rules/mcp/`; the palette rule,
skills and document templates live in `shared/workflows/palette/`, and the
memory-triage skill in `shared/workflows/memory/`. The Rust crates are the MCP
servers in `shared/mcp-servers/{aside,dispatch,harness-log,palette}` and the
installer in `shared/setup`. Harness render mappings are under
`adapters/{claude,codex,kimi}/`, and `kits/*-agent-kit/` are git submodules
whose `dist/`, entry points (`install.sh`, `install.ps1`, `Makefile`) and root
`AGENTS.md` are rendered. Do not hand-edit rendered files in `kits/`; edit
shared sources, adapters and `tooling/kit-scripts/`, then render.

This repository documents itself in `docs/` (RST: `rfc/`, `adr/`,
`changeset/`, `staging/`, `design/`, `spec/`, `principles.rst`,
`glossary.rst`); `_palette/` holds internal, git-ignored work documents.

## Build, Test, and Development Commands

- `cargo build --release --workspace` builds every Rust crate.
- `cargo test --release --workspace` runs the workspace test suite.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  matches CI lint strictness.
- `cargo fmt --all -- --check` verifies Rust formatting.
- `sh tooling/render-kit.sh codex` renders one kit; replace `codex` with
  `claude` or `kimi` as needed.
- `sh tooling/validate.sh` checks required paths, rendered outputs, inserts,
  invariant IDs, harness leaks, retired terms, size budgets, entry points and
  descriptors, and runs `palette check` on this repository when a palette
  binary is built.

## Coding Style & Naming Conventions

Rust crates use edition 2024 and standard `rustfmt` formatting. Use `snake_case`
for functions and modules, `PascalCase` for types, and avoid `unwrap()` in
production paths. Shell tooling is POSIX `sh` with `set -eu`. For rule text,
define each article `§ N` exactly once in the kernel and cite the number elsewhere,
and use the terms in `docs/glossary.rst`. Documents in `docs/` and `_palette/`
follow the RST house style in
`shared/workflows/palette/templates/house-style.rst`.

## Testing Guidelines

Rust unit tests live beside implementation modules with `#[test]`; integration
tests live in each crate's `tests/`. Add focused tests for parsers, transcript
handling, rendering edge cases, configuration edits and cross-platform path
behavior. Use synthetic fixtures only; never commit real session data or a
user's configuration. Run `sh tooling/validate.sh` whenever `shared/`,
`adapters/`, `tooling/`, workflows, or rendered kit files may be affected.

## Commit & Pull Request Guidelines

`docs/contributing.rst` is the contract for branches, commit messages, pull
request bodies and the verification a change runs; it is the palette
`contributing` family, so the palette lint checks it and the kit's git rule
follows it.

## Security & Configuration Tips

Do not print, copy, or commit live harness credentials such as
`~/.codex/config.toml`, `~/.codex/auth.json`, or `~/.kimi-code/config.toml`.
The installer preserves user-owned files (`-custom:` signatures); test install
flows in a scratch `HOME`.
