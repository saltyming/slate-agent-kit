#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
fail=0

# ── 1. required source paths ──────────────────────────────

required="
.gitmodules
README.md
LICENSE.md
Cargo.toml
docs/architecture.md
docs/support-matrix.md
docs/coverage-matrix.md
docs/legacy/claude-9.4.0-inventory.txt
adapters/claude/tokens.sed
adapters/codex/tokens.sed
adapters/kimi/tokens.sed
adapters/codex/surface.md
adapters/kimi/surface.md
shared/rules/core/kernel.md
shared/rules/core/loop-execution.md
shared/rules/core/loop-delegation.md
shared/rules/core/git-workflow.md
shared/rules/core/conventions.md
shared/rules/mcp/aside.md
shared/rules/mcp/dispatch.md
shared/workflows/palette/rules.md
shared/workflows/palette/skills/palette-init/SKILL.md
shared/prefs/aside-prefs.md.tmpl
shared/prefs/dispatch-prefs.md.tmpl
shared/mcp-servers/aside/Cargo.toml
shared/mcp-servers/dispatch/Cargo.toml
shared/mcp-servers/harness-log/Cargo.toml
tooling/render-kit.sh
tooling/install-mcp.sh
tooling/kit-scripts/configure-prefs.sh
"

for path in $required; do
  if [ ! -e "$ROOT/$path" ]; then
    echo "missing: $path" >&2
    fail=1
  fi
done

for path in kits/claude-agent-kit kits/codex-agent-kit kits/kimi-agent-kit; do
  if [ ! -d "$ROOT/$path/.git" ] && [ ! -f "$ROOT/$path/.git" ]; then
    echo "missing submodule checkout: $path" >&2
    fail=1
  fi
done

# ── 2. insert integrity ───────────────────────────────────
# Every {{@INSERT name}} marker in shared sources / surfaces must have an
# insert file for ALL THREE adapters (empty file = no contribution; missing
# file = hard error at render time — assert it here too). No orphan insert
# files, no nested markers inside insert files.

markers=$(grep -rhoE '^\{\{@INSERT [a-z0-9-]+\}\}$' \
    "$ROOT/shared/rules" "$ROOT/shared/workflows" \
    "$ROOT/adapters/codex/surface.md" "$ROOT/adapters/kimi/surface.md" 2>/dev/null \
  | sed 's/{{@INSERT \([a-z0-9-]*\)}}/\1/' | sort -u)

for m in $markers; do
  for h in claude codex kimi; do
    if [ ! -f "$ROOT/adapters/$h/inserts/$m.md" ]; then
      echo "insert file missing for marker '$m': adapters/$h/inserts/$m.md" >&2
      fail=1
    fi
  done
done

for h in claude codex kimi; do
  for f in "$ROOT/adapters/$h/inserts/"*.md; do
    [ -e "$f" ] || continue
    name=$(basename "$f" .md)
    if ! echo "$markers" | grep -qx "$name"; then
      echo "orphan insert file (no marker uses it): adapters/$h/inserts/$name.md" >&2
      fail=1
    fi
    if grep -q '{{@INSERT' "$f"; then
      echo "nested insert marker inside insert file: adapters/$h/inserts/$name.md" >&2
      fail=1
    fi
  done
done

# ── 3. rendered output audit (all three kits) ─────────────

check_rendered_file() {
  f="$1"
  if [ ! -f "$f" ]; then
    echo "missing rendered file: $f" >&2
    fail=1
    return
  fi
  # A render leak is an unrendered TOKEN — `{{KIT_VERSION}}`, `{{@INSERT …}}`,
  # `{{ASIDE_RULE_FILE}}` — which always opens `{{` + a token char (upper-case
  # or `@`). Match that shape, not bare `{{`/`}}`: shell and PowerShell
  # legitimately contain `}}` (e.g. nested `${VAR:=${VAR2:-}}`), which is not a
  # leak, so a bare-brace check false-positives on copied scripts.
  if grep -En '\{\{[A-Z@]' "$f" >/dev/null; then
    echo "unrendered placeholder in $f" >&2
    fail=1
  fi
}

claude_files="CLAUDE.md
claude-rules/claude-agent-kit--task-execution.md
claude-rules/claude-agent-kit--parallel-work.md
claude-rules/claude-agent-kit--palette.md
claude-rules/claude-agent-kit--git-workflow.md
claude-rules/claude-agent-kit--framework-conventions.md
claude-rules/claude-agent-kit--aside.md
claude-rules/claude-agent-kit--dispatch.md"

