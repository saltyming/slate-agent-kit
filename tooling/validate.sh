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
docs/glossary.rst
adapters/claude/tokens.sed
adapters/codex/tokens.sed
adapters/kimi/tokens.sed
adapters/agy/tokens.sed
adapters/claude/surface.md
adapters/codex/surface.md
adapters/kimi/surface.md
adapters/agy/surface.md
adapters/claude/inserts/models-table.md
adapters/codex/inserts/models-table.md
adapters/kimi/inserts/models-table.md
adapters/agy/inserts/models-table.md
shared/rules/constitution.md
shared/rules/implementing/execution.md
shared/rules/implementing/delegation.md
shared/rules/implementing/models.md
shared/rules/implementing/git.md
shared/rules/implementing/mcp/consult.md
shared/rules/implementing/mcp/dispatch.md
shared/workflows/palette/rules.md
shared/workflows/palette/skills/palette-init/SKILL.md
shared/workflows/palette/templates/house-style.rst
shared/workflows/memory/skills/memory-triage/SKILL.md
shared/prefs/consult-prefs.md.tmpl
shared/prefs/dispatch-prefs.md.tmpl
shared/prefs/subagent-prefs.md.tmpl
shared/prefs/git-prefs.md.tmpl
shared/prefs/comment-prefs.md.tmpl
shared/mcp-servers/dispatch/Cargo.toml
shared/crates/agent-exec/Cargo.toml
shared/crates/harness-log/Cargo.toml
shared/mcp-servers/palette/Cargo.toml
shared/setup/Cargo.toml
tooling/render-kit.sh
tooling/install-mcp.sh
tooling/slate-version
tooling/contract
tooling/kit-scripts/kit-maintainer.md.tmpl
tooling/kit-scripts/entry/install.sh.tmpl
tooling/kit-scripts/entry/install.ps1.tmpl
tooling/kit-scripts/entry/Makefile.tmpl
"

for path in $required; do
  if [ ! -e "$ROOT/$path" ]; then
    echo "missing: $path" >&2
    fail=1
  fi
done

for path in kits/claude-agent-kit kits/codex-agent-kit kits/kimi-agent-kit kits/agy-agent-kit; do
  if [ ! -d "$ROOT/$path/.git" ] && [ ! -f "$ROOT/$path/.git" ]; then
    echo "missing submodule checkout: $path" >&2
    fail=1
  fi
done

# ── 2. insert integrity ───────────────────────────────────
# Every {{@INSERT name}} marker in shared sources / surfaces must have an
# insert file for EVERY adapter (empty file = no contribution; missing
# file = hard error at render time — assert it here too). No orphan insert
# files, no nested markers inside insert files.

