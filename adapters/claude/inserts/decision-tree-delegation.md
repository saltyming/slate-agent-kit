│     ├─ Independent, non-overlapping subtasks → Subagents: Agent(subagent_type="general-purpose", …)
│     │     fire-and-forget, self-contained prompts
│     ├─ Coordinated streams that must talk / be steered → Agent Team (single implicit team):
│     │     Agent(name=…, subagent_type=…, model="sonnet", run_in_background=true, prompt=<role-only>)
│     │     task graph via TaskCreate; mid-turn steering via the workslate doorbell; leader verifies
│     ├─ Large breadth-first mechanical sweep → Workflow (separate tool; current-turn opt-in only)
│     └─ External execution step (codex/opencode/claude backend) → dispatch_submit
│           → poll dispatch_status / dispatch_logs / dispatch_steer
│           (dispatch-prefs execution policy; proactive+auto → submit directly)
