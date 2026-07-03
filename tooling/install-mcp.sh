#!/bin/sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
BIN_DIR="${BIN_DIR:-$HOME/.local/bin}"
CODEX_HOME="${CODEX_HOME:-$HOME/.codex}"
KIMI_CODE_HOME="${KIMI_CODE_HOME:-$HOME/.kimi-code}"
CODEX_BIN="${CODEX_BIN:-codex}"

DO_INSTALL=0
CONFIG_CODEX=0
CONFIG_KIMI=0
UNCONFIG_CODEX=0
UNCONFIG_KIMI=0

usage() {
  cat <<'USAGE'
Usage: tooling/install-mcp.sh [options]

Build and install the shared Slate MCP servers, then register them for one or
more harnesses.

Options:
  --install-only          Build/copy aside and dispatch, do not configure a harness
  --configure-codex       Build/copy and register via `codex mcp add`
  --configure-kimi        Build/copy and register a Kimi local plugin
  --configure-all         Build/copy and configure Codex + Kimi
  --uninstall-codex       Remove aside/dispatch from Codex config
  --uninstall-kimi        Remove the Kimi local plugin registration and files
  --bin-dir DIR           Install binaries into DIR (default: $HOME/.local/bin)
  -h, --help              Show this help

Environment:
  BIN_DIR                 Binary install dir
  CODEX_HOME              Codex home, default $HOME/.codex
  CODEX_BIN               Codex CLI, default codex
  KIMI_CODE_HOME          Kimi Code home, default $HOME/.kimi-code
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --install-only)
      DO_INSTALL=1
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
      CONFIG_CODEX=1
      CONFIG_KIMI=1
      ;;
    --uninstall-codex)
      UNCONFIG_CODEX=1
      ;;
    --uninstall-kimi)
      UNCONFIG_KIMI=1
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

if [ "$DO_INSTALL$CONFIG_CODEX$CONFIG_KIMI$UNCONFIG_CODEX$UNCONFIG_KIMI" = "00000" ]; then
  DO_INSTALL=1
  CONFIG_CODEX=1
  CONFIG_KIMI=1
fi

ASIDE_BIN="$BIN_DIR/aside"
DISPATCH_BIN="$BIN_DIR/dispatch"

build_and_install() {
  command -v cargo >/dev/null 2>&1 || {
    echo "Error: cargo is required to build shared MCP servers." >&2
    exit 1
  }
  echo "Building shared MCP servers..."
  cargo build --release -p aside -p dispatch --manifest-path "$ROOT/Cargo.toml"
  mkdir -p "$BIN_DIR"
  cp "$ROOT/target/release/aside" "$ASIDE_BIN"
  cp "$ROOT/target/release/dispatch" "$DISPATCH_BIN"
  chmod 755 "$ASIDE_BIN" "$DISPATCH_BIN"
  echo "Installed:"
  echo "  $ASIDE_BIN"
  echo "  $DISPATCH_BIN"
}

configure_codex() {
  command -v "$CODEX_BIN" >/dev/null 2>&1 || {
    echo "Error: $CODEX_BIN is required for --configure-codex." >&2
    exit 1
  }
  mkdir -p "$CODEX_HOME/slate-agent-kit/transcripts" "$CODEX_HOME/slate-agent-kit/projects"
  CODEX_HOME="$CODEX_HOME" "$CODEX_BIN" mcp remove aside >/dev/null 2>&1 || true
  CODEX_HOME="$CODEX_HOME" "$CODEX_BIN" mcp remove dispatch >/dev/null 2>&1 || true
  CODEX_HOME="$CODEX_HOME" "$CODEX_BIN" mcp add aside \
    --env ASIDE_TRANSCRIPT_DIR="$CODEX_HOME/slate-agent-kit/transcripts" \
    -- "$ASIDE_BIN"
  CODEX_HOME="$CODEX_HOME" "$CODEX_BIN" mcp add dispatch \
    --env SLATE_AGENT_STATE_HOME="$CODEX_HOME/slate-agent-kit" \
    -- "$DISPATCH_BIN"
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
  mkdir -p "$KIMI_CODE_HOME/plugins/managed" "$KIMI_CODE_HOME/slate-agent-kit/transcripts"
  node - "$KIMI_CODE_HOME" "$ASIDE_BIN" "$DISPATCH_BIN" <<'NODE'
const fs = require("fs");
const path = require("path");

const [home, asideBin, dispatchBin] = process.argv.slice(2);
const pluginId = "slate-agent-kit-mcp";
const pluginRoot = path.join(home, "plugins", "managed", pluginId);
const installedPath = path.join(home, "plugins", "installed.json");
const transcriptDir = path.join(home, "slate-agent-kit", "transcripts");
const stateHome = path.join(home, "slate-agent-kit");

fs.mkdirSync(pluginRoot, { recursive: true });
fs.mkdirSync(path.dirname(installedPath), { recursive: true });
fs.mkdirSync(transcriptDir, { recursive: true });
fs.mkdirSync(stateHome, { recursive: true });

const manifest = {
  name: pluginId,
  version: "0.1.0",
  description: "Shared Slate Agent Kit MCP servers for Kimi Code.",
  keywords: ["slate-agent-kit", "mcp", "aside", "dispatch"],
  mcpServers: {
    aside: {
      command: asideBin,
      args: [],
      cwd: pluginRoot,
      env: {
        ASIDE_TRANSCRIPT_DIR: transcriptDir
      }
    },
    dispatch: {
      command: dispatchBin,
      args: [],
      cwd: pluginRoot,
      env: {
        SLATE_AGENT_STATE_HOME: stateHome
      }
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
[ "$UNCONFIG_CODEX" -eq 0 ] || unconfigure_codex
[ "$UNCONFIG_KIMI" -eq 0 ] || unconfigure_kimi
[ "$CONFIG_CODEX" -eq 0 ] || configure_codex
[ "$CONFIG_KIMI" -eq 0 ] || configure_kimi
