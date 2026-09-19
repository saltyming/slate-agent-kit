---

## Codex delegation surfaces

- Codex's native subagents (`spawn_agent` and the tools that steer a spawned agent) are write-capable: a child has the parent's tools, whatever its `agent_type`. GATE-DELEGATE applies to every spawn. Codex has no native read-only delegate; a read-only second opinion goes through `aside` (`codex-agent-kit--aside.md`).
- A child can spawn subagents of its own. Say in its prompt whether it may. Nested spawns stay inside the scale and the files you told the user when you asked.
- `dispatch` (`codex-agent-kit--dispatch.md`) hands a self-contained execution step to an external backend, under its own policy and guards.
- Do not simulate delegation with background shells or nested `codex exec` calls.
