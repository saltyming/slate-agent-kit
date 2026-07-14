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
    codesign --force --sign - "$ib_tmp" >/dev/null 2>&1 || \
      echo "WARNING: ad-hoc codesign failed for $(basename "$ib_dest"); macOS may SIGKILL the unsigned MCP binary on launch." >&2
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
    Linux)
      # musl distros (Alpine, etc.) need the -musl asset; the glibc -gnu binary
      # fails to exec there. Detect the musl loader / ldd banner rather than
      # always assuming glibc.
      if [ -f "/lib/ld-musl-${arch}.so.1" ] \
        || { command -v ldd >/dev/null 2>&1 && ldd --version 2>&1 | grep -qi musl; }; then
        printf '%s-unknown-linux-musl' "$arch"
      else
        printf '%s-unknown-linux-gnu' "$arch"
      fi
      ;;
    MINGW*|MSYS*|CYGWIN*|Windows_NT) printf '%s-pc-windows-msvc' "$arch" ;;
    *) echo "Error: unsupported OS $(uname -s) (set PLATFORM manually)" >&2; exit 1 ;;
  esac
}

sha256_hex() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    printf ''
  fi
}

# verify_checksum <archive> <asset-filename> <have_checksums> <checksums-file>
# Hard-fails on a mismatch; warns (does not abort) when the manifest, the entry,
# or a sha256 tool is unavailable — verification is best-effort per release.
verify_checksum() {
  [ "$3" = "1" ] || return 0
  _expected=$(awk -v f="$2" '$2 == f || $2 == "*" f { print $1; exit }' "$4")
  if [ -z "$_expected" ]; then
    echo "WARNING: no checksum entry for $2; not verified." >&2
    return 0
  fi
  _actual=$(sha256_hex "$1")
  if [ -z "$_actual" ]; then
    echo "WARNING: no sha256 tool (sha256sum/shasum) found; $2 not verified." >&2
    return 0
  fi
  if [ "$_expected" != "$_actual" ]; then
    echo "Error: checksum mismatch for $2" >&2
    echo "  expected: $_expected" >&2
    echo "  actual:   $_actual" >&2
    exit 1
  fi
  echo "  verified $2 (sha256)"
}

download_prebuilt() {
  platform=$(detect_platform)
  if [ "$SLATE_RELEASE_TAG" = "latest" ]; then
    base="https://github.com/$SLATE_RELEASE_REPO/releases/latest/download"
  else
    base="https://github.com/$SLATE_RELEASE_REPO/releases/download/$SLATE_RELEASE_TAG"
  fi
  # Windows release assets are .zip (containing <name>.exe); every other target
  # ships .tar.gz.
  case "$platform" in
    *windows*) arc_ext="zip"; bin_ext=".exe" ;;
    *) arc_ext="tar.gz"; bin_ext="" ;;
  esac
  dl_dir=$(mktemp -d)
  trap 'rm -rf "$dl_dir"' EXIT HUP INT TERM
  echo "Downloading prebuilt MCP servers ($platform, $SLATE_RELEASE_TAG)..."
  # Fetch the release checksum manifest once. Present on current releases; older
  # ones lack it (warn, or hard-fail under SLATE_REQUIRE_CHECKSUM=1).
  have_checksums=0
  if fetch "$base/checksums.txt" "$dl_dir/checksums.txt" 2>/dev/null; then
    have_checksums=1
  elif [ "${SLATE_REQUIRE_CHECKSUM:-0}" = "1" ]; then
    echo "Error: no checksums.txt in this release and SLATE_REQUIRE_CHECKSUM=1." >&2
    exit 1
  else
    echo "WARNING: release has no checksums.txt; binary integrity NOT verified." >&2
  fi
  for name in aside dispatch; do
    fetch "$base/$name-$platform.$arc_ext" "$dl_dir/$name.$arc_ext"
    verify_checksum "$dl_dir/$name.$arc_ext" "$name-$platform.$arc_ext" \
      "$have_checksums" "$dl_dir/checksums.txt"
    case "$arc_ext" in
      zip)
        command -v unzip >/dev/null 2>&1 || {
          echo "Error: unzip is required to extract Windows release assets" >&2
          exit 1
        }
        unzip -oq "$dl_dir/$name.$arc_ext" -d "$dl_dir"
        ;;
      *) tar xzf "$dl_dir/$name.$arc_ext" -C "$dl_dir" ;;
    esac
  done
  mkdir -p "$BIN_DIR"
  install_binary "$dl_dir/aside$bin_ext" "$ASIDE_BIN$bin_ext"
  install_binary "$dl_dir/dispatch$bin_ext" "$DISPATCH_BIN$bin_ext"
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

