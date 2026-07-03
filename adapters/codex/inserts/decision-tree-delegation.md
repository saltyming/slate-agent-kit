│     ├─ Independent mechanical step with clear acceptance → dispatch_submit (external backend)
│     │     → poll dispatch_status / dispatch_wait / dispatch_logs / dispatch_steer
│     │     (dispatch-prefs execution policy; proactive+auto → submit directly)
│     └─ Coordinated or judgment-heavy work → stay in-session; Codex has no native
│           write-capable subagent surface — do not simulate one with shell tricks
