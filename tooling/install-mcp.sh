#!/bin/sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
BIN_DIR="${BIN_DIR:-$HOME/.local/bin}"
CODEX_HOME="${CODEX_HOME:-$HOME/.codex}"
KIMI_CODE_HOME="${KIMI_CODE_HOME:-$HOME/.kimi-code}"
CLAUDE_DIR="${CLAUDE_DIR:-$HOME/.claude}"
CODEX_BIN="${CODEX_BIN:-codex}"
CLAUDE_BIN="${CLAUDE_BIN:-claude}"
ROOTS="${DISPATCH_ROOTS:-}"
PREBUILT="${SLATE_PREBUILT:-0}"
SLATE_RELEASE_REPO="${SLATE_RELEASE_REPO:-saltyming/slate-agent-kit}"
SLATE_RELEASE_TAG="${SLATE_RELEASE_TAG:-latest}"

DO_INSTALL=0
CONFIG_CODEX=0
CONFIG_KIMI=0
CONFIG_CLAUDE=0
UNCONFIG_CODEX=0
UNCONFIG_KIMI=0
UNCONFIG_CLAUDE=0

usage() {
  cat <<'USAGE'
Usage: tooling/install-mcp.sh [options]

Build and install the shared Slate MCP servers (aside, dispatch), then register
them for one or more harnesses.

Options:
  --install-only          Build/copy aside and dispatch, do not configure a harness
  --configure-claude      Build/copy and register via `claude mcp add -s user`
  --configure-codex       Build/copy and register via `codex mcp add`
  --configure-kimi        Build/copy and register a Kimi local plugin
  --configure-all         Build/copy and configure Claude + Codex + Kimi
  --uninstall-claude      Remove aside/dispatch from Claude user-scope MCP config
  --uninstall-codex       Remove aside/dispatch from Codex config
  --uninstall-kimi        Remove the Kimi local plugin registration and files
  --roots DIRS            Colon-separated absolute workspace roots for dispatch
                          containment (DISPATCH_EXTRA_ROOTS). Required for Kimi
                          (its plugin runtime spawns MCP servers outside any
                          project, so dispatch has no project root there) and
                          recommended for Codex.
  --bin-dir DIR           Install binaries into DIR (default: $HOME/.local/bin)
  --prebuilt              Download prebuilt aside/dispatch from GitHub Releases
                          instead of building with cargo. Also used automatically
                          when cargo is unavailable. Note: "latest" release may be
                          newer than this checkout; pin with SLATE_RELEASE_TAG.
  -h, --help              Show this help

Environment:
  BIN_DIR                 Binary install dir
  CODEX_HOME              Codex home, default $HOME/.codex
  CODEX_BIN               Codex CLI, default codex
  KIMI_CODE_HOME          Kimi Code home, default $HOME/.kimi-code
  CLAUDE_BIN              Claude Code CLI, default claude
  CLAUDE_DIR              Claude home (dispatch state anchor), default $HOME/.claude
  DISPATCH_ROOTS          Same as --roots
  SLATE_PREBUILT          Same as --prebuilt (1 = on)
  SLATE_RELEASE_REPO      Release repo (default saltyming/slate-agent-kit)
  SLATE_RELEASE_TAG       Release tag (default latest)
  PLATFORM                Override the auto-detected release platform triple

Transcript forwarding (aside) reads each harness's own session log natively;
the installer only pins which harness via ASIDE_HARNESS.
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --install-only)
      DO_INSTALL=1
      ;;
    --configure-claude)
      DO_INSTALL=1
      CONFIG_CLAUDE=1
      ;;
    --configure-codex)
      DO_INSTALL=1
      CONFIG_CODEX=1
      ;;
    --configure-kimi)
      DO_INSTALL=1
      CONFIG_KIMI=1
      ;;
    --configure-all)
      DO_INSTALL=1
      CONFIG_CLAUDE=1
      CONFIG_CODEX=1
      CONFIG_KIMI=1
      ;;
    --uninstall-claude)
      UNCONFIG_CLAUDE=1
      ;;
    --uninstall-codex)
      UNCONFIG_CODEX=1
      ;;
    --uninstall-kimi)
      UNCONFIG_KIMI=1
      ;;
    --roots)
      [ "$#" -ge 2 ] || { echo "--roots requires a colon-separated path list" >&2; exit 2; }
      ROOTS="$2"
      shift
      ;;
    --prebuilt)
      PREBUILT=1
      ;;
    --bin-dir)
      [ "$#" -ge 2 ] || { echo "--bin-dir requires a path" >&2; exit 2; }
      BIN_DIR="$2"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

