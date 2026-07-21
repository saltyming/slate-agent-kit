│     ├─ Independent, non-overlapping subtasks → Agent tool (write-capable subagent), one per lane
│     ├─ Same prompt over N items (breadth-first mechanical sweep) → AgentSwarm fan-out
│     └─ External execution step (codex/opencode/claude backend) → dispatch_submit when installed
│           → poll dispatch_status / dispatch_logs / dispatch_steer
│           (dispatch-prefs execution policy; proactive+auto → submit directly)
