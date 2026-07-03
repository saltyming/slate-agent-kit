#!/bin/sh
# Generate the aside/dispatch preference files for a rendered kit.
#
# Harness-neutral: the kit's Makefile/install.sh invokes this with
#   RULES_DIR=<harness rules dir> PREFIX=<kit prefix> [MANIFEST=<file>] \
#     sh scripts/configure-prefs.sh
# Templates are looked up next to this script as
#   <PREFIX>--aside-prefs.md.tmpl / <PREFIX>--dispatch-prefs.md.tmpl
# (rendered from shared/prefs/ by slate's render-kit.sh).
#
# Values come from environment variables with safe defaults; on an interactive
# TTY the three high-impact knobs are prompted (unless PREFS_PROMPT=no).
# Existing prefs files are preserved unless PREFS_RECONFIGURE=yes — they are
# user-owned (custom signature).
set -eu

HERE="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
: "${RULES_DIR:?RULES_DIR is required}"
: "${PREFIX:?PREFIX is required}"
MANIFEST="${MANIFEST:-}"

ASIDE_PREFERRED="${ASIDE_PREFERRED:-none}"
ASIDE_CODEX_MODEL="${ASIDE_CODEX_MODEL:-}"
ASIDE_COPILOT_MODEL="${ASIDE_COPILOT_MODEL:-}"
ASIDE_CODEX_EFFORT="${ASIDE_CODEX_EFFORT:-}"
ASIDE_COPILOT_EFFORT="${ASIDE_COPILOT_EFFORT:-}"
ASIDE_CODEX_FALLBACK="${ASIDE_CODEX_FALLBACK:-}"
ASIDE_COPILOT_FALLBACK="${ASIDE_COPILOT_FALLBACK:-}"
ASIDE_POLICY="${ASIDE_POLICY:-conservative}"

DISPATCH_POLICY="${DISPATCH_POLICY:-conservative}"
DISPATCH_APPROVAL="${DISPATCH_APPROVAL:-ask}"
DISPATCH_GRANULARITY="${DISPATCH_GRANULARITY:-ask}"
DISPATCH_BACKEND="${DISPATCH_BACKEND:-}"
DISPATCH_MODEL="${DISPATCH_MODEL:-}"
DISPATCH_EFFORT="${DISPATCH_EFFORT:-}"
DISPATCH_FALLBACK="${DISPATCH_FALLBACK:-}"

prompt_value() {
  # prompt_value <label> <current> — echoes the chosen value
  label="$1"
  current="$2"
  printf '%s [%s]: ' "$label" "$current" >&2
  IFS= read -r answer || answer=""
  if [ -n "$answer" ]; then
    printf '%s' "$answer"
  else
    printf '%s' "$current"
  fi
}

if [ -t 0 ] && [ "${PREFS_PROMPT:-yes}" != "no" ]; then
  echo "Configuring aside/dispatch preferences (Enter keeps the default)." >&2
  ASIDE_PREFERRED=$(prompt_value "aside preferred backend (none/codex/copilot)" "$ASIDE_PREFERRED")
  ASIDE_POLICY=$(prompt_value "aside auto-call policy (conservative/preference-only/proactive)" "$ASIDE_POLICY")
  DISPATCH_POLICY=$(prompt_value "dispatch execution policy (conservative/preference-only/proactive)" "$DISPATCH_POLICY")
  DISPATCH_APPROVAL=$(prompt_value "dispatch approval mode (ask/auto)" "$DISPATCH_APPROVAL")
fi

render_prefs() {
  tmpl="$1"
  dest="$2"
  [ -f "$tmpl" ] || {
    echo "Error: template not found: $tmpl" >&2
    exit 1
  }
  if [ -f "$dest" ] && [ "${PREFS_RECONFIGURE:-no}" != "yes" ]; then
    echo "  prefs exist, keeping: $dest (set PREFS_RECONFIGURE=yes to regenerate)"
    return 0
  fi
  mkdir -p "$(dirname -- "$dest")"
  sed \
    -e "s|@@ASIDE_PREFERRED@@|$ASIDE_PREFERRED|g" \
    -e "s|@@ASIDE_CODEX_MODEL@@|$ASIDE_CODEX_MODEL|g" \
    -e "s|@@ASIDE_COPILOT_MODEL@@|$ASIDE_COPILOT_MODEL|g" \
    -e "s|@@ASIDE_CODEX_EFFORT@@|$ASIDE_CODEX_EFFORT|g" \
    -e "s|@@ASIDE_COPILOT_EFFORT@@|$ASIDE_COPILOT_EFFORT|g" \
    -e "s|@@ASIDE_CODEX_FALLBACK@@|$ASIDE_CODEX_FALLBACK|g" \
    -e "s|@@ASIDE_COPILOT_FALLBACK@@|$ASIDE_COPILOT_FALLBACK|g" \
    -e "s|@@ASIDE_POLICY@@|$ASIDE_POLICY|g" \
    -e "s|@@DISPATCH_POLICY@@|$DISPATCH_POLICY|g" \
    -e "s|@@DISPATCH_APPROVAL@@|$DISPATCH_APPROVAL|g" \
    -e "s|@@DISPATCH_GRANULARITY@@|$DISPATCH_GRANULARITY|g" \
    -e "s|@@DISPATCH_BACKEND@@|$DISPATCH_BACKEND|g" \
    -e "s|@@DISPATCH_MODEL@@|$DISPATCH_MODEL|g" \
    -e "s|@@DISPATCH_EFFORT@@|$DISPATCH_EFFORT|g" \
    -e "s|@@DISPATCH_FALLBACK@@|$DISPATCH_FALLBACK|g" \
    "$tmpl" > "$dest"
  echo "  prefs: $dest"
  if [ -n "$MANIFEST" ]; then
    echo "$dest" >> "$MANIFEST"
  fi
}

render_prefs "$HERE/${PREFIX}--aside-prefs.md.tmpl" "$RULES_DIR/${PREFIX}--aside-prefs.md"
render_prefs "$HERE/${PREFIX}--dispatch-prefs.md.tmpl" "$RULES_DIR/${PREFIX}--dispatch-prefs.md"
