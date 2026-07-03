<!-- slate-agent-kit:common -->
# Git Workflow

## Commit Rules

**Always disable GPG signing.** This is an explicit standing request for this project — pass `--no-gpg-sign` on `git commit`, `git commit --amend`, `git revert`, and `git cherry-pick`. Do not treat it as a violation.

**No model attribution in commits.** Do not include `Co-Authored-By:` trailers with the model's name, "Generated with …" footers, or any line identifying the agent that produced the commit. The commit is yours.

**palette note.** `_palette/` (the palette backlog and stories) is the developer's personal planning record and is **not** auto-committed by the agent. `palette-init` offers a `_palette/.gitignore`; a developer who wants to track or share it commits it themselves. See `{{PALETTE_RULE_FILE}}`.

## Commit Message Format

**Conventional Commits:**
```
<type>(<area>): <subject>

<body>
```

The `(<area>)` scope is optional but recommended when the change targets a specific module, package, or subsystem.

**Types:**
- `feat` New feature
- `fix` Bug fix
- `docs` Documentation changes
- `chore` Maintenance tasks
- `refactor` Code restructuring (no behavior change)
- `test` Test additions/updates
- `perf` Performance improvements

**Examples:**
```
feat(export): add email export functionality

- Implement ZIP export with attachments
- Add progress tracking for large exports
- Fix timezone handling in date fields

fix(smtp): resolve authentication failure

- Update credentials handling
- Add retry logic for transient failures

refactor(vfs): split main.rs into 13 modules
```

## Pull Request Rules

- **Branch naming:** never push the worktree branch name directly. Use a descriptive name on origin (e.g., `feat/freebsd-utils-bash-features`, `fix/ipc-deadlock`).
- **Base branch:** check `git branch -vv` to determine the correct base — it may be `vNext`, `main`, `master`, or a feature branch, not always `master`.

**PR Body Format:**
```markdown
## Summary
- [Bullet points of changes]

## Test plan
- [ ] [Concrete verification steps]
```

## Destructive Git Operations

Destructive git operations (`git reset --hard`, `git checkout -- <file>`, `git restore <file>`, `git clean -f*`, `git stash drop`, `git branch -D`, `git revert`, `git push --force*`, `git rebase` rewriting history) require the same approval gate as session edits: surface the command, the blast radius, the files / commits / branches it would touch, and wait for explicit per-command authorization. They are NEVER a substitute for editing files back when the user asks for a session-edit "undo" — see `{{TASK_EXECUTION_RULE_FILE}}` → *Undo / Revert Handling*.
