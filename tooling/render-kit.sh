#!/bin/sh
# Render one harness kit from the slate sources: the installable payload under
# <kit>/dist/ (manual, rules, skills with the palette templates, prefs
# templates, and the kit.toml descriptor the installer reads), the kit's entry
# points (install.sh, install.ps1, Makefile) and the maintainer AGENTS.md at the
# kit root. Paths the earlier layout used are removed so a stale payload never
# ships. Token values come from adapters/<harness>/tokens.sed and
# tooling/slate-version; {{@INSERT name}} lines come from adapters/<harness>/inserts/.
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
legacy=""
case "$harness" in
  claude)
    target="${2:-$ROOT/kits/claude-agent-kit}"
    primary="CLAUDE.md"
    prefix="claude-agent-kit"
    delegation_name="parallel-work"
    load="rules-dir"
    legacy='"workslate"'
    ;;
  codex)
    target="${2:-$ROOT/kits/codex-agent-kit}"
    primary="AGENTS.md"
    prefix="codex-agent-kit"
    delegation_name="delegation"
    load="concat"
    surface_src="$ROOT/adapters/codex/surface.md"
    surface_name="codex-surface"
    ;;
  kimi)
    target="${2:-$ROOT/kits/kimi-agent-kit}"
    primary="AGENTS.md"
    prefix="kimi-agent-kit"
    delegation_name="delegation"
    load="concat"
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
slate_version=$(sed -n '1p' "$ROOT/tooling/slate-version")
[ -n "$slate_version" ] || {
  echo "tooling/slate-version is empty" >&2
  exit 1
}
kit_version=$(sed -n 's/^s#{{KIT_VERSION}}#\([^#]*\)#g$/\1/p' "$sed_script")
[ -n "$kit_version" ] || {
  echo "KIT_VERSION missing from $sed_script" >&2
  exit 1
}

entry_dir="$ROOT/tooling/kit-scripts/entry"
for f in install.sh.tmpl install.ps1.tmpl Makefile.tmpl; do
  [ -f "$entry_dir/$f" ] || {
    echo "missing entry point template: $entry_dir/$f" >&2
    exit 1
  }
done

# Render one source file: expand `{{@INSERT <name>}}` marker lines from
# adapters/<harness>/inserts/<name>.md (an empty file means "this harness
# contributes nothing here"; a MISSING file is a hard error so a typo'd marker
# can never silently drop content), then apply token substitution — insert
# bodies get tokens expanded too.
render() {
  src="$1"
  dest="$2"
  mkdir -p "$(dirname -- "$dest")"
  # POSIX sh has no `pipefail`, so a naive `awk … | sed > dest` would discard
  # awk's exit status (the pipeline reports sed's) and write a truncated file on
  # a missing insert — defeating the hard-error guard above. Stage awk to a temp,
  # check its status, then run sed.
  _render_tmp="$dest.render-tmp.$$"
  if awk -v dir="$inserts_dir" '
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
  ' "$src" > "$_render_tmp"; then
    sed -f "$sed_script" "$_render_tmp" > "$dest"
    rm -f "$_render_tmp"
  else
    rm -f "$_render_tmp"
    echo "render failed: $src (missing insert file or unreadable source)" >&2
    exit 1
  fi
}

# Substitute the entry-point and maintainer tokens, which are per kit and not
# part of any adapter's tokens.sed.
render_entry() {
  src="$1"
  dest="$2"
  sed \
    -e "s#{{KIT_NAME}}#$prefix#g" \
    -e "s#{{KIT_REPO}}#saltyming/$prefix#g" \
    -e "s#{{SLATE_REPO}}#saltyming/slate-agent-kit#g" \
    -e "s#{{SLATE_VERSION}}#$slate_version#g" \
    -e "s#{{HARNESS}}#$harness#g" \
    -e "s#{{PRIMARY_MANUAL_FILE}}#$primary#g" \
    "$src" > "$dest"
}

dist="$target/dist"
rm -rf "$dist"
mkdir -p "$dist/rules" "$dist/skills" "$dist/prefs"

# Paths of the earlier layout, which had the payload at the kit root.
rm -rf "$target/${harness}-rules" "$target/${harness}-skills" "$target/scripts"
[ "$harness" = "claude" ] && rm -f "$target/CLAUDE.md"

render "$ROOT/shared/rules/core/kernel.md" "$dist/$primary"

# Rule files in the order a concatenating harness loads them.
rules=""
add_rule() {
  render "$1" "$dist/rules/${prefix}--$2.md"
  rules="$rules \"${prefix}--$2.md\","
}
if [ -n "$surface_src" ]; then
  add_rule "$surface_src" "$surface_name"
fi
add_rule "$ROOT/shared/rules/core/loop-execution.md" task-execution
add_rule "$ROOT/shared/workflows/palette/rules.md" palette
add_rule "$ROOT/shared/rules/core/loop-delegation.md" "$delegation_name"
add_rule "$ROOT/shared/rules/core/git-workflow.md" git-workflow
add_rule "$ROOT/shared/rules/mcp/aside.md" aside
add_rule "$ROOT/shared/rules/mcp/dispatch.md" dispatch

skills=""
for skill in palette-init palette-resume palette-state palette-record palette-spec palette-ux palette-ui palette-rules; do
  render "$ROOT/shared/workflows/palette/skills/$skill/SKILL.md" "$dist/skills/$skill/SKILL.md"
  skills="$skills \"$skill\","
done
for t in "$ROOT"/shared/workflows/palette/templates/*.rst; do
  render "$t" "$dist/skills/palette-init/templates/$(basename "$t")"
done
render "$ROOT/shared/workflows/memory/skills/memory-triage/SKILL.md" "$dist/skills/memory-triage/SKILL.md"
skills="$skills \"memory-triage\","

prefs=""
for p in aside dispatch subagent git comment; do
  render "$ROOT/shared/prefs/$p-prefs.md.tmpl" "$dist/prefs/$p-prefs.md"
  prefs="$prefs \"$p\","
done

# The descriptor the installer reads (spec/installer.rst, Descriptor).
{
  echo "kit = \"$prefix\""
  echo "harness = \"$harness\""
  echo "version = \"$kit_version\""
  echo "slate_version = \"$slate_version\""
  echo "primary = \"$primary\""
  echo "load = \"$load\""
  echo "rules = [${rules%,} ]"
  echo "skills = [${skills%,} ]"
  echo "prefs = [${prefs%,} ]"
  echo 'servers = [ "aside", "dispatch", "palette" ]'
  echo "legacy = [ $legacy ]"
} > "$dist/kit.toml"

render_entry "$entry_dir/install.sh.tmpl" "$target/install.sh"
chmod +x "$target/install.sh"
render_entry "$entry_dir/install.ps1.tmpl" "$target/install.ps1"
render_entry "$entry_dir/Makefile.tmpl" "$target/Makefile"
render_entry "$ROOT/tooling/kit-scripts/kit-maintainer.md.tmpl" "$target/AGENTS.md"

echo "rendered $harness kit into $target"
