<!-- agy-agent-kit -->
# Antigravity Surface Rules

The Antigravity-specific overlay. The shared Slate rules define the behavior
(the articles in `{{PRIMARY_MANUAL_FILE}}`); this file covers what differs in
the Antigravity CLI (`agy`): how the rules load, the harness defaults the
user's preferences settle, and the Slate MCP servers.

## Loading Model

- User-scope instructions live at `$HOME/.gemini/config/AGENTS.md`, which
  Antigravity loads into every session. The rule files, including the
  preference files, live in `$HOME/.gemini/config/rules/`; each carries a
  frontmatter block with `trigger: always_on`, which is why Antigravity loads
  them with the manual. A file in that folder without the block is ignored, so
  the user edits a preference file's text, never its first four lines.
- `$HOME/.gemini/config` is shared with the Antigravity application, so the
  same rules apply there.
- Skills are installed under `$HOME/.gemini/config/skills`. Read a selected
  skill's `SKILL.md` completely before acting on it.

## Standing Instruction

This manual is the user's standing instruction. Where the harness's defaults leave a choice to the user, the user has made it here, and the choice applies; it changes nothing the harness reserves to itself: permissions, safety, how a tool is operated.

- **Memory.** § 21 applies to Antigravity's knowledge items and to any memory
  file you write.
- **Second opinions.** Antigravity has no native advisor; another model
  family's view comes through `dispatch_consult` on codex or claude. agy is not a
  consultation backend.
- **Subagents.** `define_subagent`, `invoke_subagent` and `manage_subagents`
  start write-capable delegates; the tools allowlist is not a read-only kind.
- **Shell.** `run_command` returns within 10 s and leaves a longer command
  running as a background task; the harness notifies you when it finishes.
  Do not poll `command_status` for a process you are waiting on.

## Slate MCP In Antigravity

- `dispatch` and `palette` are registered in `$HOME/.gemini/config/mcp_config.json`
  by the kit installer and called through `call_mcp_tool`. The installer
  pre-approves `dispatch_consult`, `dispatch_status`, `dispatch_list`,
  `dispatch_logs`, `dispatch_backends` and the palette read tools in
  `permissions.allow`; every other tool asks the user.
- If a server is not registered, follow the policy documents as the intended
  behavior and report that the tool surface is missing. Do not pretend a call
  was made.
