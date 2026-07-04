# slate-agent-kit

`slate-agent-kit` is the meta-repository and single source of truth for the
agent-kit family — one shared operating manual + tooling stack rendered into
three coding-agent harnesses:

- **[claude-agent-kit](https://github.com/saltyming/claude-agent-kit)** — Claude Code
- **[codex-agent-kit](https://github.com/saltyming/codex-agent-kit)** — OpenAI Codex CLI
- **[kimi-agent-kit](https://github.com/saltyming/kimi-agent-kit)** — Kimi Code CLI

Each kit is a git submodule under `kits/`. Their rule files are **rendered
outputs** of the shared source here — edit the source, never the kits.

## Install a kit

Most users install one harness kit directly (not this repo). Each installer
lays down that harness's operating manual, rules, and palette skills, then
registers the shared `aside` / `dispatch` MCP servers and generates the
preference files.

**Claude Code** → `~/.claude`

```sh
curl -fsSL https://raw.githubusercontent.com/saltyming/claude-agent-kit/main/install.sh | sh
```
```powershell
irm https://raw.githubusercontent.com/saltyming/claude-agent-kit/main/install.ps1 | iex
```

**Codex CLI** → `~/.codex`

```sh
curl -fsSL https://raw.githubusercontent.com/saltyming/codex-agent-kit/main/install.sh | sh
```
```powershell
irm https://raw.githubusercontent.com/saltyming/codex-agent-kit/main/install.ps1 | iex
```

**Kimi Code** → `~/.kimi-code`

```sh
curl -fsSL https://raw.githubusercontent.com/saltyming/kimi-agent-kit/main/install.sh | sh
```
```powershell
irm https://raw.githubusercontent.com/saltyming/kimi-agent-kit/main/install.ps1 | iex
```

Common options (see each kit's README for the full list):

- Prerequisites: the harness CLI itself; plus `git` (curl path fetches the
  shared MCP source) **or** a Rust toolchain to build `aside`/`dispatch`. When
  cargo is absent the installers download prebuilt release binaries instead.
- `--uninstall` / `-Uninstall` — remove kit-signed files; your own `-custom:`
  prefs are kept.
- `--skip-mcp` (or `SKIP_MCP=1`) — install rules/skills only.
- Kimi dispatch needs a workspace root: `DISPATCH_ROOTS=/abs/workspace` (the
  plugin runtime spawns MCP servers outside any project).

## What lives here

- `shared/rules/core` — the invariant kernel + execution/delegation loops.
- `shared/rules/mcp` — `aside` (read-only consultation) and `dispatch`
  (write-capable execution) policy.
- `shared/workflows/palette` — the palette product-intent loop + skills.
- `shared/prefs` — aside/dispatch preference templates.
- `shared/mcp-servers/{aside,dispatch,harness-log}` — the portable Rust MCP
  workspace shared by every harness.
- `adapters/{claude,codex,kimi}` — per-harness render mappings (tokens, insert
  fragments, surface rules).
- `tooling/` — `render-kit.sh`, `validate.sh`, `install-mcp.sh`, and the shared
  `kit-scripts/` (single prefs generator + the Kimi plugin writer).

Harness-specific machinery stays in the submodules — notably `workslate`
(Claude-only task tracking) is not shared.

## For maintainers

Never hand-edit a rendered kit. Edit the shared source, then render + validate:

```sh
sh tooling/render-kit.sh claude   # and/or codex, kimi
sh tooling/validate.sh            # must print "validate: OK" before committing
```

Common rules are not summaries — they preserve the operational detail of the
kit rules and remove only harness-specific surfaces (a harness tool becomes a
`{{TOKEN}}` or moves into a surface insert). All harnesses render the
formal-language rule: in Korean the default register is polite formal
(`합니다` / `습니다` / `드립니다`); casual banmal is used only when the user
explicitly asks.

## Releases

Tagging `v*` publishes prebuilt `aside` / `dispatch` binaries for 8 targets
(macOS, Linux-gnu, Linux-musl, Windows × aarch64/x86_64) plus a `checksums.txt`,
via GitHub Actions. On macOS/Linux, `tooling/install-mcp.sh --prebuilt`
auto-selects and SHA-256-verifies the matching binary (musl included); on
Windows the kits' `install.ps1` fetch the `.zip` builds. Pin a version with
`SLATE_RELEASE_TAG=vX.Y.Z`, or require integrity verification with
`SLATE_REQUIRE_CHECKSUM=1`. CI runs build/test/clippy/fmt on the Rust workspace
plus `tooling/validate.sh` over the rendered kits on every push/PR to `main`.

## License

[MIT](LICENSE.md) © 2026 Hamin Sung.
