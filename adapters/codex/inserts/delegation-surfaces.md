---

## Codex delegation surfaces

- Codex's native subagents (`spawn_agent` and the tools that steer a spawned agent) are write-capable: a child has the parent's tools, whatever its `agent_type`. They run on the default the prefs set (`[agents] default_subagent_model`), else on the session's model. Codex has no read-only subagent; a read-only opinion goes through aside.
- A child can spawn subagents of its own. Say in its prompt whether it may; nested spawns stay within the count and files you stated.
- Do not simulate delegation with background shells or nested `codex exec` calls.
