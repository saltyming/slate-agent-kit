# Architecture

`slate-agent-kit` is a meta repository, not a replacement for the individual
harness repositories.

## Ownership

| Area | Owner |
|---|---|
| Shared scope, verification, undo/revert, communication, planning, git, and delegation rules | `slate-agent-kit/shared/rules/common` |
| Shared aside/dispatch policy rules | `slate-agent-kit/shared/rules/mcp` |
| Palette loop and palette skills | `slate-agent-kit/shared/workflows/palette` |
| Aside MCP server | `slate-agent-kit/shared/mcp-servers/aside` |
| Dispatch MCP server | `slate-agent-kit/shared/mcp-servers/dispatch` |
| Adapter render mappings | `slate-agent-kit/adapters/<harness>` |
| Claude workslate, Claude hooks, Claude installer | `kits/claude-agent-kit` |
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

`aside` and `dispatch` move into shared MCP because they are conceptually
portable. The shared policy rules live in `shared/rules/mcp` and are rendered
into each harness repo under that harness's filename conventions. Harness
installers may still differ because each harness discovers MCP servers and
loads rules differently.

`workslate` does not move into shared because it depends on Claude Code hook
payloads, Claude session identity, and Claude team mechanics.
