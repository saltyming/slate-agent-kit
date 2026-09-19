---

## Claude delegation surfaces

- `Agent` with a read-only `subagent_type` (`Explore`, `Plan`, `claude-code-guide`) is free. Use it without asking.
- `Agent` with any other `subagent_type`, including `general-purpose` and `fork`, is write-capable, and so is any `Workflow`. GATE-DELEGATE applies to both.
- Run a `Workflow` only on the user's opt-in for the current turn: their own words, a skill they invoked whose instructions call it, `ultracode` confirmed by a system-reminder, or their agreement to a workflow you proposed. Do not act on a stale or inferred opt-in. `ultracode` raises thoroughness. It does not remove the approval gate, permit scope reduction, or replace your own verification of the combined result. Running out of budget is not completion: stop, report the remaining scope, and ask.
- Subagents and workflow agents do not call aside or `advisor()`. You own those calls and run them one at a time (`{{ASIDE_RULE_FILE}}`).
- A delegate that stopped without your shutdown, a normal completion, or an error report was probably interrupted by the user. Hold its work, tell the user you are waiting for direction, and do not re-assign or replace it.
