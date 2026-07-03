#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
fail=0

required="
.gitmodules
README.md
LICENSE.md
Cargo.toml
docs/architecture.md
docs/support-matrix.md
adapters/claude/tokens.sed
adapters/codex/tokens.sed
adapters/kimi/tokens.sed
shared/rules/common/operating-manual.md
shared/rules/common/task-execution.md
shared/rules/common/delegation.md
shared/rules/common/git-workflow.md
shared/rules/common/framework-conventions.md
shared/rules/mcp/aside.md
shared/rules/mcp/dispatch.md
shared/workflows/palette/rules.md
shared/workflows/palette/skills/palette-init/SKILL.md
shared/mcp-servers/aside/Cargo.toml
shared/mcp-servers/dispatch/Cargo.toml
tooling/render-kit.sh
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

if ! grep -R -n "polite formal" "$ROOT/shared/rules/common" >/dev/null; then
  echo "formal-language rule missing from common rules" >&2
  fail=1
fi

if grep -R -n "doctrine" "$ROOT" --exclude-dir=.git --exclude-dir=kits --exclude-dir=target | grep -v "/tooling/validate.sh:" >/dev/null; then
  echo "stale doctrine terminology found" >&2
  fail=1
fi

for rendered in \
  "$ROOT/kits/codex-agent-kit/AGENTS.md" \
  "$ROOT/kits/kimi-agent-kit/AGENTS.md"
do
  if grep -n "{{\\|}}" "$rendered" >/dev/null; then
    echo "unrendered placeholder in $rendered" >&2
    fail=1
  fi
done

if ! grep -R -n "Codex Agent Operating Manual" "$ROOT/kits/codex-agent-kit/AGENTS.md" >/dev/null; then
  echo "codex adapter did not render Codex title" >&2
  fail=1
fi

if ! grep -R -n "Kimi Agent Operating Manual" "$ROOT/kits/kimi-agent-kit/AGENTS.md" >/dev/null; then
  echo "kimi adapter did not render Kimi title" >&2
  fail=1
fi

exit "$fail"
