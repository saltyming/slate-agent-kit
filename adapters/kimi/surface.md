<!-- kimi-agent-kit -->
# Kimi Surface Rules

This file is the Kimi-specific overlay. The shared Slate rules define the
behavioral policy (invariants and gates in `{{PRIMARY_MANUAL_FILE}}`); this file
binds that policy to Kimi Code CLI's actual tools, configuration paths, and
loading semantics.

## Loading Model

- User-scope instructions live at `$KIMI_CODE_HOME/AGENTS.md`; when
  `KIMI_CODE_HOME` is unset, use `$HOME/.kimi-code/AGENTS.md`. Kimi loads that
  single file once and it applies to every project.
- The installer concatenates `AGENTS.md` and every `kimi-rules/*.md` file into
  that user-scope `AGENTS.md`. Do not rely on `$KIMI_CODE_HOME/rules/*.md`
  being auto-loaded; the rules directory is installed as reference/source
  material, while the concatenated `AGENTS.md` is the loaded instruction
  surface.
- Preference files (`{{ASIDE_PREFS_FILE}}`, `{{DISPATCH_PREFS_FILE}}`) live in
  `$KIMI_CODE_HOME/rules/` and are **read on demand** before aside/dispatch
  use — deliberately not part of the concat so the user can edit them without
  reinstalling.
- Skills live under `$KIMI_CODE_HOME/skills` and are scanned natively. Read a
  selected skill's `SKILL.md` completely before acting on it.

## Planning And Progress

- Use `TodoList` as the tactical tracker (`{{TASK_TRACKER}}` in the shared
  rules) for multi-step work, 2+ files, or 2+ deliverables. Populate it
  *before* writing code; update statuses as work actually moves.
- `workslate` is Claude-only. In Kimi, do not mention or require its tools; `TodoList` is the system of record for tactical progress.

## Delegation Binding

- `Agent(subagent_type="explore" | "plan", …)` — read-only delegates: free and
  proactive (INV-GATE-1).
- `Agent(subagent_type="coder", …)` — write-capable: GATE-DELEGATE applies.
- `AgentSwarm` — the fan-out helper for same-prompt-many-inputs sweeps.
- `Skill` — the canonical entry for project-scoped helpers (e.g.
  `palette-init`).
- Details and Kimi-specific anti-patterns live in the delegation rule's
  **Kimi delegation surfaces** section (`{{DELEGATION_RULE_FILE}}`).

## Slate MCP In Kimi

- `aside` and `dispatch` are exposed through the local plugin
  `slate-agent-kit-mcp` (`$KIMI_CODE_HOME/plugins/managed/slate-agent-kit-mcp`,
  registered in `plugins/installed.json` by slate's
  `tooling/install-mcp.sh --configure-kimi`). Tool names are
  **plugin-prefixed**, e.g.
  `mcp__plugin-slate-agent-kit-mcp_aside__aside_list` and
  `mcp__plugin-slate-agent-kit-mcp_dispatch__dispatch_submit` — they differ
  from the plain `mcp__aside__*` names other harnesses use.
- The installer sets `ASIDE_HARNESS=kimi`: aside reads Kimi's own session logs
  natively (`session_index.jsonl` → `agents/main/wire.jsonl`) for transcript
  forwarding, with the shared redaction contract.
- dispatch containment: the Kimi plugin runtime spawns MCP servers in the
  plugin directory, not your project, so dispatch has **no project root** in
  Kimi — the installer must be run with `--roots <workspace-root>` (sets
  `DISPATCH_EXTRA_ROOTS`) or every `dispatch_submit` returns
  `no_project_root`. `dispatch_backends` reports the live containment
  configuration.
- If the plugin is not installed, follow the policy documents as desired
  behavior and report that the tool surface is missing; do not pretend the
  call was made.
