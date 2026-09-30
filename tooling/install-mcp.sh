#!/bin/sh
# install-mcp.sh - thin wrapper that builds slate-setup and runs `slate-setup mcp`.
#
# It installs the shared MCP servers (aside, dispatch, palette) and registers
# them with Claude Code, Codex and Kimi Code. Every step lives in slate-setup;
# this script only translates its documented options and environment variables.
set -eu

ROOT="$(unset CDPATH; cd -- "$(dirname -- "$0")/.." && pwd)"

usage() {
  cat <<'USAGE'
Usage: tooling/install-mcp.sh [options]

Build and install the shared Slate MCP servers (aside, dispatch, palette), then
register them for one or more harnesses. All work is done by `slate-setup mcp`.

Options:
  --install-only          Build/copy the servers, do not configure a harness
  --configure-claude      Build/copy and register via `claude mcp add -s user`
  --configure-codex       Build/copy and register via `codex mcp add`
  --configure-kimi        Build/copy and register a Kimi local plugin
  --configure-all         Build/copy and configure Claude + Codex + Kimi
  --uninstall-claude      Remove the servers from Claude user-scope MCP config
  --uninstall-codex       Remove the servers from Codex config
  --uninstall-kimi        Remove the Kimi local plugin registration and files
  --roots DIRS            Absolute workspace roots for dispatch and palette
                          containment, separated like PATH (':' on Unix).
                          Required for Kimi (its plugin runtime spawns MCP
                          servers outside any project, so there is no project
                          root there) and recommended for Codex.
  --bin-dir DIR           Install binaries into DIR (default: $HOME/.local/bin)
  --prebuilt              Download prebuilt binaries from GitHub Releases instead
                          of building with cargo. Also used automatically when
                          cargo is unavailable. "latest" may be newer than this
                          checkout; pin with SLATE_RELEASE_TAG.
  -y, --yes               Ask nothing; take current or default values
  -h, --help              Show this help

Environment:
  BIN_DIR                 Binary install dir
  CODEX_HOME              Codex home, default $HOME/.codex
  CODEX_BIN               Codex CLI, default codex
  KIMI_CODE_HOME          Kimi Code home, default $HOME/.kimi-code
  CLAUDE_BIN              Claude Code CLI, default claude
  CLAUDE_DIR              Claude home, default $HOME/.claude (sets CLAUDE_CONFIG_DIR)
  DISPATCH_ROOTS          Same as --roots
  SLATE_PREBUILT          Same as --prebuilt (1 = on)
  SLATE_RELEASE_REPO      Release repo (default saltyming/slate-agent-kit)
  SLATE_RELEASE_TAG       Release tag (default latest)
  PLATFORM                Override the auto-detected release platform triple
  SLATE_SETUP             Use this slate-setup binary instead of building one

Transcript forwarding (aside) reads each harness's own session log natively;
the installer only pins which harness via ASIDE_HARNESS.
USAGE
}

INSTALL=0
CONFIGURE=""
UNCONFIGURE=""
ROOTS="${DISPATCH_ROOTS:-}"
BIN_DIR="${BIN_DIR:-}"
PREBUILT="${SLATE_PREBUILT:-0}"
YES=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --install-only) INSTALL=1 ;;
    --configure-claude) INSTALL=1; CONFIGURE="$CONFIGURE claude" ;;
    --configure-codex) INSTALL=1; CONFIGURE="$CONFIGURE codex" ;;
    --configure-kimi) INSTALL=1; CONFIGURE="$CONFIGURE kimi" ;;
    --configure-all) INSTALL=1; CONFIGURE="$CONFIGURE claude codex kimi" ;;
    --uninstall-claude) UNCONFIGURE="$UNCONFIGURE claude" ;;
    --uninstall-codex) UNCONFIGURE="$UNCONFIGURE codex" ;;
    --uninstall-kimi) UNCONFIGURE="$UNCONFIGURE kimi" ;;
    --roots)
      [ "$#" -ge 2 ] || { echo "--roots requires a path list" >&2; exit 2; }
      ROOTS="$2"; shift ;;
    --bin-dir)
      [ "$#" -ge 2 ] || { echo "--bin-dir requires a path" >&2; exit 2; }
      BIN_DIR="$2"; shift ;;
    --prebuilt) PREBUILT=1 ;;
    -y|--yes) YES=1 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

# No option at all: install and configure every harness, as before.
if [ "$INSTALL" = 0 ] && [ -z "$CONFIGURE" ] && [ -z "$UNCONFIGURE" ]; then
  INSTALL=1
  CONFIGURE=" claude codex kimi"
fi

[ -z "${CLAUDE_DIR:-}" ] || : "${CLAUDE_CONFIG_DIR:=$CLAUDE_DIR}"
export CLAUDE_CONFIG_DIR="${CLAUDE_CONFIG_DIR:-}"
[ -n "$CLAUDE_CONFIG_DIR" ] || unset CLAUDE_CONFIG_DIR
if [ -n "${PLATFORM:-}" ] && [ -z "${SLATE_PLATFORM:-}" ]; then
  export SLATE_PLATFORM="$PLATFORM"
fi

if [ -n "${SLATE_SETUP:-}" ]; then
  SETUP="$SLATE_SETUP"
elif command -v cargo >/dev/null 2>&1; then
  echo "Building slate-setup..." >&2
  cargo build --release -p slate-setup --manifest-path "$ROOT/Cargo.toml" >&2
  SETUP="${CARGO_TARGET_DIR:-$ROOT/target}/release/slate-setup"
elif command -v slate-setup >/dev/null 2>&1; then
  echo "cargo not found - using slate-setup from PATH with prebuilt release binaries." >&2
  SETUP="$(command -v slate-setup)"
  PREBUILT=1
else
  echo "Error: cargo is required to build slate-setup (or put a slate-setup binary on PATH or in SLATE_SETUP)." >&2
  exit 1
fi

# Options shared by both invocations. The expansions below are deliberately
# unquoted: each expands to zero words or to a flag plus one quoted value.
YES_FLAG=""
[ "$YES" = 0 ] || YES_FLAG="--yes"

run_setup() {
  # shellcheck disable=SC2086
  "$SETUP" mcp ${BIN_DIR:+--bin-dir "$BIN_DIR"} ${YES_FLAG:+$YES_FLAG} "$@"
}

if [ -n "$UNCONFIGURE" ]; then
  set --
  for h in $UNCONFIGURE; do set -- "$@" --harness "$h"; done
  run_setup --uninstall "$@"
fi

if [ "$INSTALL" = 1 ]; then
  set --
  for h in $CONFIGURE; do set -- "$@" --harness "$h"; done
  if [ "$PREBUILT" = 1 ]; then
    set -- "$@" --binaries prebuilt
    case "${SLATE_RELEASE_TAG:-latest}" in
      latest|"") ;;
      v*) set -- "$@" --slate-version "${SLATE_RELEASE_TAG#v}" ;;
      *) set -- "$@" --slate-version "$SLATE_RELEASE_TAG" ;;
    esac
  else
    set -- "$@" --binaries build --slate-dir "$ROOT"
  fi
  [ -z "$ROOTS" ] || set -- "$@" --roots "$ROOTS"
  run_setup "$@"
fi
