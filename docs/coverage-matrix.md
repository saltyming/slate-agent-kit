# Coverage matrix — claude-agent-kit v9.4.0 → slate redesigned corpus

Maps every binding item in `docs/legacy/claude-9.4.0-inventory.txt` (frozen at
tag v9.4.0, commit 12421d8) to its location in the redesigned corpus. "New
location" names a shared source file § section, or an adapter insert file (which
lands in the claude render at the marked position). Statuses: **moved**
(text relocated, semantics identical), **merged-into-INV** (restated as a kernel
invariant + procedure pointer; full procedure preserved where noted),
**unchanged** (same file, same text), **superseded** (semantics consciously
changed — flagged for user review).

## [OVERRIDE] / HARD RULE / (HARD) markers

| 9.4.0 location | Item | New location | Status |
|---|---|---|---|
| CLAUDE.md:13 | System Prompt Notice ([OVERRIDE] convention) | adapters/claude/inserts/kernel-notice.md → CLAUDE.md § System Prompt Notice | moved |
| CLAUDE.md:36 | [OVERRIDE] "highly capable" investigation test | kernel.md § Humility First (test text) + inserts/kernel-overrides.md (quote) | moved |
| CLAUDE.md:45 | [OVERRIDE] verification beyond UI | kernel.md **INV-VERIFY-1** + inserts/kernel-overrides.md (quote) | merged-into-INV |
| CLAUDE.md:48 | [OVERRIDE] faithful reporting | kernel.md **INV-VERIFY-2** + inserts/kernel-overrides.md | merged-into-INV |
| CLAUDE.md:50 | [OVERRIDE] no context-usage bail + (a)(b)(c) scoping | kernel.md **INV-CTX-1** + inserts/kernel-overrides.md (scoping list) | merged-into-INV |
| CLAUDE.md:52 | [OVERRIDE] complete entire scope / no deferral / in-scope anchor | kernel.md **INV-SCOPE-1** (full text incl. announced-split ban, supporting-work anchor, raise-before-starting) | merged-into-INV |
| CLAUDE.md:54–60 | Scope judgment is user-owned (3 cases) | kernel.md **INV-SCOPE-2** / **INV-STATE-1/2** / **INV-SCOPE-3** + gate pointers | merged-into-INV |
| CLAUDE.md:78 | [OVERRIDE] observation vs action ("bug fix doesn't need cleanup") | kernel.md § Collaboration + inserts/kernel-overrides.md (quotes) | moved |
| CLAUDE.md:82 | [OVERRIDE] Agent proactive-use, delegation-tools-only scoping | inserts/collab-overrides.md + kernel.md **INV-GATE-1** | moved |
| aside.md:6 | [OVERRIDE] `# Advisor Tool` section | mcp/aside.md § Relation to native advisor surfaces + inserts/aside-native-advisor.md (advisor() naming, full-transcript contrast) | moved |
| aside.md:43 | HARD RULE — no aside/advisor concurrency | mcp/aside.md § Triggers (generic hazard) + inserts/aside-native-advisor.md (Claude stdio-interference specifics) | moved |
| aside.md:105 | HARD RULE — paths over excerpts | mcp/aside.md § HARD RULE: hand the backend file paths | unchanged |
| dispatch.md:20 | HARD RULE — ending a turn on a running task | mcp/dispatch.md § Ending a turn + inserts/dispatch-notify.md (ScheduleWakeup / Agent-Workflow contrast) | moved |
| dispatch.md:52 | HARD RULE — approval gate | mcp/dispatch.md **GATE-DISPATCH** | moved |
| dispatch.md:67 | [OVERRIDE] precedence over the gate | mcp/dispatch.md § Invariant precedence (reworded: OVERRIDE-quote framing → INV-SCOPE-1/INV-GATE-3 references; same semantics) | superseded |
| task-execution.md:45 | HARD RULE — Plan Integrity post-inspection | core/loop-execution.md **GATE-SCOPE-CONFIRM** | moved |
| task-execution.md:77 | shrink-vs-define complementarity clarification | core/loop-execution.md § GATE-SCOPE-CONFIRM clarifications | moved |
| task-execution.md:111 | [OVERRIDE] entire specified scope (design docs) | core/loop-execution.md § Execution Requirements (policy) + kernel.md INV-SCOPE-1; system-prompt quotes live in inserts/kernel-overrides.md | moved |
| task-execution.md:119 | [OVERRIDE] "prefer editing existing files" | core/loop-execution.md § Execution Requirements → "Create new files when the spec says so" (quote dropped, policy + failure-mode example intact) | moved |
| task-execution.md:141 | HARD RULE — Forced Spec/Plan Deviation | core/loop-execution.md **GATE-DEVIATION** (3 triggers + disambiguating tests + anti-loophole + 7-step action, verbatim) | moved |
| task-execution.md:157 | anything-outside-three → no-reduction | core/loop-execution.md § GATE-DEVIATION ("Anything outside these three follows INV-SCOPE-1") | moved |
| task-execution.md:172 | HARD RULE — Undo / Revert Handling (A/B) | core/loop-execution.md § Undo / Revert Handling A/B (INV-STATE-1/2 procedures, verbatim) | moved |
| task-execution.md:216 | HARD RULE — explicit git-command carve-out (C) | core/loop-execution.md **GATE-GIT** (verbatim, incl. project-mandatory flags) | moved |
| palette.md:8 | Engagement — folder-gated (HARD) | workflows/palette/rules.md § Engagement (unchanged) | unchanged |
| palette.md:17 | Auto-engagement ≠ auto-authority (HARD) | workflows/palette/rules.md (unchanged) | unchanged |
| palette.md:21 | Authority firewall Tier A/B (HARD) | workflows/palette/rules.md (procedure) + kernel.md **INV-AUTH-1** (definition) | merged-into-INV |
| palette.md:30 | Scope invariant (HARD) | workflows/palette/rules.md § Scope invariant (unchanged) | unchanged |
| git-workflow.md:6 | [OVERRIDE] --no-gpg-sign standing request | core/git-workflow.md § Commit Rules + inserts/git-overrides.md (quote) | moved |
| git-workflow.md:9 | [OVERRIDE] no Co-Authored-By | core/git-workflow.md (no-attribution rule) + inserts/git-overrides.md (quote) | moved |
| git-workflow.md:52 | [OVERRIDE] no 🤖 PR footer | inserts/git-overrides.md (merged with the commit-attribution override) | moved |
| parallel-work.md:257 | HARD RULE — completion report format | inserts/delegation-surfaces.md § Completion report format (verbatim) | moved |
| parallel-work.md:335 | HARD RULE — msg_send session_id+agent_id | inserts/delegation-surfaces.md § Mid-turn steering (all constraints preserved: reject-on-omit, register guard, task_init exemption, sender bypass, leader `agent_id=""`) | moved |

