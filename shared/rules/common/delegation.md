<!-- slate-agent-kit:common -->
# Delegation

Every harness exposes different delegation surfaces. This common rule defines
the safety and scope policy that all of them must preserve.

- **Read-only delegates** inspect, search, plan, review, or summarize and return
  results without mutating files or external state.
- **Write-capable delegates** can edit files, run write-capable tools, or mutate
  state.
- **Fan-out helpers** run the same role or prompt over multiple inputs and
  aggregate results.
- **Skills / commands / workflows** are helper surfaces. They do not bypass
  scope, approval, or verification rules.

If a harness lacks a mechanism needed for safe delegation, surface that gap to
the user rather than improvising an unsafe substitute.

## Read-only subagents are free — use them proactively

Read-only delegates read widely and return only a summary — they *reduce* the leader's context cost. Reach for these without asking; they are **not** gated.

For read-only lookups larger than ~3 Grep/Glob queries, prefer a subagent over inline search — the leader's context is the bottleneck, and a subagent returns the result, not the evidence.

## Write-capable subagents are gated — surface/propose → agree

Any write-capable delegate can edit files or mutate state. The delegated result is what you see, not the reasoning; a wrong choice becomes a committed mistake. The user owns that cost, so the agent owns the proposal:

**For every write-capable subagent, surface first, spawn on agreement:**

- The **mechanism** — for example, a harness worker subagent, a team lane, a fan-out job, or a dispatch execution step.
- The **rough cost / scale** — "single subagent, ~5 files touched".
- **The files it will write** — `src/auth/login.ts`, `src/auth/session.ts`, … .

Spawn only after explicit agreement. Generic "just go" against an earlier ambiguous phrasing does not count — re-propose.

**The gate is on capability, not on the prompt.** Even if you intend to send a read-only prompt to a write-capable delegate, the surface/propose gate still applies — because the delegate *can* edit, a prompt-level "just read" is not a barrier. Use a genuinely read-only delegate for read-only work; do not work around the gate by changing prompt wording.

## Selection by dependency structure

After the gate is agreed, pick the surface by what the work looks like, not by default:

- **Independent, non-overlapping subtasks, contract-stable** → parallel write-capable delegates may be appropriate after the gate.
- **Same prompt, many inputs** (e.g. review N files, classify N items) → a fan-out helper may be appropriate if the prompt template is tight.
- **Sequential or tightly-coupled work** → in-session. Don't fan out subtasks that have to happen in order.
- **A subtask needs a different role** (research, design) → use a role-specific read-only or advisory delegate for that one slot.

Before calling any split "independent", identify shared contracts first — public types, schemas, migration ordering, shared tests, invariants — and keep those leader-owned. Independent-looking files often share a contract.

## One writer per final target file

No two delegates edit the same file. If two subagents would touch the same file, assign it to exactly one (or one explicit owner the leader integrates after). Otherwise the merge is the bottleneck.

## Delegated children inherit the scope rules

A delegate that hits a forced spec/plan deviation stops and reports to the leader, who then asks the user — a child never shrinks scope, reinterprets a budget, or substitutes a cleaner design on its own (see `{{TASK_EXECUTION_RULE_FILE}}` > Forced Spec/Plan Deviation).

**palette note.** When delegating a palette story, hand the delegate the *approved, tracked scope* (as `{{TASK_TRACKER}}` entries mapped from the story's acceptance criteria), never the raw `story-*.rst`. A story's acceptance criteria pass the approval gate first, then become the delegate's brief — a child receives Tier-B scope, not a Tier-A artifact. See `{{PALETTE_RULE_FILE}}`.

## Prompt rules for delegates

- **Self-contained** — include all necessary context inline. Subagents do not inherit the parent's conversation history.
- **Specific paths and expected outputs** — name files, name what success looks like.
- **State what the subagent should NOT do** — explicit out-of-scope cuts down on re-litigating settled decisions.
- **Frame settled decisions as constrained choices, not open questions.** A subtask that already carries a decision you made should be passed in framed ("do A; use B only if X; don't introduce a third pattern"), not as an open prompt ("figure out how to handle Y"). Open prompts re-open design space you've already settled — burning tokens and risking divergence from what sibling lanes assume.

## When unsure, do the work in-session and propose

A slower in-session edit beats a fast-but-silently-wrong delegated one. If parallelism would genuinely help, surface it ("this splits into N independent edits — want me to fan out subagents, ~rough cost?") and proceed on agreement.

## Anti-patterns

- **Spawning a write-capable delegate silently** — bypasses the gate. The result is durable on disk; the user must have agreed.
- **Assigning implementation work to a read-only delegate** — they cannot edit files. Silent failure.
- **Two delegates editing the same file** — manual merge pain, lost work. One writer per file.
- **Fan-out over a too-coarse prompt** — a 12-item sweep with "general code reviewer" returns 12 low-signal summaries. Tighten the prompt template, or run as in-session review.
- **Reaching for subagents when in-session is faster** — leader context is the goal, but coordination overhead is a real cost for tasks smaller than a few files. The scale is the user's, not yours.