if [ "$DO_INSTALL$CONFIG_CLAUDE$CONFIG_CODEX$CONFIG_KIMI$UNCONFIG_CLAUDE$UNCONFIG_CODEX$UNCONFIG_KIMI" = "0000000" ]; then
  DO_INSTALL=1
  CONFIG_CLAUDE=1
  CONFIG_CODEX=1
  CONFIG_KIMI=1
fi

ASIDE_BIN="$BIN_DIR/aside"
DISPATCH_BIN="$BIN_DIR/dispatch"

# Copy a freshly built binary into place atomically (temp file + rename) so a
# live MCP server keeps its old inode instead of being clobbered mid-run, and
# ad-hoc codesign it on macOS (an in-place overwrite invalidates the signature
# and macOS may SIGKILL the running process).
install_binary() {
  # POSIX sh has no local variables — prefix these so they cannot shadow a
  # caller's state (download_prebuilt's $dl_dir survives across our calls).
  ib_src="$1"
  ib_dest="$2"
  ib_tmp="$ib_dest.tmp.$$"
  cp "$ib_src" "$ib_tmp"
  chmod 755 "$ib_tmp"
  if [ "$(uname -s)" = "Darwin" ] && command -v codesign >/dev/null 2>&1; then
    codesign --force --sign - "$ib_tmp" >/dev/null 2>&1 || true
  fi
  mv -f "$ib_tmp" "$ib_dest"
}

fetch() {
  url="$1"
  dest="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$dest"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$dest" "$url"
  else
    echo "Error: curl or wget required to download prebuilt binaries." >&2
    exit 1
  fi
}

detect_platform() {
  if [ -n "${PLATFORM:-}" ]; then
    printf '%s' "$PLATFORM"
    return 0
  fi
  arch=$(uname -m)
  case "$arch" in
    arm64|aarch64) arch=aarch64 ;;
    x86_64|amd64) arch=x86_64 ;;
    *) echo "Error: unsupported architecture $arch (set PLATFORM manually)" >&2; exit 1 ;;
  esac
  case "$(uname -s)" in
    Darwin) printf '%s-apple-darwin' "$arch" ;;
    Linux) printf '%s-unknown-linux-gnu' "$arch" ;;
    *) echo "Error: unsupported OS $(uname -s) (set PLATFORM manually)" >&2; exit 1 ;;
  esac
}

download_prebuilt() {
  platform=$(detect_platform)
  if [ "$SLATE_RELEASE_TAG" = "latest" ]; then
    base="https://github.com/$SLATE_RELEASE_REPO/releases/latest/download"
  else
    base="https://github.com/$SLATE_RELEASE_REPO/releases/download/$SLATE_RELEASE_TAG"
  fi
  dl_dir=$(mktemp -d)
  trap 'rm -rf "$dl_dir"' EXIT HUP INT TERM
  echo "Downloading prebuilt MCP servers ($platform, $SLATE_RELEASE_TAG)..."
  for name in aside dispatch; do
    fetch "$base/$name-$platform.tar.gz" "$dl_dir/$name.tar.gz"
    tar xzf "$dl_dir/$name.tar.gz" -C "$dl_dir"
  done
  mkdir -p "$BIN_DIR"
  install_binary "$dl_dir/aside" "$ASIDE_BIN"
  install_binary "$dl_dir/dispatch" "$DISPATCH_BIN"
}

