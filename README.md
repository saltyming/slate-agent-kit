# slate-agent-kit

`slate-agent-kit` is the meta-repository and single source of truth for the
agent-kit family — one shared operating manual + tooling stack rendered into
three coding-agent harnesses:

- **[claude-agent-kit](https://github.com/saltyming/claude-agent-kit)** — Claude Code
- **[codex-agent-kit](https://github.com/saltyming/codex-agent-kit)** — OpenAI Codex CLI
- **[kimi-agent-kit](https://github.com/saltyming/kimi-agent-kit)** — Kimi Code CLI

Each kit is a git submodule under `kits/`. Its installable payload (`dist/`)
and entry points are **rendered outputs** of the shared source here — edit the
source, never the kits.

## Install a kit

Most users install one harness kit directly (not this repo). Every kit installs
through the same program, `slate-setup`: it asks its questions (with the current
value as the default), shows a summary of every change, and then lays down the
harness's operating manual, rules, palette and memory skills, registers the
shared `aside`, `dispatch` and `palette` MCP servers, writes the preference
files, and records everything so uninstall can reverse it.

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

Commands and options (see each kit's README for the full list):

- `install.sh [install|configure|uninstall]` — `configure` re-asks the
  preferences, custom rules and workspace roots; `uninstall` removes kit-signed
  files and restores the configuration values it changed, keeping your own
  `-custom:` files unless you choose otherwise. `--uninstall` still works.
- Prerequisites: the harness CLI itself. The installer and the servers are
  downloaded prebuilt for the platform; `--binaries build` builds them from a
  slate checkout with cargo instead, and `--binaries skip` (or `--skip-mcp`)
  installs the rules and skills only.
- `--roots <paths>` — workspace roots for dispatch and palette. Kimi needs one
  (its plugin runtime starts servers outside any project); the installer asks.
- `--yes` answers every question with its current or default value; `--dry-run`
  prints the summary and changes nothing.

## What lives here

- `shared/rules/core` — the kernel (direction, autonomy, prohibitions,
  reporting, memory) and the execution, delegation, git and convention rules.
- `shared/rules/mcp` — when to consult (`aside`) and when to dispatch.
- `shared/workflows/palette` — the palette document system: rule, skills and
  templates; `shared/workflows/memory` — the memory-triage skill.
- `shared/prefs` — aside, dispatch, subagent, git and comment preference
  templates.
- `shared/mcp-servers/{aside,dispatch,harness-log,palette}` and `shared/setup` —
  the Rust MCP servers and the `slate-setup` installer, shared by every harness.
- `adapters/{claude,codex,kimi}` — per-harness render mappings (tokens, insert
  fragments, surface rules).
- `tooling/` — `render-kit.sh`, `validate.sh`, `install-mcp.sh`,
  `slate-version`, and `kit-scripts/` (the entry-point and maintainer
  templates).
- `docs/` — this repository's own decision records and maintained documents.

The kits ship no binaries of their own; every binary is built and released from
this repo.

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

Tagging `v*` publishes prebuilt `aside`, `dispatch`, `palette` and `slate-setup`
binaries for 8 targets (macOS, Linux-gnu, Linux-musl, Windows × aarch64/x86_64)
plus a `checksums.txt`, via GitHub Actions. A kit's entry point downloads
`slate-setup` for its platform from the release named in `tooling/slate-version`
and verifies it against `checksums.txt`; `slate-setup` does the same for the
servers. CI runs build/test/clippy/fmt on the Rust workspace plus
`tooling/validate.sh` over the rendered kits on every push/PR to `main`.

## License

[MIT](LICENSE.md) © 2026 Hamin Sung.
