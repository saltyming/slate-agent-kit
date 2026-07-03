<!-- slate-agent-kit:common -->
# The Delegation Loop

When work fans out to delegates: **gate → select mechanism → integrate →
verify**. This file owns GATE-DELEGATE and the procedures behind INV-GATE-1/2/3
(defined in `{{PRIMARY_MANUAL_FILE}}`).

## Delegate taxonomy — and where aside/dispatch sit

- **Read-only delegates** inspect, search, plan, review, or summarize and return
  results without mutating files or external state.
- **Write-capable delegates** can edit files, run write-capable tools, or mutate
  state.
- **Fan-out helpers** run the same role or prompt over multiple inputs and
  aggregate results.
- **Skills / commands / workflows** are helper surfaces. They do not bypass
  scope, approval, or verification rules.

Two MCP-backed surfaces sit alongside the harness-native ones, and this is the
one place their relationship is stated: **`aside`** (`{{ASIDE_RULE_FILE}}`) is
*horizontal* consultation — a read-only cross-family second opinion; you stay in
charge. **`dispatch`** (`{{DISPATCH_RULE_FILE}}`) is *hierarchical* delegation —
it hands a **write-capable** execution step to an external coding agent and
tracks it asynchronously. A read-only opinion is aside; entrusting a build/edit
step to an external agent is dispatch, which carries its own approval gate
(GATE-DISPATCH) and server-enforced guards.

If a harness lacks a mechanism needed for safe delegation, surface that gap to
the user rather than improvising an unsafe substitute.

## Read-only delegates are free — use them proactively

Read-only delegates read widely and return only a summary — they *reduce* the leader's context cost. Reach for these without asking; they are **not** gated (INV-GATE-1).

For read-only lookups larger than ~3 search queries, prefer a subagent over inline search — the leader's context is the bottleneck, and a subagent returns the result, not the evidence.

## GATE-DELEGATE: write-capable delegation (HARD RULE)

**GATE-DELEGATE — the procedure for INV-GATE-1.** Any write-capable delegate can edit files or mutate state. The delegated result is what you see, not the reasoning; a wrong choice becomes a committed mistake. The user owns that cost, so the agent owns the proposal:

**For every write-capable delegate, surface first, spawn on agreement:**

- The **mechanism** — for example, a harness worker subagent, a team lane, a fan-out job, or a dispatch execution step.
- The **rough cost / scale** — "single subagent, ~5 files touched".
- **The files it will write** — `src/auth/login.ts`, `src/auth/session.ts`, … .

Spawn only after explicit agreement. Generic "just go" against an earlier ambiguous phrasing does not count — re-propose.

**The gate is on capability, not on the prompt.** Even if you intend to send a read-only prompt to a write-capable delegate, the gate still applies — because the delegate *can* edit, a prompt-level "just read" is not a barrier. Use a genuinely read-only delegate for read-only work; do not work around the gate by changing prompt wording. Default for an unknown/ambiguous delegate type: treat as write-capable.

**Mechanism-specific carve-out:** a harness adapter may carry an explicit user-configured auto-approval policy for one mechanism (e.g. dispatch's `proactive` + `auto` execution policy) — that policy then governs that mechanism instead of this gate. Nothing else loosens it.

## Selection by dependency structure

After the gate is agreed, pick the surface by what the work looks like, not by default:

- **Independent, non-overlapping subtasks, contract-stable** → parallel write-capable delegates.
- **Same prompt, many inputs** (review N files, classify N items) → a fan-out helper, if the prompt template is tight.
- **Sequential or tightly-coupled work** → in-session. Don't fan out subtasks that have to happen in order.
- **A subtask needs a different role** (research, design) → a role-specific read-only or advisory delegate for that one slot.
- **An isolated, self-contained execution step** (mechanical edits, long verification loops, well-scoped sweeps) → `dispatch` to an external backend, under its own policy.

Before calling any split "independent", identify shared contracts first — public types, schemas, migration ordering, shared tests, invariants — and keep those leader-owned. Independent-looking files often share a contract.

## One writer per final target file (INV-GATE-2 procedure)

No two delegates edit the same file. If two subagents would touch the same file, assign it to exactly one (or one explicit owner the leader integrates after). Otherwise the merge is the bottleneck. Worktree-style isolation prevents disk clobber but not divergent edits — the rule holds regardless.

## Delegated children inherit the invariants (INV-GATE-3 procedure)

A delegate that hits a forced spec/plan deviation stops and reports to the leader, who then asks the user — a child never shrinks scope, reinterprets a budget, or substitutes a cleaner design on its own (GATE-DEVIATION in `{{TASK_EXECUTION_RULE_FILE}}`). When delegating a palette story, hand the delegate the *approved, tracked scope*, never the raw Tier-A artifact (`{{PALETTE_RULE_FILE}}` § Gate bindings).

## Prompt rules for delegates

- **Self-contained** — include all necessary context inline. Subagents do not inherit the parent's conversation history.
- **Specific paths and expected outputs** — name files, name what success looks like.
- **State what the delegate should NOT do** — explicit out-of-scope cuts down on re-litigating settled decisions.
- **Name the operating envelope.** A delegate sees only its prompt, so the durability bar (INV-QUALITY-1) must be stated, not assumed: name the platforms, harnesses, input classes, and callers the change must hold under, and require the cause fixed rather than the symptom patched. Delegates optimize for making the immediate step pass — the envelope in the brief is what stands between you and a works-on-my-case patch.
- **Frame settled decisions as constrained choices, not open questions.** A subtask that already carries a decision you made should be passed in framed ("do A; use B only if X; don't introduce a third pattern"), not as an open prompt ("figure out how to handle Y"). Open prompts re-open design space you've already settled — burning tokens and risking divergence from what sibling lanes assume.
- **A delegate can't watch the live output of a backgrounded, long-running, or streaming process.** Design such commands around one of three escapes, and name the chosen one in the prompt: (1) **write-then-read** — redirect to a log file and read it back; (2) **observability sink** — a structured results file / status endpoint the delegate queries; (3) **output-free success check** — an artifact or exit-code file that decides success without a stream.

## When unsure, do the work in-session and propose

A slower in-session edit beats a fast-but-silently-wrong delegated one. If parallelism would genuinely help, surface it ("this splits into N independent edits — want me to fan out subagents, ~rough cost?") and proceed on agreement.

## Anti-patterns

- **Spawning a write-capable delegate silently** — bypasses GATE-DELEGATE. The result is durable on disk; the user must have agreed.
- **Assigning implementation work to a read-only delegate** — it cannot edit files. Silent failure.
- **Two delegates editing the same file** — manual merge pain, lost work (INV-GATE-2).
- **Fan-out over a too-coarse prompt** — a 12-item sweep with "general code reviewer" returns 12 low-signal summaries. Tighten the prompt template, or run as in-session review.
- **Reaching for delegates when in-session is faster** — leader context is the goal, but coordination overhead is a real cost for tasks smaller than a few files. The scale is the user's, not yours.

{{@INSERT delegation-surfaces}}
