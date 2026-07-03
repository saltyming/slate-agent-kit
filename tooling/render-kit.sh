#!/bin/sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"

usage() {
  echo "Usage: $0 <claude|codex|kimi> [target-dir]" >&2
  exit 2
}

harness="${1:-}"
[ -n "$harness" ] || usage

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
    ;;
  kimi)
    target="${2:-$ROOT/kits/kimi-agent-kit}"
    rules_dir="$target/kimi-rules"
    skills_dir="$target/kimi-skills"
    primary="$target/AGENTS.md"
    prefix="kimi-agent-kit"
    delegation_name="delegation"
    ;;
  *) usage ;;
esac

sed_script="$ROOT/adapters/$harness/tokens.sed"
[ -f "$sed_script" ] || {
  echo "missing adapter sed script: $sed_script" >&2
  exit 1
}

mkdir -p "$rules_dir" "$skills_dir"

render() {
  src="$1"
  dest="$2"
  mkdir -p "$(dirname -- "$dest")"
  sed -f "$sed_script" "$src" > "$dest"
}

render "$ROOT/shared/rules/common/operating-manual.md" "$primary"
render "$ROOT/shared/rules/common/task-execution.md" "$rules_dir/${prefix}--task-execution.md"
render "$ROOT/shared/rules/common/delegation.md" "$rules_dir/${prefix}--${delegation_name}.md"
render "$ROOT/shared/rules/common/git-workflow.md" "$rules_dir/${prefix}--git-workflow.md"
render "$ROOT/shared/rules/common/framework-conventions.md" "$rules_dir/${prefix}--framework-conventions.md"
render "$ROOT/shared/workflows/palette/rules.md" "$rules_dir/${prefix}--palette.md"
render "$ROOT/shared/rules/mcp/aside.md" "$rules_dir/${prefix}--aside.md"
render "$ROOT/shared/rules/mcp/dispatch.md" "$rules_dir/${prefix}--dispatch.md"

for skill in palette-init palette-rules palette-spec palette-ui palette-ux; do
  render "$ROOT/shared/workflows/palette/skills/$skill/SKILL.md" "$skills_dir/$skill/SKILL.md"
done

echo "rendered $harness kit into $target"
