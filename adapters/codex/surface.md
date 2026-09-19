<!-- codex-agent-kit -->
# Codex Surface Rules

The Codex-specific overlay. The shared Slate rules define the behavior
(invariants and gates in `{{PRIMARY_MANUAL_FILE}}`); this file covers what
differs in Codex: how the rules load, goals, editing, and the Slate MCP servers.

## Loading Model

- User-scope instructions live at `$CODEX_HOME/AGENTS.md`; when `CODEX_HOME` is
  unset, `$HOME/.codex/AGENTS.md`.
- The installer concatenates `AGENTS.md` and every `codex-rules/*.md` file into
  that user-scope `AGENTS.md`, which is the loaded instruction surface. Codex
  does not auto-load `$CODEX_HOME/rules/*.md`; that directory is reference
  material.
- The preference files (`{{ASIDE_PREFS_FILE}}`, `{{DISPATCH_PREFS_FILE}}`,
  `{{GIT_PREFS_FILE}}`, `{{COMMENT_PREFS_FILE}}`) live in `$CODEX_HOME/rules/` and
  are read on demand: before an aside or dispatch call, before the first commit
  or PR of a session, and before the first file, comment, doc comment, or
  header you write in a session, including in a file you are only editing.
  They are not part of the concat, so the user can edit them without
  reinstalling.
- Skills live under `$CODEX_HOME/skills`. Read a selected skill's `SKILL.md`
  completely before acting on it.

## Goals

- Use the goal tools only for goals the user or system explicitly requested. Do
  not create a goal because a task is large.
- Mark a goal `complete` only when the objective is achieved and no required
  work remains (INV-VERIFY-2). Mark it `blocked` only under Codex's
  blocked-threshold rule, not because the work is large or slow, or because a
  clarification would help.

## Editing

- Use `apply_patch` for manual file edits. Do not create or edit files with
  shell heredocs, `cat > file`, Python write scripts, or ad-hoc redirection when
  a direct patch is enough. Formatters, lockfile generation, and other
  mechanical tools may write files when that is their normal purpose.
- A small patch is a discipline about diff size, not about design horizon
  (INV-QUALITY-1). The minimal diff that fixes the cause across the declared
  operating envelope is right. The smaller diff that hides today's symptom on
  today's machine is not. Codex sessions tend toward the latter, so check the
  envelope before calling a patch done.

## Clarification

`request_user_input` works in Plan mode only. In Default mode, make reasonable
assumptions and execute. Ask a short plain-text question only when the answer
cannot be found locally and a wrong assumption would be costly (the
clarification heuristic in `{{PRIMARY_MANUAL_FILE}}`).

## Slate MCP In Codex

- `aside` and `dispatch` are registered as Codex MCP servers in
  `$CODEX_HOME/config.toml` by slate's `tooling/install-mcp.sh --configure-codex`.
- If `dispatch_submit` returns `no_project_root`, Codex spawned the MCP server
  outside your project. Tell the user to re-run the installer with
  `--roots <workspace-root>`.
- If the MCP servers are not installed, follow the policy documents as the
  intended behavior and report that the tool surface is missing. Do not pretend
  a call was made.