build_and_install() {
  if [ "$PREBUILT" != "1" ] && ! command -v cargo >/dev/null 2>&1; then
    echo "cargo not found — falling back to prebuilt release binaries." >&2
    PREBUILT=1
  fi
  if [ "$PREBUILT" = "1" ]; then
    download_prebuilt
  else
    echo "Building shared MCP servers..."
    cargo build --release -p aside -p dispatch --manifest-path "$ROOT/Cargo.toml"
    mkdir -p "$BIN_DIR"
    install_binary "$ROOT/target/release/aside" "$ASIDE_BIN"
    install_binary "$ROOT/target/release/dispatch" "$DISPATCH_BIN"
  fi
  echo "Installed:"
  echo "  $ASIDE_BIN"
  echo "  $DISPATCH_BIN"
}

configure_claude() {
  command -v "$CLAUDE_BIN" >/dev/null 2>&1 || {
    echo "Error: $CLAUDE_BIN is required for --configure-claude." >&2
    exit 1
  }
  "$CLAUDE_BIN" mcp remove aside -s user >/dev/null 2>&1 || true
  "$CLAUDE_BIN" mcp remove dispatch -s user >/dev/null 2>&1 || true
  "$CLAUDE_BIN" mcp add aside -s user --transport stdio \
    -e ASIDE_HARNESS=claude \
    -- "$ASIDE_BIN"
  # SLATE_AGENT_STATE_HOME=$CLAUDE_DIR keeps dispatch state at the
  # pre-consolidation path (~/.claude/projects/<dashed>/dispatch), so existing
  # per-project task history is not orphaned.
  if [ -n "$ROOTS" ]; then
    "$CLAUDE_BIN" mcp add dispatch -s user --transport stdio \
      -e SLATE_AGENT_STATE_HOME="$CLAUDE_DIR" \
      -e DISPATCH_EXTRA_ROOTS="$ROOTS" \
      -- "$DISPATCH_BIN"
  else
    "$CLAUDE_BIN" mcp add dispatch -s user --transport stdio \
      -e SLATE_AGENT_STATE_HOME="$CLAUDE_DIR" \
      -- "$DISPATCH_BIN"
  fi
  echo "Configured Claude user-scope MCP servers (aside, dispatch)"
}

unconfigure_claude() {
  command -v "$CLAUDE_BIN" >/dev/null 2>&1 || {
    echo "Error: $CLAUDE_BIN is required for --uninstall-claude." >&2
    exit 1
  }
  "$CLAUDE_BIN" mcp remove aside -s user >/dev/null 2>&1 || true
  "$CLAUDE_BIN" mcp remove dispatch -s user >/dev/null 2>&1 || true
  echo "Removed Claude user-scope MCP registrations (aside, dispatch)"
}

configure_codex() {
  command -v "$CODEX_BIN" >/dev/null 2>&1 || {
    echo "Error: $CODEX_BIN is required for --configure-codex." >&2
    exit 1
  }
  mkdir -p "$CODEX_HOME/slate-agent-kit"
  CODEX_HOME="$CODEX_HOME" "$CODEX_BIN" mcp remove aside >/dev/null 2>&1 || true
  CODEX_HOME="$CODEX_HOME" "$CODEX_BIN" mcp remove dispatch >/dev/null 2>&1 || true
  CODEX_HOME="$CODEX_HOME" "$CODEX_BIN" mcp add aside \
    --env ASIDE_HARNESS=codex \
    -- "$ASIDE_BIN"
  if [ -n "$ROOTS" ]; then
    CODEX_HOME="$CODEX_HOME" "$CODEX_BIN" mcp add dispatch \
      --env SLATE_AGENT_STATE_HOME="$CODEX_HOME/slate-agent-kit" \
      --env DISPATCH_EXTRA_ROOTS="$ROOTS" \
      -- "$DISPATCH_BIN"
  else
    CODEX_HOME="$CODEX_HOME" "$CODEX_BIN" mcp add dispatch \
      --env SLATE_AGENT_STATE_HOME="$CODEX_HOME/slate-agent-kit" \
      -- "$DISPATCH_BIN"
    echo "Note: no --roots given. If Codex spawns MCP servers outside your project," >&2
    echo "dispatch_submit will reject working_dirs (no_project_root) until you re-run" >&2
    echo "with --roots <workspace-root>." >&2
  fi
  echo "Configured Codex MCP servers in $CODEX_HOME/config.toml"
}