## Section headings

| 9.4.0 location | Section(s) | New location | Status |
|---|---|---|---|
| CLAUDE.md:11–98 | System Prompt Notice / Core Principles / Three-Phase / Humility / Quality / Communication / Collaboration / Quick Reference / Decision Tree | core/kernel.md (same sections; invariants extracted to § Invariants; Korean formal rule now also applies to the claude render — additive) | moved |
| aside.md:19–151 | Decision rules / Proactive policy / Triggers / model+effort / Backend capabilities / paths-over-excerpts / Transcript redaction / Cost / Reporting | mcp/aside.md (same sections; "aside ≠ advisor()" heading renamed "aside differs from native advisors"; redaction section documents native 3-harness readers) | moved |
| dispatch.md:6–100 | Async model / Watching+steering / Execution policy / Approval gate / Writing the task / Model fallback / Server guards / Cost | mcp/dispatch.md (same sections; + claude backend, no_project_root, GATE-DISPATCH anchor) | moved |
| git-workflow.md:4–62 | Commit Rules / Message Format / PR Rules | core/git-workflow.md (same; destructive-git restatement collapsed to a GATE-GIT pointer) | moved |
| framework-conventions.md:4–33 | React/Nextjs / Rust / Python | core/conventions.md | unchanged |
| palette.md:8–340 | all palette sections incl. rubrics + schemas | workflows/palette/rules.md (same; + § Gate bindings consolidating the scattered palette notes from CLAUDE.md, task-execution, parallel-work, dispatch, git-workflow) | moved |
| task-execution.md:4–115 | Before Starting / Investigation / Implementation / After Completion | core/loop-execution.md (same structure; restated invariants now ID references) | moved |
| task-execution.md:238–277 | Task Sessions / Stop verify hook / Team Messaging pointer | inserts/execution-harness.md (claude render only; verbatim) + inserts/execution-tracking.md (tracker table) | moved |
| parallel-work.md:12–39 | Delegation: when and how to engage it | core/loop-delegation.md (GATE-DELEGATE + taxonomy + selection; generic text now shared across harnesses) | moved |
| parallel-work.md:41–96 | Choosing / Spawn mechanism / Subagents | inserts/delegation-surfaces.md (same sections; generic prompt rules + live-output escapes now in core/loop-delegation.md § Prompt rules) | moved |
| parallel-work.md:100–475 | Agent Teams (all subsections) / Communication / Patterns / Known Limitations / Anti-Patterns | inserts/delegation-surfaces.md § Agent Teams (cost table, model choice, composition, leader workflow+checklist, intervention incl. user-interrupt rule, teammate behavior, claiming policy, mid-turn steering, communication table, patterns, limitations, anti-patterns — condensed prose, every binding constraint preserved) | moved |
| parallel-work.md:477–523 | Workflow (opt-in signals, ultracode, model+effort, quality flow, code staging, aside-in-workflow ban) | inserts/delegation-surfaces.md § Workflow (all four ultracode non-loosening rules, opt-in list, quality flow, nested-orchestration ban, aside ban preserved) | moved |