codex_files="AGENTS.md
codex-rules/codex-agent-kit--codex-surface.md
codex-rules/codex-agent-kit--task-execution.md
codex-rules/codex-agent-kit--delegation.md
codex-rules/codex-agent-kit--palette.md
codex-rules/codex-agent-kit--git-workflow.md
codex-rules/codex-agent-kit--framework-conventions.md
codex-rules/codex-agent-kit--aside.md
codex-rules/codex-agent-kit--dispatch.md
scripts/codex-agent-kit--aside-prefs.md.tmpl
scripts/codex-agent-kit--dispatch-prefs.md.tmpl
scripts/configure-prefs.sh"

kimi_files="AGENTS.md
kimi-rules/kimi-agent-kit--kimi-surface.md
kimi-rules/kimi-agent-kit--task-execution.md
kimi-rules/kimi-agent-kit--delegation.md
kimi-rules/kimi-agent-kit--palette.md
kimi-rules/kimi-agent-kit--git-workflow.md
kimi-rules/kimi-agent-kit--framework-conventions.md
kimi-rules/kimi-agent-kit--aside.md
kimi-rules/kimi-agent-kit--dispatch.md
scripts/kimi-agent-kit--aside-prefs.md.tmpl
scripts/kimi-agent-kit--dispatch-prefs.md.tmpl
scripts/configure-prefs.sh"

for f in $claude_files; do check_rendered_file "$ROOT/kits/claude-agent-kit/$f"; done
for f in $codex_files; do check_rendered_file "$ROOT/kits/codex-agent-kit/$f"; done
for f in $kimi_files; do check_rendered_file "$ROOT/kits/kimi-agent-kit/$f"; done

for kit in claude codex kimi; do
  for skill in palette-init palette-rules palette-spec palette-ui palette-ux; do
    check_rendered_file "$ROOT/kits/${kit}-agent-kit/${kit}-skills/$skill/SKILL.md"
  done
done

# ── 4. harness-leak greps ─────────────────────────────────
# Claude-only machinery must not leak into codex/kimi renders, and vice versa.
# Allowlist: the surfaces intentionally say "workslate is Claude-only".

for kit in codex kimi; do
  leaks=$(grep -rEn 'workslate_task_|advisor\(\)|Agent Team|ScheduleWakeup|ultracode|CLAUDE\.md' \
      "$ROOT/kits/${kit}-agent-kit/${kit}-rules" "$ROOT/kits/${kit}-agent-kit/AGENTS.md" 2>/dev/null \
    | grep -Ev 'workslate.*Claude-only|Claude-only.*workslate' || true)
  if [ -n "$leaks" ]; then
    echo "claude-only machinery leaked into $kit render:" >&2
    echo "$leaks" | head -5 >&2
    fail=1
  fi
done

claude_leaks=$(grep -rEn 'AgentSwarm|TodoList|apply_patch|KIMI_CODE_HOME|CODEX_HOME|update_plan' \
    "$ROOT/kits/claude-agent-kit/CLAUDE.md" "$ROOT/kits/claude-agent-kit/claude-rules" 2>/dev/null || true)
if [ -n "$claude_leaks" ]; then
  echo "non-claude surface bindings leaked into claude render:" >&2
  echo "$claude_leaks" | head -5 >&2
  fail=1
fi

# ── 5. formal-language + stale-terminology ────────────────

if ! grep -R -n "polite formal" "$ROOT/shared/rules/core" >/dev/null; then
  echo "formal-language rule missing from core rules" >&2
  fail=1
fi

if grep -R -n "doctrine" "$ROOT" --exclude-dir=.git --exclude-dir=kits --exclude-dir=target | grep -v "/tooling/validate.sh:" >/dev/null; then
  echo "stale doctrine terminology found" >&2
  fail=1
fi

# ── 6. INV/GATE id integrity ──────────────────────────────
# Every referenced INV-*/GATE-* id must have exactly one bold definition line
# somewhere in the shared sources (+ palette).