unconfigure_codex() {
  command -v "$CODEX_BIN" >/dev/null 2>&1 || {
    echo "Error: $CODEX_BIN is required for --uninstall-codex." >&2
    exit 1
  }
  CODEX_HOME="$CODEX_HOME" "$CODEX_BIN" mcp remove aside >/dev/null 2>&1 || true
  CODEX_HOME="$CODEX_HOME" "$CODEX_BIN" mcp remove dispatch >/dev/null 2>&1 || true
  echo "Removed Codex MCP registrations from $CODEX_HOME/config.toml"
}

configure_kimi() {
  command -v node >/dev/null 2>&1 || {
    echo "Error: node is required to write Kimi plugin metadata." >&2
    exit 1
  }
  if [ -z "$ROOTS" ]; then
    echo "WARNING: --configure-kimi without --roots. The Kimi plugin runtime spawns" >&2
    echo "MCP servers in the plugin directory (not your project), so dispatch has no" >&2
    echo "project root there and will reject every dispatch_submit (no_project_root)" >&2
    echo "until you re-run this script with --roots <workspace-root>." >&2
  fi
  mkdir -p "$KIMI_CODE_HOME/plugins/managed" "$KIMI_CODE_HOME/slate-agent-kit"
  node - "$KIMI_CODE_HOME" "$ASIDE_BIN" "$DISPATCH_BIN" "$ROOTS" "$HOME/.kimi-code" <<'NODE'
const fs = require("fs");
const path = require("path");

const [home, asideBin, dispatchBin, roots, defaultHome] = process.argv.slice(2);
const pluginId = "slate-agent-kit-mcp";
const pluginRoot = path.join(home, "plugins", "managed", pluginId);
const installedPath = path.join(home, "plugins", "installed.json");
const stateHome = path.join(home, "slate-agent-kit");

fs.mkdirSync(pluginRoot, { recursive: true });
fs.mkdirSync(path.dirname(installedPath), { recursive: true });
fs.mkdirSync(stateHome, { recursive: true });

// aside reads Kimi session logs natively via $KIMI_CODE_HOME; only pass it
// through when this install targets a non-default home.
const asideEnv = { ASIDE_HARNESS: "kimi" };
if (path.resolve(home) !== path.resolve(defaultHome)) {
  asideEnv.KIMI_CODE_HOME = home;
}
const dispatchEnv = { SLATE_AGENT_STATE_HOME: stateHome };
if (roots) {
  dispatchEnv.DISPATCH_EXTRA_ROOTS = roots;
}
if (path.resolve(home) !== path.resolve(defaultHome)) {
  dispatchEnv.KIMI_CODE_HOME = home;
}

const manifest = {
  name: pluginId,
  version: "0.2.0",
  description: "Shared Slate Agent Kit MCP servers for Kimi Code.",
  keywords: ["slate-agent-kit", "mcp", "aside", "dispatch"],
  mcpServers: {
    aside: {
      command: asideBin,
      args: [],
      cwd: pluginRoot,
      env: asideEnv
    },
    dispatch: {
      command: dispatchBin,
      args: [],
      cwd: pluginRoot,
      env: dispatchEnv
    }
  },
  interface: {
    displayName: "Slate Agent Kit MCP",
    shortDescription: "aside read-only consultation and dispatch execution delegation.",
    developerName: "Slate Agent Kit"
  }
};

fs.writeFileSync(
  path.join(pluginRoot, "kimi.plugin.json"),
  `${JSON.stringify(manifest, null, 2)}\n`
);

fs.writeFileSync(
  path.join(pluginRoot, "SKILL.md"),
  [
    "# Slate Agent Kit MCP",
    "",
    "This local plugin exposes the shared Slate Agent Kit MCP servers to Kimi Code.",
    "",
    "- aside tools are read-only consultation tools.",
    "- dispatch tools are write-capable execution delegation tools and must follow the dispatch approval gate.",
    "",
    "Expected MCP tool prefixes are harness-generated from this plugin id and server name, for example:",
    "",
    "- `mcp__plugin-slate-agent-kit-mcp_aside__aside_list`",
    "- `mcp__plugin-slate-agent-kit-mcp_aside__aside_codex`",
    "- `mcp__plugin-slate-agent-kit-mcp_dispatch__dispatch_submit`",
    "- `mcp__plugin-slate-agent-kit-mcp_dispatch__dispatch_status`",
    ""
  ].join("\n")
);

let registry = { version: 1, plugins: [] };
if (fs.existsSync(installedPath)) {
  try {
    registry = JSON.parse(fs.readFileSync(installedPath, "utf8"));
  } catch {
    registry = { version: 1, plugins: [] };
  }
}
if (!Array.isArray(registry.plugins)) {
  registry.plugins = [];
}

const now = new Date().toISOString();
const existing = registry.plugins.find((p) => p && p.id === pluginId);
registry.plugins = registry.plugins.filter((p) => p && p.id !== pluginId);
registry.plugins.push({
  id: pluginId,
  root: pluginRoot,
  source: "local",
  enabled: true,
  installedAt: existing && existing.installedAt ? existing.installedAt : now,
  updatedAt: now,
  originalSource: "local:slate-agent-kit"
});

fs.writeFileSync(installedPath, `${JSON.stringify(registry, null, 2)}\n`);
NODE
  echo "Configured Kimi MCP plugin in $KIMI_CODE_HOME/plugins/managed/slate-agent-kit-mcp"
}

