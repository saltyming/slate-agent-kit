<!-- slate-agent-kit:common -->
# Git Workflow

Signing, model attribution, commit message format, PR body format and branch naming are the user's preferences in `{{HARNESS_RULES_DIR}}/{{GIT_PREFS_FILE}}` ({{PREFS_LOADING}}); the kit sets no default.

- A value still `unset` when needed is asked for and written into the file; only the values needed now. A missing file: ask for this action and say the file is not installed.
- When the repository's convention (a palette project's contributing document, `CONTRIBUTING`, a PR template, recent `git log`) differs from a prefs value, ask which to follow here and record it under "Repository overrides". The current-turn instruction outranks the file.
- Check `git branch -vv` for the base before opening a PR. Destructive git follows § 13 and never undoes session edits (§ 11).

{{@INSERT git-overrides}}
