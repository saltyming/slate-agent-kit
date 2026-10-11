<!-- slate-agent-kit:common -->
# Dispatch

The `dispatch` MCP server hands an execution step to a coding-agent backend that runs headless, write-capable and asynchronously; how to operate it is in its own instructions.

- For execution, not judgment: isolated mechanical edits, long verify-and-fix loops, well-scoped sweeps, independent steps with clear target files and acceptance criteria. Not for an open product question, edits overlapping active local or user changes, work needing interactive judgment, or anything that is not one self-contained spec. Level: `{{HARNESS_RULES_DIR}}/{{DISPATCH_PREFS_FILE}}` ({{PREFS_LOADING}}); at `suggest`, propose the step with its `working_dir`, scope and backend in one line and wait; at `auto`, submit and say the same (§ 8). Backend, model, effort and fallback come from the prefs unless the user names others; a fallback retry or automatic restart runs the same approved step.
- One self-contained step per dispatch; a dependent step waits for `succeeded`. Put the operating envelope into `constraints` and write `acceptance` against the contract (§ 9), from a palette deliverable's `Done when` where one exists; prefer objectives that converge when re-run. The step inherits every article (§ 15).
- Done only when `dispatch_status` shows `succeeded` and the captured result is reviewed (§ 16); after a cancel, confirm `cancelled`. No notification arrives and `dispatch_status` does not block: check later in the turn, and when ending the turn with a run open, arm a follow-up where the harness offers one, else tell the user to ask you to check back.

{{@INSERT dispatch-notify}}