src_all=$(cat "$ROOT"/shared/rules/core/*.md "$ROOT"/shared/rules/mcp/*.md "$ROOT/shared/workflows/palette/rules.md" 2>/dev/null)
refs=$(printf '%s' "$src_all" | grep -oE '(INV|GATE)(-[A-Z0-9]+)+' | sort -u)
for id in $refs; do
  defs=$(printf '%s' "$src_all" | grep -cE "^\*\*$id( |\.| —)" || true)
  if [ "$defs" -ne 1 ]; then
    echo "id $id has $defs definition lines (want exactly 1)" >&2
    fail=1
  fi
done

# ── 7. coverage-matrix anchors ────────────────────────────
# Every inventory line id must appear in the matrix; matrix rows must be
# non-empty in the new-location column (spot format check only — semantic
# review is human).

if [ -f "$ROOT/docs/coverage-matrix.md" ]; then
  if grep -n '| *|' "$ROOT/docs/coverage-matrix.md" | grep -v '^1:' >/dev/null; then
    echo "coverage-matrix has rows with an empty column" >&2
    fail=1
  fi
fi

# ── 8. size guards (simulated installer concat) ───────────

concat_lines() {
  kit="$1"; shift
  total=0
  for f in "$@"; do
    [ -f "$ROOT/kits/$kit/$f" ] || continue
    n=$(grep -c '' "$ROOT/kits/$kit/$f" || echo 0)
    total=$((total + n))
  done
  echo "$total"
}

codex_concat=$(concat_lines codex-agent-kit AGENTS.md \
  codex-rules/codex-agent-kit--codex-surface.md \
  codex-rules/codex-agent-kit--task-execution.md \
  codex-rules/codex-agent-kit--palette.md \
  codex-rules/codex-agent-kit--delegation.md \
  codex-rules/codex-agent-kit--git-workflow.md \
  codex-rules/codex-agent-kit--framework-conventions.md \
  codex-rules/codex-agent-kit--aside.md \
  codex-rules/codex-agent-kit--dispatch.md)
kimi_concat=$(concat_lines kimi-agent-kit AGENTS.md \
  kimi-rules/kimi-agent-kit--kimi-surface.md \
  kimi-rules/kimi-agent-kit--task-execution.md \
  kimi-rules/kimi-agent-kit--palette.md \
  kimi-rules/kimi-agent-kit--delegation.md \
  kimi-rules/kimi-agent-kit--git-workflow.md \
  kimi-rules/kimi-agent-kit--framework-conventions.md \
  kimi-rules/kimi-agent-kit--aside.md \
  kimi-rules/kimi-agent-kit--dispatch.md)

echo "concat size: codex=${codex_concat} lines, kimi=${kimi_concat} lines"
for pair in "codex:$codex_concat" "kimi:$kimi_concat"; do
  kit=${pair%%:*}
  n=${pair##*:}
  if [ "$n" -gt 1400 ]; then
    echo "WARNING: $kit concatenated AGENTS.md would be $n lines (>1400) — review for bloat" >&2
  fi
done

# ── 9. rendered titles ────────────────────────────────────

if ! grep -n "Codex Agent Operating Manual" "$ROOT/kits/codex-agent-kit/AGENTS.md" >/dev/null 2>&1; then
  echo "codex adapter did not render Codex title" >&2
  fail=1
fi

if ! grep -n "Kimi Agent Operating Manual" "$ROOT/kits/kimi-agent-kit/AGENTS.md" >/dev/null 2>&1; then
  echo "kimi adapter did not render Kimi title" >&2
  fail=1
fi

if ! grep -n "Claude Agent Operating Manual" "$ROOT/kits/claude-agent-kit/CLAUDE.md" >/dev/null 2>&1; then
  echo "claude adapter did not render Claude title" >&2
  fail=1
fi

# ── 10. installer sanity ──────────────────────────────────
# Guards the per-kit installers — previously untested. The signature check below
# would have caught the claude/kimi `--uninstall` no-op (install.sh keyed on the
# wrong signature and silently left every rule/skill installed).

for kit in claude codex kimi; do
  kd="$ROOT/kits/${kit}-agent-kit"
  for f in install.sh Makefile install.ps1; do
    [ -f "$kd/$f" ] || { echo "$kit: missing installer $f" >&2; fail=1; }
  done
  if [ -f "$kd/install.sh" ] && ! sh -n "$kd/install.sh" 2>/dev/null; then
    echo "$kit: install.sh has a syntax error" >&2; fail=1
  fi
  # install.sh + Makefile uninstall must key on the shared signature, or
  # `--uninstall` silently leaves every managed rule/skill in place.
  for f in install.sh Makefile; do
    if [ -f "$kd/$f" ] && ! grep -q 'slate-agent-kit:common' "$kd/$f"; then
      echo "$kit: $f never references 'slate-agent-kit:common' (uninstall would no-op)" >&2
      fail=1
    fi
  done
done

# Parse-check the PowerShell installers when pwsh is available (advisory otherwise).
if command -v pwsh >/dev/null 2>&1; then
  for kit in claude codex kimi; do
    ps1="$ROOT/kits/${kit}-agent-kit/install.ps1"
    [ -f "$ps1" ] || continue
    errcount=$(pwsh -NoProfile -Command "\$e=[ref]\$null; \$null=[System.Management.Automation.Language.Parser]::ParseFile('$ps1', [ref]\$null, \$e); \$e.Value.Count" 2>/dev/null)
    if [ "$errcount" != "0" ]; then
      echo "$kit: install.ps1 has parse error(s)" >&2; fail=1
    fi
  done
fi

if [ "$fail" -eq 0 ]; then
  echo "validate: OK"
fi
exit "$fail"
