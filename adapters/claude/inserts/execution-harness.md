## Task Sessions (workslate)

**`workslate_task_init(name)` is mandatory before using any task tool.** Tasks are stored in SQLite (`workslate.db`) and shared across all agent instances in the same project.

**Namespaces:** Tasks use `ws:` (personal) or `team:` (team coordination) prefixes:
- `workslate_task_create("Fix auth", namespace="ws")` → creates `ws:1`
- `workslate_task_create("Port handlers", namespace="team", owner="backend-dev")` → creates `team:1`
- `workslate_task_done("team:1")` — ID format: `"3"` (defaults to ws), `"ws:3"`, or `"team:3"`

**Cross-namespace dependencies:** `depends_on: ["ws:1", "team:2"]` — a task can depend on tasks in either namespace.

**Footer** shows both namespaces: `── Tasks (session) ws:[3/5] team:[8/12] ──`

**Workflow:**
1. `workslate_task_init("auth-refactor")` — create or resume a named session
2. `workslate_task_create(name, namespace?, owner?, depends_on?)` — create tasks
3. `workslate_task_done("ws:1")` / `workslate_task_update("team:3", status="in_progress")` — update
4. `workslate_task_list(namespace?)` — list tasks, optional namespace filter
5. `workslate_task_sessions()` — list all sessions with per-namespace counters

**Rules:**
- `workslate_task_init` must be called before any task operation
- Only one session is active at a time per MCP server instance
- Switching sessions does NOT clear the previous session's tasks (SQLite persists)
- Restarting the MCP server clears the active session — call `workslate_task_init` again to resume
- Multiple agent instances can read/write the same session concurrently (SQLite WAL mode)

## Anti-self-grading Stop verify hook

`make install` registers (alongside the doorbell hooks) a `type:"agent"` Stop hook that spawns an independent verifier subagent before Claude is allowed to end a turn. The verifier reads the turn's transcript tail for completion claims (tasks finished, tests passing, builds green), spot-checks the load-bearing claims against **real repository state** (reads the files, runs the cited tests/commands), and returns `{"ok": true}` to allow the stop or `{"ok": false, "reason": ...}` to block it — the reason is fed back to Claude as its next instruction. A turn with no completion claims passes immediately, and `stop_hook_active` bounds the block to one retry per turn.

**It is deliberately standalone.** The verifier has no workslate dependency — it judges the conversation's claims against the repo directly. Task tracking (`workslate_task_*`) and stop-verification are orthogonal: the board records intent and progress; the hook audits completion claims (INV-VERIFY-1/2 enforced mechanically).

**Relationship to `/goal`.** Claude Code's native `/goal` command (v2.1.139+) is a *session-scoped, user-typed* Stop hook whose tool-less evaluator judges transcript text alone. This hook is complementary: installed once, it applies to every session, and its verifier has tool access rather than judging text alone. Both can be active and fire independently on Stop.

**Cost tradeoff, accepted by design.** Claude Code's hook system has no mechanism for one hook entry to gate whether a sibling entry fires, so this `type:"agent"` entry spawns a verifier subagent on *every* Stop event, in *every* session — a real cost/latency addition. Bundled into the default install anyway since this is a single-user personal toolchain. Uninstall with `workslate --uninstall-hooks` (removes all workslate-owned hooks, including this one) if the tradeoff stops being worth it.

## Team Messaging Tools (Agent Teams)

For multi-agent coordination, workslate exposes `workslate_register(role, session_id, agent_id)`, `workslate_msg_send(recipient, subject, body, urgent?, session_id?, agent_id?)`, and `workslate_inbox_read(role)`, enabling **mid-turn steering** of running teammates via per-tool-call doorbell hooks. The full mechanics — leader/`team-lead` identity, the `session_id`+`agent_id` requirement on every `msg_send`, startup sequence, role addressing — are canonical in `claude-agent-kit--parallel-work.md` → **Mid-Turn Steering & Team Messaging**; this is a pointer, not a restatement. Task status is surfaced by the doorbell hook on every tool call (installed by `make install`), not appended to workslate tool results.
