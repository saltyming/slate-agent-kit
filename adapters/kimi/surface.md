<!-- kimi-agent-kit -->
# Kimi Surface Rules

The Kimi-specific overlay. The shared Slate rules define the behavior
(invariants and gates in `{{PRIMARY_MANUAL_FILE}}`); this file covers what
differs in Kimi Code CLI: how the rules load and the Slate MCP plugin.

## Loading Model

- User-scope instructions live at `$KIMI_CODE_HOME/AGENTS.md`; when
  `KIMI_CODE_HOME` is unset, `$HOME/.kimi-code/AGENTS.md`. Kimi loads that single
  file once and it applies to every project.
- The installer concatenates `AGENTS.md` and every `kimi-rules/*.md` file into
  that user-scope `AGENTS.md`, which is the loaded instruction surface. Kimi
  does not auto-load `$KIMI_CODE_HOME/rules/*.md`; that directory is reference
  material.
- The preference files (`{{ASIDE_PREFS_FILE}}`, `{{DISPATCH_PREFS_FILE}}`,
  `{{GIT_PREFS_FILE}}`, `{{COMMENT_PREFS_FILE}}`) live in `$KIMI_CODE_HOME/rules/` and
  are read on demand: before an aside or dispatch call, before the first commit
  or PR of a session, and before the first file, comment, doc comment, or
  header you write in a session, including in a file you are only editing.
  They are not part of the concat, so the user can edit them without
  reinstalling.
- Skills live under `$KIMI_CODE_HOME/skills` and are scanned natively. Read a
  selected skill's `SKILL.md` completely before acting on it.

## Slate MCP In Kimi

- `aside` and `dispatch` come through the local plugin `slate-agent-kit-mcp`,
  registered by slate's `tooling/install-mcp.sh --configure-kimi`. Tool names
  are plugin-prefixed, for example
  `mcp__plugin-slate-agent-kit-mcp_aside__aside_list`, and differ from the plain
  `mcp__aside__*` names other harnesses use.
- The Kimi plugin runtime spawns MCP servers in the plugin directory, not in
  your project, so dispatch has no project root in Kimi. Unless the installer
  was run with `--roots <workspace-root>`, every `dispatch_submit` returns
  `no_project_root`; tell the user to re-run it with that flag.
- If the plugin is not installed, follow the policy documents as the intended
  behavior and report that the tool surface is missing. Do not pretend a call
  was made.