# codex_set_server_key <server> <key> <toml-value> — set (or, with an empty
# <toml-value>, remove) a direct key under [mcp_servers.<server>] in the Codex
# config. Codex exposes no `codex mcp add` flags for these (config.toml-only), so
# we edit the block codex just wrote. Inserted as a direct key right after the
# table header, ahead of the [.env] sub-table; re-running is idempotent.
# <toml-value> is written verbatim, so string values must carry their own quotes.
codex_set_server_key() {
  cssk_cfg="$CODEX_HOME/config.toml"
  [ -f "$cssk_cfg" ] || return 0
  awk -v server="mcp_servers.$1" -v key="$2" -v val="$3" '
    BEGIN { tgt = "[" server "]"; pat = "^[ \t]*" key "[ \t]*="; intgt = 0; done = 0 }
    {
      if ($0 ~ /^\[/) {
        intgt = ($0 == tgt)
        print
        if (intgt && !done && val != "") { print key " = " val; done = 1 }
        next
      }
      if (intgt && $0 ~ pat) next
      print
    }
  ' "$cssk_cfg" > "$cssk_cfg.slate-tmp" && mv "$cssk_cfg.slate-tmp" "$cssk_cfg"
}

# codex_set_code_mode_ns <add|remove> <namespace> — list (or unlist) an MCP tool
# namespace in [features.code_mode] so Codex hands its tools to the model as
# plain blocking function calls instead of routing them through a code-mode cell.
#
# Since Codex 0.144 the model reaches MCP tools from inside a code-mode `exec`
# cell, which yields after yield_time_ms (10s by default) and leaves the model to
# poll with `wait`. An aside call blocks for the whole backend run — minutes with
# a reasoning-heavy model — so the model sees "still running" over and over,
# eventually terminates the cell and reports the backend as unresponsive, losing
# an answer that was about to arrive. Outside code mode the call simply blocks
# until it returns (bounded by tool_timeout_sec), so there is no exit to take.
#
# Both keys are required — setting only one of them does not leave a usable aside
# surface: excluded_tool_namespaces on its own drops the tools from the model's
# view entirely. The namespace is the MCP tool prefix Codex derives from the
# server name (`mcp__aside`), not the server name itself. Namespaces already
# listed under either key are preserved.
codex_set_code_mode_ns() {
  cscm_cfg="$CODEX_HOME/config.toml"
  [ -f "$cscm_cfg" ] || return 0
  if awk -v mode="$1" -v ns="$2" -v cfg="$cscm_cfg" '
    function trim(s) { sub(/^[ \t]+/, "", s); sub(/[ \t]+$/, "", s); return s }
    function parse(line, key, arr,   inner, t, i, cnt, v) {
      if (line !~ /\]/) {
        printf("install-mcp.sh: %s in [features.code_mode] spans several lines in %s;\n", key, cfg) > "/dev/stderr"
        printf("  refusing to rewrite it. Collapse it onto one line and re-run.\n") > "/dev/stderr"
        failed = 1
        exit 2
      }
      inner = line
      sub(/^[^[]*\[/, "", inner)
      sub(/\].*$/, "", inner)
      cnt = split(inner, t, ",")
      arr[0] = 0
      for (i = 1; i <= cnt; i++) {
        v = trim(t[i])
        gsub(/^"|"$/, "", v)
        if (v != "") { arr[0]++; arr[arr[0]] = v }
      }
    }
    function has(arr, v,   i) { for (i = 1; i <= arr[0]; i++) if (arr[i] == v) return 1; return 0 }
    function put(arr, v) { if (!has(arr, v)) { arr[0]++; arr[arr[0]] = v } }
    function drop(arr, v,   i, m, keep) {
      m = 0
      for (i = 1; i <= arr[0]; i++) if (arr[i] != v) keep[++m] = arr[i]
      for (i = 1; i <= arr[0]; i++) delete arr[i]
      for (i = 1; i <= m; i++) arr[i] = keep[i]
      arr[0] = m
    }
    function render(key, arr,   i, s) {
      s = ""
      for (i = 1; i <= arr[0]; i++) s = s (i > 1 ? ", " : "") "\"" arr[i] "\""
      return key " = [" s "]"
    }
    function dump(   i, keep) {
      if (add) { put(X, ns); put(D, ns) } else { drop(X, ns); drop(D, ns) }
      keep = (X[0] > 0 || D[0] > 0)
      for (i = 1; i <= bn; i++) if (trim(body[i]) != "") keep = 1
      if (!keep) return
      print target
      if (X[0] > 0) print render(KX, X)
      if (D[0] > 0) print render(KD, D)
      for (i = 1; i <= bn; i++) print body[i]
    }
    BEGIN {
      add = (mode == "add")
      target = "[features.code_mode]"
      KX = "excluded_tool_namespaces"
      KD = "direct_only_tool_namespaces"
      X[0] = 0; D[0] = 0
      in_cm = 0; in_feat = 0; saw = 0; bn = 0; failed = 0
    }
    /^[ \t]*\[/ {
      hdr = trim($0)
      if (in_cm) { dump(); in_cm = 0; bn = 0 }
      in_feat = 0
      if (hdr == target) { saw = 1; in_cm = 1; next }
      if (hdr == "[features]") in_feat = 1
      print
      next
    }
    {
      # A scalar features.code_mode would make the [features.code_mode] table a
      # TOML duplicate key and break every Codex session; never silently drop it.
      if (in_feat && $0 ~ /^[ \t]*code_mode[ \t]*=/) {
        printf("install-mcp.sh: [features] in %s sets a scalar `code_mode` key, which collides\n", cfg) > "/dev/stderr"
        printf("  with the [features.code_mode] table aside needs. Remove that line and re-run.\n") > "/dev/stderr"
        failed = 1
        exit 2
      }
      if (in_cm) {
        if ($0 ~ ("^[ \t]*" KX "[ \t]*=")) { parse($0, KX, X); next }
        if ($0 ~ ("^[ \t]*" KD "[ \t]*=")) { parse($0, KD, D); next }
        body[++bn] = $0
        next
      }
      print
    }
    END {
      if (failed) exit 2
      if (in_cm) { dump() }
      else if (add && !saw) {
        put(X, ns); put(D, ns)
        print ""
        print target
        print render(KX, X)
        print render(KD, D)
      }
    }
  ' "$cscm_cfg" > "$cscm_cfg.slate-tmp"; then
    mv "$cscm_cfg.slate-tmp" "$cscm_cfg"
  else
    rm -f "$cscm_cfg.slate-tmp"
    echo "install-mcp.sh: aborted; $cscm_cfg left unchanged." >&2
    exit 1
  fi
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
  # aside blocks for the whole backend call (a claude -p / codex advisor run can
  # take many minutes), which would otherwise trip Codex's per-tool-call timeout.
  # Raise it to 30 minutes for aside. (dispatch tool calls are bounded — submit
  # returns immediately and dispatch_wait is capped — so it keeps the default.)
  codex_set_server_key aside tool_timeout_sec 1800
  # Codex classes an MCP tool as approval-required unless told otherwise, and a
  # headless `codex exec` (approval_policy=never) has nobody to approve it — the
  # call comes back as `user cancelled MCP tool call` before the backend is even
  # spawned, which is why a dispatch-spawned Codex backend cannot consult aside.
  # Every aside tool is a read-only advisor, so pre-approving them is honest.
  # (dispatch is write-capable and deliberately keeps the default approval path.)
  codex_set_server_key aside default_tools_approval_mode '"approve"'
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
  # Last, so that a config this cannot safely rewrite still leaves both servers
  # registered — the user fixes the conflict it names and re-runs.
  codex_set_code_mode_ns add mcp__aside
  echo "Configured Codex MCP servers in $CODEX_HOME/config.toml"
}

unconfigure_codex() {
  command -v "$CODEX_BIN" >/dev/null 2>&1 || {
    echo "Error: $CODEX_BIN is required for --uninstall-codex." >&2
    exit 1
  }
  CODEX_HOME="$CODEX_HOME" "$CODEX_BIN" mcp remove aside >/dev/null 2>&1 || true
  CODEX_HOME="$CODEX_HOME" "$CODEX_BIN" mcp remove dispatch >/dev/null 2>&1 || true
  # `codex mcp remove` deletes the whole [mcp_servers.aside] block (its direct keys
  # with it); strip explicitly too so no orphaned override can linger. The code-mode
  # exclusion lives outside that block, so it has to be unlisted on its own.
  codex_set_server_key aside tool_timeout_sec ""
  codex_set_server_key aside default_tools_approval_mode ""
  codex_set_code_mode_ns remove mcp__aside
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
  node "$ROOT/tooling/kit-scripts/write-kimi-plugin.js" \
    "$KIMI_CODE_HOME" "$ASIDE_BIN" "$DISPATCH_BIN" "$HOME/.kimi-code" "$ROOTS"
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
