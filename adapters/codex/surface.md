<!-- codex-agent-kit -->
# Codex Surface Rules

This file is the Codex-specific overlay. The shared Slate rules define the
behavioral policy (invariants and gates in `{{PRIMARY_MANUAL_FILE}}`); this file
binds that policy to Codex's actual tools, configuration paths, and loading
semantics.

## Loading Model

- User-scope instructions live at `$CODEX_HOME/AGENTS.md`; when `CODEX_HOME` is
  unset, use `$HOME/.codex/AGENTS.md`.
- The installer concatenates `AGENTS.md` and every `codex-rules/*.md` file into
  that user-scope `AGENTS.md`. Do not rely on `$CODEX_HOME/rules/*.md` being
  auto-loaded by Codex; the rules directory is installed as reference/source
  material, while the concatenated `AGENTS.md` is the loaded instruction surface.
- Preference files (`{{ASIDE_PREFS_FILE}}`, `{{DISPATCH_PREFS_FILE}}`) live in
  `$CODEX_HOME/rules/` and are **read on demand** before aside/dispatch use —
  they are deliberately not part of the concat so the user can edit them without
  reinstalling.
- Skills live under `$CODEX_HOME/skills`. Read a selected skill's `SKILL.md`
  completely before acting on it.

## Planning And Progress

- Use `update_plan` as the tactical tracker (`{{TASK_TRACKER}}` in the shared
  rules) for multi-step work, 2+ files, or 2+ deliverables. At most one item is
  `in_progress` at a time. Update statuses as work actually moves, not only at
  the end.
- Use `get_goal` / `create_goal` / `update_goal` only for explicit user- or
  system-requested goals. Do not create a goal merely because a task is large.
- Mark a goal `complete` only when the objective is actually achieved and no
  required work remains (INV-VERIFY-2). Mark `blocked` only under Codex's
  blocked-threshold rule, not because the work is large, slow, or would benefit
  from clarification.

## Editing

- Use `apply_patch` for manual file edits. Do not create or edit files with
  shell heredocs, `cat > file`, Python write scripts, or ad-hoc redirection when
  a direct patch is sufficient.
- A small patch is a *diff-size* discipline, not a *design-horizon* one
  (INV-QUALITY-1): the minimal diff that fixes the cause across the declared
  operating envelope is right; the even smaller diff that silences today's
  symptom on today's machine is not. Codex sessions habitually optimize for the
  latter — check the envelope before calling a patch done.
- Formatting commands, package-manager lockfile generation, and other mechanical
  tool outputs may write files when that is the tool's normal purpose.
- Preserve user-owned changes (INV-STATE-3): check `git status` / relevant diffs
  before editing, and do not revert or overwrite changes you did not make.
- `workslate` is Claude-only. In Codex, do not mention or require its tools; use `update_plan`, the goal surface when explicitly
  requested, and the shared MCP `dispatch` policy where installed.

## Shell And Tool Discovery

- Prefer `rg` / `rg --files` for search. Use `find` / `grep` only when `rg` is
  unavailable or the query needs their exact behavior.
- Use `multi_tool_use.parallel` only for independent developer-tool calls that
  can safely run at the same time, especially read-only commands such as `rg`,
  `sed`, `ls`, `git status`, and `git show`.
- Use `tool_search` for deferred MCP/app/tool discovery. Do not inspect MCP app
  resources directly when the active Codex instructions say to discover them via
  `tool_search`.
- `request_user_input` is Plan-mode-only. In Default mode, make reasonable
  assumptions and execute; ask a concise plain-text question only when the
  answer cannot be discovered locally and a wrong assumption would be costly
  (the WHAT/HOW clarification heuristic in `{{PRIMARY_MANUAL_FILE}}`).

## Slate MCP In Codex

- `aside` and `dispatch` are registered as normal Codex MCP servers in
  `$CODEX_HOME/config.toml` via `codex mcp add` (slate's
  `tooling/install-mcp.sh --configure-codex`); expected tool names are
  `mcp__aside__aside_list` / `aside_codex` / `aside_copilot` / `aside_claude` and
  `mcp__dispatch__dispatch_submit` / `dispatch_status` / `dispatch_wait` /
  `dispatch_logs` / `dispatch_steer` / `dispatch_cancel` / `dispatch_backends`.
- The installer sets `ASIDE_HARNESS=codex`: aside reads the **Codex session
  rollout natively** for transcript forwarding (interactive sessions only —
  headless `codex exec` children are excluded), with the shared redaction
  contract.
- dispatch containment: if Codex spawned the MCP server outside your project,
  `dispatch_submit` returns `no_project_root` — re-run the installer with
  `--roots <workspace-root>` (sets `DISPATCH_EXTRA_ROOTS`).
  `dispatch_backends` reports the live `project_root` / `extra_roots`.
- If the MCP servers are not installed, follow the policy documents as desired
  behavior and report that the tool surface is missing; do not pretend the call
  was made.