markers=$(grep -rhoE '^\{\{@INSERT [a-z0-9-]+\}\}$' \
    "$ROOT/shared/rules" "$ROOT/shared/workflows" \
    "$ROOT"/adapters/*/surface.md 2>/dev/null \
  | sed 's/{{@INSERT \([a-z0-9-]*\)}}/\1/' | sort -u)

for m in $markers; do
  for h in claude codex kimi agy; do
    if [ ! -f "$ROOT/adapters/$h/inserts/$m.md" ]; then
      echo "insert file missing for marker '$m': adapters/$h/inserts/$m.md" >&2
      fail=1
    fi
  done
done

for h in claude codex kimi agy; do
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

# ── 3. rendered output audit (every kit) ─────────────

check_rendered_file() {
  f="$1"
  if [ ! -f "$f" ]; then
    echo "missing rendered file: $f" >&2
    fail=1
    return
  fi
  # A render leak is an unrendered TOKEN — `{{KIT_VERSION}}`, `{{@INSERT …}}`,
  # `{{CONSULT_RULE_FILE}}` — which always opens `{{` + a token char (upper-case
  # or `@`). Match that shape, not bare `{{`/`}}`: shell and PowerShell
  # legitimately contain `}}` (e.g. nested `${VAR:=${VAR2:-}}`), which is not a
  # leak, so a bare-brace check false-positives on copied scripts.
  if grep -En '\{\{[A-Z@]' "$f" >/dev/null; then
    echo "unrendered placeholder in $f" >&2
    fail=1
  fi
}

for kit in claude codex kimi agy; do
  kd="$ROOT/kits/${kit}-agent-kit"
  primary=AGENTS.md
  [ "$kit" = claude ] && primary=CLAUDE.md
  check_rendered_file "$kd/dist/$primary"
  check_rendered_file "$kd/dist/kit.toml"
  check_rendered_file "$kd/AGENTS.md"
  for f in "$kd"/dist/rules/*.md "$kd"/dist/prefs/*.md "$kd"/dist/skills/*/SKILL.md "$kd"/dist/skills/palette-init/templates/*.rst; do
    check_rendered_file "$f"
  done
  for p in consult dispatch subagent git comment; do
    [ -f "$kd/dist/prefs/$p-prefs.md" ] || { echo "missing prefs template: $kd/dist/prefs/$p-prefs.md" >&2; fail=1; }
  done
  for skill in palette-init palette-resume palette-state palette-record palette-spec palette-ux palette-ui palette-rules memory-triage; do
    [ -f "$kd/dist/skills/$skill/SKILL.md" ] || { echo "missing skill: $kd/dist/skills/$skill/SKILL.md" >&2; fail=1; }
  done
  # The earlier layout kept the payload at the kit root; a leftover would be
  # loaded as the kit repository's own instructions.
  for old in "${kit}-rules" "${kit}-skills" scripts; do
    [ -e "$kd/$old" ] && { echo "earlier-layout leftover: $kd/$old" >&2; fail=1; }
  done
  [ "$kit" = claude ] && [ -e "$kd/CLAUDE.md" ] && { echo "earlier-layout leftover: $kd/CLAUDE.md" >&2; fail=1; }
done

# ── 3b. Antigravity frontmatter and per-file limit ────────
# Antigravity loads a rules-folder file only when it opens with a closed YAML
# frontmatter block, and drops a file over 24,000 bytes; the manual carries no
# block. The block is a fixed two-key shape, so the check is exact: the four
# lines, in order, with the description quoted and the signature right after.

agy_kd="$ROOT/kits/agy-agent-kit"
for f in "$agy_kd"/dist/rules/*.md "$agy_kd"/dist/prefs/*.md; do
  [ -f "$f" ] || continue
  l1=$(sed -n '1p' "$f"); l2=$(sed -n '2p' "$f"); l3=$(sed -n '3p' "$f"); l4=$(sed -n '4p' "$f"); l5=$(sed -n '5p' "$f")
  if [ "$l1" != "---" ] || [ "$l2" != "trigger: always_on" ] || [ "$l4" != "---" ] \
     || ! printf '%s' "$l3" | grep -Eq '^description: "[^"]+"$' \
     || ! printf '%s' "$l5" | grep -Eq '^<!-- (slate-agent-kit:common|agy-agent-kit(-custom:[a-z-]+)?) -->$'; then
    echo "agy frontmatter malformed (want ---, trigger: always_on, description: \"...\", ---, signature): $f" >&2
    fail=1
  fi
  if [ "$(wc -c < "$f")" -gt 24000 ]; then
    echo "agy rule file over Antigravity's 24,000-byte limit: $f" >&2
    fail=1
  fi
done
if [ -f "$agy_kd/dist/AGENTS.md" ] && [ "$(sed -n '1p' "$agy_kd/dist/AGENTS.md")" = "---" ]; then
  echo "agy manual must not carry frontmatter: $agy_kd/dist/AGENTS.md" >&2
  fail=1
fi

# ── 4. harness-leak greps ─────────────────────────────────
# Claude-only machinery must not leak into codex/kimi renders, and vice versa.
# workslate must not appear in any render.

for kit in codex kimi agy; do
  leaks=$(grep -rEn 'workslate|advisor\(\)|Agent Team|ScheduleWakeup|ultracode|CLAUDE\.md' \
      "$ROOT/kits/${kit}-agent-kit/dist/rules" "$ROOT/kits/${kit}-agent-kit/dist/AGENTS.md" 2>/dev/null || true)
  if [ -n "$leaks" ]; then
    echo "claude-only machinery leaked into $kit render:" >&2
    echo "$leaks" | head -5 >&2
    fail=1
  fi
done

claude_leaks=$(grep -rEn 'workslate|AgentSwarm|TodoList|apply_patch|KIMI_CODE_HOME|CODEX_HOME|update_plan|TaskOutput|disable_timeout|write_stdin|yield_time_ms|run_command|command_status|WaitMsBeforeAsync|invoke_subagent' \
    "$ROOT/kits/claude-agent-kit/dist/CLAUDE.md" "$ROOT/kits/claude-agent-kit/dist/rules" 2>/dev/null || true)
if [ -n "$claude_leaks" ]; then
  echo "non-claude surface bindings leaked into claude render:" >&2
  echo "$claude_leaks" | head -5 >&2
  fail=1
fi

# The waiting means of one harness must not leak into another render.
codex_wait_leaks=$(grep -rEn 'TaskOutput|disable_timeout|run_in_background|run_command|command_status' "$ROOT/kits/codex-agent-kit/dist/rules" "$ROOT/kits/codex-agent-kit/dist/AGENTS.md" 2>/dev/null || true)
if [ -n "$codex_wait_leaks" ]; then
  echo "another harness's waiting means leaked into codex render:" >&2
  echo "$codex_wait_leaks" | head -5 >&2
  fail=1
fi
kimi_wait_leaks=$(grep -rEn -- 'write_stdin|yield_time_ms|run_in_background: true|run_command|command_status' "$ROOT/kits/kimi-agent-kit/dist/rules" "$ROOT/kits/kimi-agent-kit/dist/AGENTS.md" 2>/dev/null || true)
if [ -n "$kimi_wait_leaks" ]; then
  echo "another harness's waiting means leaked into kimi render:" >&2
  echo "$kimi_wait_leaks" | head -5 >&2
  fail=1
fi

agy_leaks=$(grep -rEn 'TaskOutput|disable_timeout|run_in_background|write_stdin|yield_time_ms|AgentSwarm|TodoList|apply_patch|update_plan|spawn_agent|KIMI_CODE_HOME|CODEX_HOME' "$ROOT/kits/agy-agent-kit/dist/rules" "$ROOT/kits/agy-agent-kit/dist/AGENTS.md" 2>/dev/null || true)
if [ -n "$agy_leaks" ]; then
  echo "another harness's surface or waiting means leaked into agy render:" >&2
  echo "$agy_leaks" | head -5 >&2
  fail=1
fi

# A retired server leaves no name behind in the rules, prefs, adapters and
# descriptors; records and changelogs keep the history.
retired_server=$(grep -rniE '\baside\b' "$ROOT/shared/rules" "$ROOT/shared/prefs" "$ROOT/adapters" "$ROOT/tooling/render-kit.sh" \
    "$ROOT"/kits/*-agent-kit/dist/rules "$ROOT"/kits/*-agent-kit/dist/prefs "$ROOT"/kits/*-agent-kit/dist/kit.toml \
    "$ROOT"/kits/*-agent-kit/dist/AGENTS.md "$ROOT"/kits/*-agent-kit/dist/CLAUDE.md "$ROOT"/kits/*-agent-kit/AGENTS.md 2>/dev/null || true)
if [ -n "$retired_server" ]; then
  echo "retired server named in sources or renders:" >&2
  echo "$retired_server" | head -5 >&2
  fail=1
fi

# ── 5. formal-language + stale-terminology ────────────────

if ! grep -n "polite informal register" "$ROOT/shared/rules/constitution.md" >/dev/null; then
  echo "register rule missing from the constitution" >&2
  fail=1
fi

if grep -R -n "doctrine" "$ROOT" --exclude-dir=.git --exclude-dir=kits --exclude-dir=target | grep -v "/tooling/validate.sh:" >/dev/null; then
  echo "stale doctrine terminology found" >&2
  fail=1
fi

# ── 5b. retired terms and trigger phrases ─────────────────
# The rules and manuals use the glossary's terms (docs/glossary.rst) and state
# purpose and judgment, never a mandatory trigger.

for kit in claude codex kimi agy; do
  hits=$(grep -rniE '\b(story|stories|slice|slices|handoff|hand-off)\b|\\b(INV|GATE)-[A-Z]|whether or not the user asked|do not reconsider|use them without asking' \
      "$ROOT/kits/${kit}-agent-kit/dist/rules" "$ROOT"/kits/${kit}-agent-kit/dist/*.md 2>/dev/null || true)
  if [ -n "$hits" ]; then
    echo "retired term or trigger phrase in $kit render:" >&2
    echo "$hits" | head -5 >&2
    fail=1
  fi
done

# A retired backend leaves no name behind: not in the sources the kits are
# built from and not in what a kit installs.
retired_backend=$(grep -rniE 'copilot' "$ROOT/shared" "$ROOT/adapters" \
    "$ROOT"/kits/*-agent-kit/dist --exclude-dir=target 2>/dev/null || true)
if [ -n "$retired_backend" ]; then
  echo "retired backend named in sources or renders:" >&2
  echo "$retired_backend" | head -5 >&2
  fail=1
fi

# The dispatch tool descriptions and get_info name the dispatch preferences as
# they are (a level, backend, model, effort, fallback), not the earlier
# policies and approval modes.
retired_policy=$(grep -rniE 'conservative|preference-only|proactive|approval mode|approval granularity' \
    "$ROOT/shared/mcp-servers/dispatch/src" 2>/dev/null || true)
if [ -n "$retired_policy" ]; then
  echo "retired dispatch policy term in the dispatch sources:" >&2
  echo "$retired_policy" | head -5 >&2
  fail=1
fi

# ── 6. article id integrity ───────────────────────────────
# Every referenced article (§ N, optionally with a letter suffix) must have
# exactly one bold definition line, in the constitution.

src_all=$(cat "$ROOT"/shared/rules/*.md "$ROOT"/shared/rules/implementing/*.md "$ROOT"/shared/rules/implementing/mcp/*.md "$ROOT/shared/workflows/palette/rules.md" \
  "$ROOT"/shared/workflows/*/skills/*/SKILL.md "$ROOT"/adapters/*/inserts/*.md "$ROOT"/adapters/*/surface.md 2>/dev/null)
