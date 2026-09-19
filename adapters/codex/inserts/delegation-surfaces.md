---

## Codex delegation surfaces

Codex has no native write-capable subagent surface. Do not simulate one with
background shells or nested `codex exec` calls. The write-capable mechanism in
Codex is the shared `dispatch` MCP server (`codex-agent-kit--dispatch.md`), and
read-only second opinions go through `aside` (`codex-agent-kit--aside.md`).

For work that would need a coordinated team or a large parallel fan-out, split
it into sequential in-session work plus independent steps that can be
dispatched, or tell the user about the gap.
