---

## Claude delegation surfaces

Three mechanisms parallelize work in Claude Code — two via the `Agent` tool, one via the separate **`Workflow`** tool. The fourth surface, **`dispatch`**, is external execution delegation (shared taxonomy above; `claude-agent-kit--dispatch.md`).

- **Subagents** — `Agent` fire-and-forget: runs once, returns a result. Read-only types (`Explore`, `Plan`) are free and proactive; `general-purpose` is the write-capable type (GATE-DELEGATE applies) for genuinely **independent** subtasks.
- **Teammates** — `Agent` with `name` + `run_in_background: true`: a persistent, steerable peer in the single implicit team (no `TeamCreate`; `team_name` is ignored). For **dependent, coordinated** work — 3+ parallel streams that must talk or be steered mid-task. A foreground or unnamed `Agent` call is a subagent, not a teammate.
- **`Workflow`** — a separate tool running a deterministic script over many subagents (fan-out/pipeline), current-turn opt-in only (see *Workflow* below). `Agent` exposes `model` but no effort knob; Workflow's `agent()` exposes both — set them explicitly, they don't inherit.

### Teams — cost and composition

Each teammate is a full Claude Code instance (loads CLAUDE.md, MCP, skills; does NOT inherit your conversation); a 5-teammate team costs roughly 3–5× solo. Match the mechanism to the work: <5 files → single session, even if asked (explain why); 5–10 files → single session, or 1–2 subagents for isolated edits; 10+ files with clean non-overlapping scopes → a team is justified; overlapping scopes / shared types dominate → still single session. Scale informs what you *propose*, never whether to spawn unprompted (GATE-DELEGATE). Cap teams at 3–5 teammates; default them to `model="sonnet"` (escalate one to Opus only for demonstrated cross-module reasoning load); the leader stays on the session model and owns integration. Implementation teammates need a write-capable `subagent_type` — never `Explore`/`Plan`. Creation prompts carry **role + file scope only, never tasks** (a teammate with tasks in its prompt starts before the task graph exists).

### Coordination — built-in task list + workslate messaging

**Tasks:** the built-in task list (`TaskCreate` / `TaskUpdate` / `TaskList`) is the team's coordination system — the leader designs the task graph, reserves shared types / integration / cross-scope work for itself, and teammates self-claim eligible tasks (unblocked, unassigned, inside their file scope, not touching shared contracts). One writer per file (INV-GATE-2); teammate scopes must not overlap.

**Messaging:** native `SendMessage` is the channel — but it delivers only at the recipient's turn boundary. The workslate bridge closes the gap: a PostToolUse hook mirrors every native send into a SQLite inbox, and the recipient's PreToolUse doorbell announces unread messages **mid-turn**. Wiring is near-zero-touch: teammates are auto-registered under their agent name by the SubagentStart hook (the hint says so; if it instructs manual registration, call `workslate_register` with the hint's BOTH ids); the **leader registers once** — `workslate_register(role="team-lead", session_id=<SessionStart hint value>, agent_id="")` — so teammate→main traffic reaches its doorbell too. When the doorbell reports unread, drain with `workslate_inbox_read(role=<yours>, session_id=<hint value>)`; a bridged message read early will also arrive natively at the next turn boundary — a duplicate is expected, not a re-send. Call `workslate_msg_send` directly only for `urgent=true` steering that must interrupt. Do not use messages to coordinate task dependencies — that is the task list's job.

### Leader workflow

1. Register as team-lead (above) and drain your inbox once.
2. Spawn teammates (`Agent`, named, backgrounded, sonnet, role+scope prompt) — they explore their scope while waiting.
3. Design the task graph with `TaskCreate`; reserve shared contracts for yourself.
4. Monitor; intervene only on: inconsistent assumptions across reports, a silent stall on a claimed task, downstream failure after an upstream "completion", scope drift, duplicated work. **A teammate that stopped without your shutdown, normal completion, or an error report = assume the user interrupted it directly — hold its work, surface "waiting for user direction", do not re-assign or replace.**
5. Build & verify after completions; fix integration (imports, visibility, wiring) yourself.
6. Shut down every teammate (`shutdown_request`) when done.

### Teammate behavior

On creation: explore your scope only — do not implement; wait for tasks. Claim → work within scope → report → claim next. Report blockers to the leader immediately; never run build/test yourself (ask the leader); never shrink a task's scope (INV-GATE-3); message dependents directly when your output (types, APIs, formats) feeds their tasks. Before idling, drain your inbox (the doorbell only fires while you run tools).

**Completion report (HARD RULE)** — plain text to the leader, under ~500 tokens: `TASK: <id> — DONE` / `CHANGED:` file:line-range + 1-line each / `VERIFICATION:` concrete evidence ("grep 'fn old_name' → 0 matches"), not assertions / optional `DEFERRED:` + `GOTCHA:` (one-line trap worth propagating) / `NEXT:` ready-for-X | blocked-on-Y. No narration, no pasted code.

### Anti-patterns

6+ teammates (overhead dominates — cap 5); leader hand-dispatching every task (self-claim exists); leader skipping the build (integration issues found late); task instructions in creation prompts; `Explore`/`Plan` teammates for implementation (cannot edit — silent failure); messaging for dependency coordination; a Workflow `agent()` left on default model/effort. No session resume for in-process teammates (`/resume` won't restore them); task status can lag — check and update manually when stuck.

### Workflow (the third delegation surface)

`Workflow` runs a deterministic JS script orchestrating many subagents — for large, breadth-first, mechanical work (codebase-wide sweeps, N-file migrations, reviewer panels). **Real capability, never the default**: treat like any write-capable delegation — surface/propose → run on agreement. Valid opt-ins: `ultracode` confirmed for the current turn by a system-reminder; the user asking for a workflow in their own words; a user-invoked skill whose instructions call it; or the user agreeing to one you proposed. Never fire off stale or inferred opt-in. `ultracode` raises thoroughness (author workflows for substantive tasks, multi-phase with you in the loop) — it does NOT collapse the approval gate, license scope reduction, or replace your own verification of the synthesized result; budget exhaustion is not completion (stop, report remaining scope, ask).

Quality flow: pipeline by default, barrier only when a stage genuinely needs all prior results; diversify verifier lenses; one writer per target file (parallel writers → `isolation: 'worktree'`, you own the merge); one well-scoped fan-out per workflow — read each result and decide the next phase yourself; guard loops on `budget.total` and `log()` any silent cap; self-contained agent prompts; no nested orchestration; set `opts.model`/`opts.effort` explicitly on every non-trivial `agent()`. **Workflow subagents never call aside or `advisor()`** — cost, coherence, and the stdio-concurrency hazard; you own those surfaces, strictly serialized.