refs=$(printf '%s' "$src_all" | grep -oE '§ [0-9]+[a-z]?' | sort -u | sed 's/§ //')
for id in $refs; do
  defs=$(printf '%s' "$src_all" | grep -cE "^\*\*§ $id( |\.)" || true)
  if [ "$defs" -ne 1 ]; then
    echo "id $id has $defs definition lines (want exactly 1)" >&2
    fail=1
  fi
done

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

codex_concat=$(concat_lines codex-agent-kit dist/AGENTS.md $(cd "$ROOT/kits/codex-agent-kit" && ls dist/rules/*.md 2>/dev/null))
kimi_concat=$(concat_lines kimi-agent-kit dist/AGENTS.md $(cd "$ROOT/kits/kimi-agent-kit" && ls dist/rules/*.md 2>/dev/null))

echo "concat size: codex=${codex_concat} lines, kimi=${kimi_concat} lines"
for pair in "codex:$codex_concat" "kimi:$kimi_concat"; do
  kit=${pair%%:*}
  n=${pair##*:}
  if [ "$n" -gt 1400 ]; then
    echo "WARNING: $kit concatenated AGENTS.md would be $n lines (>1400) — review for bloat" >&2
  fi
done

# ── 8b. standing-corpus byte budgets (hard) ───────────────
# The rendered standing rules are injected into every session; per-rule
# compliance degrades as the corpus grows, so growth past these ceilings is a
# regression, not a style issue. Raising a ceiling is a deliberate release
# decision, never a drive-by. Prefs templates are user-owned config and live in
# dist/prefs/, outside the corpus.

corpus_bytes() {
  total=0
  for f in "$@"; do
    [ -f "$f" ] || continue
    n=$(wc -c < "$f")
    total=$((total + n))
  done
  echo "$total"
}

claude_bytes=$(corpus_bytes "$ROOT/kits/claude-agent-kit/dist/CLAUDE.md" "$ROOT"/kits/claude-agent-kit/dist/rules/*.md)
codex_bytes=$(corpus_bytes "$ROOT/kits/codex-agent-kit/dist/AGENTS.md" "$ROOT"/kits/codex-agent-kit/dist/rules/*.md)
kimi_bytes=$(corpus_bytes "$ROOT/kits/kimi-agent-kit/dist/AGENTS.md" "$ROOT"/kits/kimi-agent-kit/dist/rules/*.md)
agy_bytes=$(corpus_bytes "$ROOT/kits/agy-agent-kit/dist/AGENTS.md" "$ROOT"/kits/agy-agent-kit/dist/rules/*.md)

echo "standing corpus: claude=${claude_bytes}B codex=${codex_bytes}B kimi=${kimi_bytes}B agy=${agy_bytes}B"
budget_check() {
  kit="$1"; n="$2"; cap="$3"
  if [ "$n" -gt "$cap" ]; then
    echo "$kit standing corpus is ${n} bytes (budget ${cap}) — raising a ceiling is a release decision, recorded in an ADR" >&2
    fail=1
  fi
}
budget_check claude "$claude_bytes" 34000
budget_check codex "$codex_bytes" 34000
budget_check kimi "$kimi_bytes" 33000
budget_check agy "$agy_bytes" 34000

# ── 9. rendered titles ────────────────────────────────────

if ! grep -n "Codex Agent Operating Manual" "$ROOT/kits/codex-agent-kit/dist/AGENTS.md" >/dev/null 2>&1; then
  echo "codex adapter did not render Codex title" >&2
  fail=1
fi

if ! grep -n "Kimi Agent Operating Manual" "$ROOT/kits/kimi-agent-kit/dist/AGENTS.md" >/dev/null 2>&1; then
  echo "kimi adapter did not render Kimi title" >&2
  fail=1
fi

if ! grep -n "Antigravity Agent Operating Manual" "$ROOT/kits/agy-agent-kit/dist/AGENTS.md" >/dev/null 2>&1; then
  echo "agy adapter did not render Antigravity title" >&2
  fail=1
fi

if ! grep -n "Claude Agent Operating Manual" "$ROOT/kits/claude-agent-kit/dist/CLAUDE.md" >/dev/null 2>&1; then
  echo "claude adapter did not render Claude title" >&2
  fail=1
fi

# ── 10. installer sanity ──────────────────────────────────
# The entry points only obtain and run slate-setup; the descriptor tells it
# what to install. Both are rendered, so a missing or malformed one means the
# render is stale.

for kit in claude codex kimi agy; do
  kd="$ROOT/kits/${kit}-agent-kit"
  for f in install.sh Makefile install.ps1; do
    [ -f "$kd/$f" ] || { echo "$kit: missing entry point $f" >&2; fail=1; }
  done
  if [ -f "$kd/install.sh" ] && ! sh -n "$kd/install.sh" 2>/dev/null; then
    echo "$kit: install.sh has a syntax error" >&2; fail=1
  fi
  for key in kit harness version contract slate_version primary load rules skills prefs servers; do
    if [ -f "$kd/dist/kit.toml" ] && ! grep -q "^$key = " "$kd/dist/kit.toml"; then
      echo "$kit: dist/kit.toml lacks '$key'" >&2; fail=1
    fi
  done
done

# Parse-check the PowerShell installers when pwsh is available (advisory otherwise).
if command -v pwsh >/dev/null 2>&1; then
  for kit in claude codex kimi agy; do
    ps1="$ROOT/kits/${kit}-agent-kit/install.ps1"
    [ -f "$ps1" ] || continue
    errcount=$(pwsh -NoProfile -Command "\$e=[ref]\$null; \$null=[System.Management.Automation.Language.Parser]::ParseFile('$ps1', [ref]\$null, \$e); \$e.Value.Count" 2>/dev/null)
    if [ "$errcount" != "0" ]; then
      echo "$kit: install.ps1 has parse error(s)" >&2; fail=1
    fi
  done
fi

# ── 11. palette documents ─────────────────────────────────
# This repository's own palette and docs/ pass the palette lint. The check runs
# when a palette binary is available (CI builds it first); otherwise it is
# reported as skipped.

palette_bin=""
if [ -x "$ROOT/target/release/palette" ]; then palette_bin="$ROOT/target/release/palette"
elif [ -x "$ROOT/target/debug/palette" ]; then palette_bin="$ROOT/target/debug/palette"
elif command -v palette >/dev/null 2>&1; then palette_bin=$(command -v palette)
fi
if [ -n "$palette_bin" ]; then
  if ! "$palette_bin" check "$ROOT" >&2; then
    echo "palette check failed on this repository" >&2
    fail=1
  fi
else
  echo "palette check skipped: no palette binary built"
fi

if [ "$fail" -eq 0 ]; then
  echo "validate: OK"
fi
exit "$fail"