unconfigure_kimi() {
  command -v node >/dev/null 2>&1 || {
    echo "Error: node is required to update Kimi plugin metadata." >&2
    exit 1
  }
  node - "$KIMI_CODE_HOME" <<'NODE'
const fs = require("fs");
const path = require("path");

const [home] = process.argv.slice(2);
const pluginId = "slate-agent-kit-mcp";
const pluginRoot = path.join(home, "plugins", "managed", pluginId);
const installedPath = path.join(home, "plugins", "installed.json");

if (fs.existsSync(installedPath)) {
  let registry = { version: 1, plugins: [] };
  try {
    registry = JSON.parse(fs.readFileSync(installedPath, "utf8"));
  } catch {
    registry = { version: 1, plugins: [] };
  }
  if (Array.isArray(registry.plugins)) {
    registry.plugins = registry.plugins.filter((p) => p && p.id !== pluginId);
  }
  fs.writeFileSync(installedPath, `${JSON.stringify(registry, null, 2)}\n`);
}

fs.rmSync(pluginRoot, { recursive: true, force: true });
NODE
  echo "Removed Kimi MCP plugin registration from $KIMI_CODE_HOME"
}

[ "$DO_INSTALL" -eq 0 ] || build_and_install
[ "$UNCONFIG_CLAUDE" -eq 0 ] || unconfigure_claude
[ "$UNCONFIG_CODEX" -eq 0 ] || unconfigure_codex
[ "$UNCONFIG_KIMI" -eq 0 ] || unconfigure_kimi
[ "$CONFIG_CLAUDE" -eq 0 ] || configure_claude
[ "$CONFIG_CODEX" -eq 0 ] || configure_codex
[ "$CONFIG_KIMI" -eq 0 ] || configure_kimi
