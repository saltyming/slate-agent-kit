---

## Kimi delegation surfaces

- `Agent` with `subagent_type="explore"` or `"plan"` is read-only; `"coder"` is write-capable, and so is an `AgentSwarm` of coders. Subagents run on the `[secondary_model]` default the prefs set, else on the session's model.
- For one prompt over many inputs, prefer a single `AgentSwarm` to hand-rolled parallel `Agent` calls, with a tight prompt template.
- Do not simulate a persistent team with background shells.
