# slate-agent-kit — Project Guide

Meta-repo and single source of truth for the agent-kit family: shared rule
sources, skills and document templates, the shared MCP servers, and the
installer. The three harness kits (`kits/{claude,codex,kimi}-agent-kit`) are git
submodules whose payload is **rendered** from this repo. A kit's root
`AGENTS.md` describes maintaining that kit; the manual a kit installs is its
`dist/CLAUDE.md` or `dist/AGENTS.md`.

## The one hard rule

**Never hand-edit rendered outputs.** Each kit's `dist/`, `install.sh`,
`install.ps1`, `Makefile` and root `AGENTS.md` are generated. Edit the sources
(`shared/`, `adapters/<h>/`, `tooling/kit-scripts/`), then:

```sh
sh tooling/render-kit.sh claude   # and/or codex, kimi
sh tooling/validate.sh            # must print "validate: OK" before committing
```

## Rule text

When a tool or behavior is removed or changed, rewrite the rule text so the new behavior is stated (what the agent does instead), not renamed mechanically. Guidance lives in one rule file; tool descriptions, `get_info` and READMEs lose dead references and gain no duplicate guidance. Reviewer suggestions outside the requested scope are left out unless the user asks.

## Topology

- `shared/rules/core/` — kernel (the articles, § 1 to § 21, in six parts) + the execution, delegation and git rule files, which hold only what the articles do not imply; `shared/rules/mcp/` — consultation (aside) and dispatch.
- `shared/workflows/palette/` — palette rule, skills, and `templates/` (also the palette server's schema); `shared/workflows/memory/` — the memory-triage skill; `shared/prefs/` — prefs templates.
- `shared/mcp-servers/{aside,dispatch,harness-log,palette}` and `shared/setup` (the `slate-setup` installer) — the Rust workspace (repo-root `Cargo.toml`). The kits ship no binaries of their own.
- `adapters/<harness>/` — `tokens.sed` (render-time `{{TOKEN}}` values, including `KIT_VERSION`), `inserts/*.md` (per-marker fragments); codex and kimi also have `surface.md` (harness surface rules).
- `tooling/` — `render-kit.sh`, `validate.sh`, `install-mcp.sh` (build the servers and register them through `slate-setup`), `slate-version` (the slate release a kit's binaries come from), `kit-scripts/` (entry-point and maintainer templates).
- `docs/` — this repo's own records and maintained documents (RST): `rfc/`, `adr/`, `changeset/`, `staging/`, `design/`, `spec/`, `principles.rst`, `glossary.rst`. `_palette/` holds the internal work documents (git-ignored). Indexes and `staging/` are generated: after changing `docs/` outside the palette server's tools, run `palette generate .` and commit the result; CI only checks them.

## Render mechanics

- `{{TOKEN}}` — substituted from `adapters/<h>/tokens.sed`. Kit version bumps happen there (`KIT_VERSION`); the slate release number lives in `tooling/slate-version`.
- `{{@INSERT name}}` — replaced with `adapters/<h>/inserts/<name>.md`. Every harness must have the file for every marker: empty file = no contribution, missing file = hard render error. Insert content passes through `tokens.sed` afterwards.
- Entry points and the maintainer `AGENTS.md` take per-kit tokens (`{{KIT_NAME}}`, `{{KIT_REPO}}`, `{{SLATE_VERSION}}`, `{{HARNESS}}`) from `render-kit.sh` itself.
- Articles: each `§ N` is defined exactly once, in the kernel, as a bold `**§ N Title.**` anchor with numbered clauses and a `Test:` line; everything else cites the number. Numbers never move: a new article takes the next free number or a letter suffix. `validate.sh` enforces one definition per cited article, treats `INV-`/`GATE-` as retired terms, harness-leak greps (no `advisor()`/`ultracode`/`ScheduleWakeup` in codex/kimi renders; no `TodoList`/`AgentSwarm`/`apply_patch` in claude renders; no `workslate` in any render), retired terms and trigger phrases, hard byte budgets on each rendered corpus, and `palette check` on this repo's own documents when a palette binary is built.

## Rust workspace

- `cargo build/test/clippy --workspace` at repo root covers aside, dispatch, harness-log, palette and slate-setup.
- CI runs ubuntu/macos/**windows** with `clippy -D warnings` on the **latest stable** — run `rustup update stable` locally before trusting a local clippy pass; an older local toolchain misses new lints.
- Code and tests must hold on Windows: slugs flatten `\` and `:` alongside `/`; tests assert path components, never separator-dependent rendered strings.
- Release builds Linux targets with cargo-zigbuild: pure-Rust dependencies only.
- Test fixtures are synthetic only — never commit real session data or a user's configuration.

## CI / Release

- `ci.yml` (push to main/next, PR to main): `validate.sh` always (checkout needs `submodules: recursive`); the 3-OS build+test+clippy+fmt only when Rust sources, `Cargo.*` or the palette templates changed (`dorny/paths-filter`); shellcheck (advisory) only when `tooling/` changed.
- `release.yml` (tag `v*`): 8-platform aside/dispatch/palette/slate-setup artifacts (cargo-zigbuild for Linux targets) and `checksums.txt` → GitHub Release. A kit's entry point downloads `slate-setup` from the release named in `tooling/slate-version` (the latest release when that one does not exist yet), and the clone fallback tracks slate **main** — keep main green and consumable.
- Pushing a tag in the same push that first adds a workflow file does not trigger it; push the tag separately.
- A script step added to a workflow is checked by running the extracted snippet locally as written, not by running an equivalent check in another form (a PowerShell quoting error once failed kit CI that way).

## Release train (order matters)

1. Develop a breaking release on a `next` branch in slate and in each kit; ordinary changes go to main directly. A change that touches only an MCP server binary still goes through every step below: the kits ship the servers, and a kit release is how users receive them.
2. Change `shared/` (+ `adapters/`, `tooling/`); bump `KIT_VERSION` in each affected kit's `tokens.sed` and, for a slate release, `tooling/slate-version`.
3. Verify locally: render, `validate.sh`, `cargo test`/`clippy` on the latest stable, and installs into a scratch **`HOME`** (a scratch harness home alone is not enough: harness CLIs edit the real user config under `$HOME`). Review the diff (advisor, and aside at its level) **before** any tag.
4. Re-render the kits, update each kit's `CHANGELOG.md`, commit, and push each kit's main without a tag.
5. Push slate's source changes and the submodule pin bumps in **one** push, so that main's `validate.sh` never runs a new check against old renders; wait for slate and kit CI to be green.
6. Tag each kit, then tag slate `vX.Y.Z` (this publishes the binaries).
7. For rules-affecting releases, refresh the live homes: `make install` in each kit (installs into `~/.claude`, `~/.codex`, `~/.kimi-code`; new rules apply from each harness's next session).
8. A regression found after a tag is reported and waits for the user; another release is the user's decision.

## Secrets

Never print, copy, or commit `~/.kimi-code/config.toml` (contains a live API
key) or `~/.codex/{config.toml,auth.json}`. Backups and verification steps
exclude them.

## Live homes

`~/.claude`, `~/.codex`, `~/.kimi-code` are production installs of the rendered
kits. `slate-setup` replaces or removes only kit-signed files; user-owned files
(`-custom:` signatures, e.g. prefs and custom rules) are preserved. Replacing
MCP binaries uses atomic rename + macOS ad-hoc codesign, so live sessions keep
running and pick the new binary up on restart.