## Consciously superseded items (user review required)

1. **dispatch.md:67 "[OVERRIDE] precedence"** — reworded from quoting CLAUDE.md
   `[OVERRIDE]`s to referencing INV-SCOPE-1/INV-GATE-3. Same semantics
   (invariants outrank the approval gate); the OVERRIDE-quote framing was
   Claude-specific and now lives only in the claude kernel insert.
2. **kimi backup `parallel-work.md` line 10** ("This project does not ship …
   an external `dispatch`-style execution broker") — superseded: the shared
   `dispatch` MCP server is now installed on Kimi via the plugin;
   adapters/kimi/inserts/delegation-surfaces.md documents dispatch as
   available-when-installed instead.
3. **task-execution.md:111/119 system-prompt quotes** — the quoted system-prompt
   text is dropped from the harness-neutral loop-execution.md (policy fully
   preserved); the Claude-facing quotes are consolidated in
   inserts/kernel-overrides.md rather than repeated per-file.
4. **CLAUDE.md Communication** — gains the formal-Korean register rule
   (INV-COMM-1) that previously existed only in the Slate shared corpus; this
   is additive for the claude render, not a removal.
5. **CLAUDE.md:82 Agent-tool "When not to use" quotation** — the long verbatim
   quote is trimmed to a reference in inserts/collab-overrides.md; the scoping
   rule (delegation tools only; read-only free; write-capable gated;
   aside/advisor out of scope; dispatch policy carve-out) is fully preserved.

## v11.0.0 subtraction release — relocations

The v11 corpus subtraction moved operational mechanics out of the standing
rules into just-in-time surfaces. Semantics preserved unless marked; the
standing rules keep every INV-*/GATE-* definition, trigger list, and
decision-shaping boundary example. Enforced by validate.sh § 8b byte budgets.

| Was (standing rules) | Now | Status |
|---|---|---|
| aside backend-capability matrix, transcript-redaction table, path-vs-excerpt examples, model/effort mechanics | aside server instructions + per-tool descriptions | moved |
| dispatch tool-by-tool async model, spec field list, logs paging/steering mechanics, server-guard enumeration | dispatch server instructions + per-tool descriptions | moved |
| palette artifact schemas, RST house style, scoring-rubric prose | `_palette/templates/*` scaffolded by palette-init (SKILL.md carries the pack) | moved |
| palette rubric weighting/bands | `_palette/templates/rubrics.md` (condensed, formulas intact) | moved |
| workslate task-system rules (ws:/team: namespaces, task_init sessions, footer) | removed with the feature — tactical tracking is the harness-native task list (the TASK_TRACKER render token) | superseded |
| workslate msg_send startup sequence + HARD guards prose | delegation-surfaces insert (compressed); mechanics in workslate tool descriptions; SubagentStart auto-registration + SendMessage bridge hook remove most startup steps | superseded |
| kernel [OVERRIDE] quotes of harness system-prompt text | ghost quotes deleted; surviving interpretation rules restated quote-free in kernel-overrides insert (live-conflict check 2026-07-17: only commit/PR attribution remains a real conflict — kept in git-overrides) | superseded |
| GATE-SCOPE-CONFIRM / GATE-DEVIATION / GATE-GIT rationale paragraphs and long enumerations | condensed in loop-execution.md; every trigger, disambiguating test, and required sequence preserved | moved |
| Agent-Teams patterns/limitations tables, completion-report long form | delegation-surfaces insert, condensed | moved |
