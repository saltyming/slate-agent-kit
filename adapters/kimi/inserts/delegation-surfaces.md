---

## Kimi delegation surfaces

Two native surfaces, plus a small helper:

- **`Agent`** — single delegate calls. `subagent_type="explore"` / `"plan"` are read-only (free, proactive); `subagent_type="coder"` is write-capable (GATE-DELEGATE). Each agent runs in its own context window and returns a summary.
- **`AgentSwarm`** — fan one delegate out to many inputs in a single call; each item launches one agent, results aggregate. Prefer one `AgentSwarm` over hand-rolling parallel `Agent` calls for a same-prompt sweep; tighten the prompt template first.
- **`Skill`** — the canonical way to invoke a project-scoped helper (e.g. `palette-init`); not a parallel surface.

Kimi does not ship Agent-Teams-style persistent shared task lists or mid-turn inter-agent steering. External execution delegation is available through the shared `dispatch` MCP plugin when installed (`kimi-agent-kit--dispatch.md`); if a delegation need outgrows `Agent` / `AgentSwarm` / `dispatch`, surface the gap to the user rather than improvising.

**Kimi-specific anti-patterns:** spawning an `explore`/`plan` subagent for implementation work (cannot edit files — silent failure); `AgentSwarm` over a too-coarse prompt (N low-signal summaries); simulating a persistent team with background shells.
