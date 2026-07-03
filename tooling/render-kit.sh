#!/bin/sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"

usage() {
  echo "Usage: $0 <claude|codex|kimi> [target-dir]" >&2
  exit 2
}

harness="${1:-}"
[ -n "$harness" ] || usage

surface_src=""
surface_name=""
case "$harness" in
  claude)
    target="${2:-$ROOT/kits/claude-agent-kit}"
    rules_dir="$target/claude-rules"
    skills_dir="$target/claude-skills"
    primary="$target/CLAUDE.md"
    prefix="claude-agent-kit"
    delegation_name="parallel-work"
    ;;
  codex)
    target="${2:-$ROOT/kits/codex-agent-kit}"
    rules_dir="$target/codex-rules"
    skills_dir="$target/codex-skills"
    primary="$target/AGENTS.md"
    prefix="codex-agent-kit"
    delegation_name="delegation"
    surface_src="$ROOT/adapters/codex/surface.md"
    surface_name="codex-surface"
    ;;
  kimi)
    target="${2:-$ROOT/kits/kimi-agent-kit}"
    rules_dir="$target/kimi-rules"
    skills_dir="$target/kimi-skills"
    primary="$target/AGENTS.md"
    prefix="kimi-agent-kit"
    delegation_name="delegation"
    surface_src="$ROOT/adapters/kimi/surface.md"
    surface_name="kimi-surface"
    ;;
  *) usage ;;
esac

sed_script="$ROOT/adapters/$harness/tokens.sed"
[ -f "$sed_script" ] || {
  echo "missing adapter sed script: $sed_script" >&2
  exit 1
}
inserts_dir="$ROOT/adapters/$harness/inserts"
[ -d "$inserts_dir" ] || {
  echo "missing adapter inserts dir: $inserts_dir" >&2
  exit 1
}

mkdir -p "$rules_dir" "$skills_dir"

# Render one source file: expand `{{@INSERT <name>}}` marker lines from
# adapters/<harness>/inserts/<name>.md (an empty file means "this harness
# contributes nothing here"; a MISSING file is a hard error so a typo'd marker
# can never silently drop content), then apply token substitution — insert
# bodies get tokens expanded too.
render() {
  src="$1"
  dest="$2"
  mkdir -p "$(dirname -- "$dest")"
  awk -v dir="$inserts_dir" '
    /^\{\{@INSERT [a-z0-9-]+\}\}$/ {
      name = $2
      sub(/\}\}$/, "", name)
      path = dir "/" name ".md"
      rc = (getline line < path)
      if (rc < 0) {
        printf("missing insert file: %s\n", path) > "/dev/stderr"
        err = 1
        next
      }
      if (rc > 0) {
        print line
        while ((getline line < path) > 0) print line
      }
      close(path)
      next
    }
    { print }
    END { if (err) exit 1 }
  ' "$src" | sed -f "$sed_script" > "$dest"
}

render "$ROOT/shared/rules/core/kernel.md" "$primary"
render "$ROOT/shared/rules/core/loop-execution.md" "$rules_dir/${prefix}--task-execution.md"
render "$ROOT/shared/rules/core/loop-delegation.md" "$rules_dir/${prefix}--${delegation_name}.md"
render "$ROOT/shared/rules/core/git-workflow.md" "$rules_dir/${prefix}--git-workflow.md"
render "$ROOT/shared/rules/core/conventions.md" "$rules_dir/${prefix}--framework-conventions.md"
render "$ROOT/shared/workflows/palette/rules.md" "$rules_dir/${prefix}--palette.md"
render "$ROOT/shared/rules/mcp/aside.md" "$rules_dir/${prefix}--aside.md"
render "$ROOT/shared/rules/mcp/dispatch.md" "$rules_dir/${prefix}--dispatch.md"

if [ -n "$surface_src" ]; then
  render "$surface_src" "$rules_dir/${prefix}--${surface_name}.md"
fi

for skill in palette-init palette-rules palette-spec palette-ui palette-ux; do
  render "$ROOT/shared/workflows/palette/skills/$skill/SKILL.md" "$skills_dir/$skill/SKILL.md"
done

# Prefs templates render for every kit (configure-time values use @@NAME@@
# placeholders, distinct from render-time {{TOKEN}}s). claude keeps its own
# battle-tested interactive configure scripts; codex/kimi get the generic
# configure-prefs.sh.
mkdir -p "$target/scripts"
render "$ROOT/shared/prefs/aside-prefs.md.tmpl" "$target/scripts/${prefix}--aside-prefs.md.tmpl"
render "$ROOT/shared/prefs/dispatch-prefs.md.tmpl" "$target/scripts/${prefix}--dispatch-prefs.md.tmpl"
if [ "$harness" != "claude" ]; then
  cp "$ROOT/tooling/kit-scripts/configure-prefs.sh" "$target/scripts/configure-prefs.sh"
  chmod +x "$target/scripts/configure-prefs.sh"
fi

echo "rendered $harness kit into $target"
