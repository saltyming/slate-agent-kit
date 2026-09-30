<!-- kimi-agent-kit -->
# Kimi Surface Rules

The Kimi-specific overlay. The shared Slate rules define the behavior
(the articles in `{{PRIMARY_MANUAL_FILE}}`); this file covers what
differs in Kimi Code CLI: how the rules load and the Slate MCP plugin.

## Loading Model

- User-scope instructions live at `$KIMI_CODE_HOME/AGENTS.md`; when
  `KIMI_CODE_HOME` is unset, `$HOME/.kimi-code/AGENTS.md`. Kimi loads that single
  file once and it applies to every project.
- The installer writes that `AGENTS.md` as this manual followed by every kit
  rule file and the user's custom rules; it is the loaded instruction surface.
  Kimi does not auto-load `$KIMI_CODE_HOME/rules/*.md`; that directory is reference
  material.
- The preference files (`{{ASIDE_PREFS_FILE}}`, `{{DISPATCH_PREFS_FILE}}`,
  `{{SUBAGENT_PREFS_FILE}}`, `{{GIT_PREFS_FILE}}`, `{{COMMENT_PREFS_FILE}}`) live in
  `$KIMI_CODE_HOME/rules/` and are not part of the concatenated file, so the user can
  edit them without reinstalling. Read each one the first time a session needs
  it: before consulting aside, dispatching or starting a subagent, before the
  first commit or PR, and before the first file, comment, doc comment or header
  you write.
- Skills live under `$KIMI_CODE_HOME/skills` and are scanned natively. Read a
  selected skill's `SKILL.md` completely before acting on it.

## Slate MCP In Kimi

- `aside`, `dispatch` and `palette` come through the local plugin
  `slate-agent-kit-mcp`, registered by the kit installer. Tool names are
  plugin-prefixed, for example `mcp__plugin-slate-agent-kit-mcp_aside__aside_list`
  or `mcp__plugin-slate-agent-kit-mcp_palette__palette_status`, and differ from
  the plain names other harnesses use.
- The Kimi plugin runtime starts servers in the plugin directory, not in your
  project, so dispatch and palette need a workspace root. Without one they
  report `no_project_root`; tell the user to run the kit installer's configure
  step (`make configure` in the kit, or `install.sh configure`) and give the
  workspace root when it asks.
- If the plugin is not installed, follow the policy documents as the intended
  behavior and report that the tool surface is missing. Do not pretend a call
  was made.
