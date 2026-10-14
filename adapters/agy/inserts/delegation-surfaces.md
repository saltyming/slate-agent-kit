---

## Antigravity delegation surfaces

- `define_subagent` and `invoke_subagent` start a write-capable delegate (a tools allowlist narrows what it may call but is not a read-only kind); `manage_subagents` lists and stops them. A delegate runs on the model its definition names, else on the session's model. A read-only opinion goes through `dispatch_consult`.
- A delegate can start subagents of its own (§ 15).
- Do not simulate delegation with background `run_command` shells or nested `agy` calls.
