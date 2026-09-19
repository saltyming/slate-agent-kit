---

## Kimi delegation surfaces

- `Agent` with `subagent_type="explore"` or `"plan"` is read-only and free. Use it without asking.
- `Agent` with `subagent_type="coder"` is write-capable, and so is an `AgentSwarm` of coders. GATE-DELEGATE applies to both.
- For one prompt over many inputs, prefer a single `AgentSwarm` to hand-rolled parallel `Agent` calls, and tighten the prompt template first.
- External execution goes through the shared `dispatch` MCP plugin when it is installed (`kimi-agent-kit--dispatch.md`). If a delegation need outgrows `Agent`, `AgentSwarm`, and `dispatch`, tell the user. Do not simulate a persistent team with background shells.
