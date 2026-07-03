---

## Codex delegation surfaces

Codex has **no native write-capable subagent surface** — do not simulate one
with background shells or nested `codex exec` calls. The delegation loop's
write-capable mechanism in Codex is the shared **`dispatch`** MCP server
(`codex-agent-kit--dispatch.md`), which delegates a self-contained execution
step to an external backend (codex, opencode, or claude) under its own policy
and guards. Read-only second opinions go through **`aside`**
(`codex-agent-kit--aside.md`).

For work that would need a coordinated team or a large parallel fan-out:
either decompose it into sequential in-session work plus dispatch-able
independent steps, or surface the gap to the user.
