# slate-agent-kit — Project Guide

Meta-repo and single source of truth for the agent-kit family. Shared rule
sources and shared MCP servers live here; the three harness kits
(`kits/{claude,codex,kimi}-agent-kit`) are git submodules whose rule files are
**rendered outputs** of this repo. Do not confuse this file with
`kits/claude-agent-kit/CLAUDE.md` — that one is a rendered artifact; this one
is the guide for working on the repo itself.

## The one hard rule

**Never hand-edit rendered outputs.** The kits' `CLAUDE.md` / `AGENTS.md`,
`*-rules/*.md`, and prefs templates are generated. Edit the sources
(`shared/rules/`, `shared/prefs/`, `adapters/<h>/`), then:

```sh
sh tooling/render-kit.sh claude   # and/or codex, kimi
sh tooling/validate.sh            # must print "validate: OK" before committing
```

## Topology

- `shared/rules/core/` — kernel (invariant register) + loop files; `shared/rules/mcp/` — aside/dispatch policy; `shared/workflows/palette/` — palette outer loop; `shared/prefs/` — prefs templates.
- `shared/mcp-servers/{aside,dispatch,harness-log}` — the Rust workspace (repo-root `Cargo.toml`). `workslate` is Claude-only and lives in `kits/claude-agent-kit`, not here.
- `adapters/<harness>/` — `tokens.sed` (render-time `{{TOKEN}}` values, including `KIT_VERSION`), `inserts/*.md` (per-marker fragments), `surface.md` (harness surface rules).
- `tooling/` — `render-kit.sh`, `validate.sh`, `install-mcp.sh` (build/download + register MCP servers), `kit-scripts/configure-prefs.sh`.
- `docs/coverage-matrix.md` — claude 9.4.0 → redesigned-corpus mapping; update it when migrating or superseding rule text.

## Render mechanics

- `{{TOKEN}}` — substituted from `adapters/<h>/tokens.sed`. Kit version bumps happen here (`KIT_VERSION`), nowhere else.
- `{{@INSERT name}}` — replaced with `adapters/<h>/inserts/<name>.md`. Every harness must have the file for every marker: empty file = no contribution, missing file = hard render error. Insert content passes through `tokens.sed` afterwards.
- `@@NAME@@` — configure-time placeholders in prefs templates; render leaves them intact (`validate.sh`'s `{{` leak check stays strict because of this split).
- Invariant/gate IDs: each `INV-*` / `GATE-*` is defined exactly once, as a bold `**ID — Title.**` anchor; everything else references the ID. `validate.sh` enforces uniqueness and cross-file reference integrity, plus harness-leak greps (no `workslate`/`advisor()`/`Workflow` in codex/kimi renders; no `TodoList`/`AgentSwarm`/`apply_patch` in claude renders) and a concat size guard.

## Rust workspace

- `cargo build/test/clippy --workspace` at repo root covers aside, dispatch, harness-log.
- CI runs ubuntu/macos/**windows** with `clippy -D warnings` on the **latest stable** — run `rustup update stable` locally before trusting a local clippy pass; an older local toolchain misses new lints.
- Code and tests must hold on Windows (INV-QUALITY-1's motivating case came from this repo): slugs flatten `\` and `:` alongside `/`; tests assert path components, never separator-dependent rendered strings.
- Test fixtures are synthetic JSONL only — never commit real session data.

## CI / Release

- `ci.yml` (push/PR to main): 3-OS build+test+clippy+fmt, `validate.sh` (checkout needs `submodules: recursive`), shellcheck (advisory).
- `release.yml` (tag `v*`): 8-platform aside/dispatch artifacts (cargo-zigbuild for Linux targets) → GitHub Release. `tooling/install-mcp.sh --prebuilt` consumes the latest release; the kits' `install.sh` clone-fallback tracks slate **main** — keep main green and consumable.
- Pushing a tag in the same push that first adds a workflow file does not trigger it; push the tag separately.

## Release train (order matters)

1. Change `shared/` (+ `adapters/`); bump `KIT_VERSION` in each affected kit's `tokens.sed`.
2. Commit + push slate main; wait for CI green.
3. Re-render kits, update each kit's `CHANGELOG.md`, then commit / tag / push each kit submodule.
4. **Last**: commit the submodule pin bumps in slate and tag slate `vX.Y.Z` (this publishes the binaries).
5. ff-sync the standalone checkouts (`~/Workspace/{claude,codex,kimi}-agent-kit`).
6. For rules-affecting releases, refresh the live homes: `make install SKIP_MCP=1` in each kit (installs to `~/.claude`, `~/.codex`, `~/.kimi-code`; new rules apply from each harness's next session).

## Secrets

Never print, copy, or commit `~/.kimi-code/config.toml` (contains a live API
key) or `~/.codex/{config.toml,auth.json}`. Backups and verification steps
exclude them.

## Live homes

`~/.claude`, `~/.codex`, `~/.kimi-code` are production installs of the rendered
kits. Installers are signature-guarded: only kit-signed files are replaced or
removed; user-owned files (`-custom:` signatures, e.g. prefs) are preserved.
Replacing MCP binaries uses atomic `mv` + macOS ad-hoc codesign, so live
sessions keep running and pick the new binary up on restart.
