# Architecture

`slate-agent-kit` is a meta repository, not a replacement for the individual
harness repositories.

## Ownership

| Area | Owner |
|---|---|
| Invariant kernel + execution/delegation loops, git, conventions (INV-*/GATE-* IDs) | `slate-agent-kit/shared/rules/core` |
| Per-harness insert blocks (`{{@INSERT name}}`) and surface rules | `slate-agent-kit/adapters/<harness>/inserts`, `adapters/{codex,kimi}/surface.md` |
| Prefs templates + generic configure script | `slate-agent-kit/shared/prefs`, `tooling/kit-scripts/configure-prefs.sh` |
| Shared aside/dispatch policy rules | `slate-agent-kit/shared/rules/mcp` |
| Palette loop and palette skills | `slate-agent-kit/shared/workflows/palette` |
| Aside MCP server | `slate-agent-kit/shared/mcp-servers/aside` |
| Dispatch MCP server | `slate-agent-kit/shared/mcp-servers/dispatch` |
| Codex rollout discovery/schema (shared by aside+dispatch) | `slate-agent-kit/shared/mcp-servers/harness-log` |
| MCP build + per-harness registration (claude/codex/kimi) | `slate-agent-kit/tooling/install-mcp.sh` |
| 9.4.0 → redesigned-corpus fidelity record | `slate-agent-kit/docs/coverage-matrix.md` (+ `docs/legacy/`) |
| Adapter render mappings | `slate-agent-kit/adapters/<harness>` |
| Claude workslate (server + hooks), Claude installer; CLAUDE.md/claude-rules are RENDER OUTPUTS since v10.0.0 | `kits/claude-agent-kit` |
| Codex AGENTS/config/hooks/agents installer | `kits/codex-agent-kit` |
| Kimi AGENTS/skills installer | `kits/kimi-agent-kit` |

## Submodule Direction

The harness repos are submodules of this meta repository:

```text
slate-agent-kit/kits/<harness-agent-kit>
```

The harness repos do not vendor `slate-agent-kit`. Synchronization is driven
from the top-level meta repo.

## Common Rule Quality Bar

The common rules must remain detailed enough to prevent the known failure
modes:

- Silent or announced scope reduction.
- Completion claims without verification.
- User-owned local changes being overwritten.
- Agent-owned rollback.
- Destructive git without explicit command-level authorization.
- Write-capable delegation without a concrete user-owned gate.
- Overly terse design/debugging communication.
- Casual Korean register when formal language is required.

## Portable Capabilities

`aside` and `dispatch` are shared MCP servers for all three harnesses (since
claude-agent-kit v10.0.0 the claude kit consumes them too — its own copies are
removed). aside reads each harness's session log natively (`ASIDE_HARNESS`);
dispatch supports codex/opencode/claude backends and per-harness containment
(`DISPATCH_EXTRA_ROOTS` via `install-mcp.sh --roots`). The shared policy rules
live in `shared/rules/mcp` and are rendered into each harness repo under that
harness's filename conventions. Harness installers still differ: Claude Code
auto-loads `~/.claude/rules/*.md` as separate files, while Codex and Kimi load
only one user-scope `AGENTS.md` — their installers concatenate the manual +
rules into it.

`workslate` does not move into shared because it depends on Claude Code hook
payloads, Claude session identity, and Claude team mechanics.
