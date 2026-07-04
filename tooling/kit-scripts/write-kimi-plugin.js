#!/usr/bin/env node
// write-kimi-plugin.js — register the shared Slate Agent Kit MCP servers
// (aside, dispatch) as a local Kimi Code plugin.
//
// Single source shared by two callers, so the plugin format can never drift
// between them:
//   - POSIX:   tooling/install-mcp.sh  (configure_kimi)
//   - Windows: the kimi kit's install.ps1 (fetched into scripts/ by render-kit.sh)
//
// Usage: node write-kimi-plugin.js <home> <asideBin> <dispatchBin> <defaultHome> [roots]
//   roots is last and optional: PowerShell may drop a trailing empty-string
//   argument, and keeping the always-present defaultHome ahead of it means a
//   dropped roots is harmless (dispatch simply gets no extra workspace roots).

const fs = require("fs");
const path = require("path");

const [home, asideBin, dispatchBin, defaultHome, roots] = process.argv.slice(2);
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

// Atomic write helper: write a sibling temp file then rename, so a crash or
// interrupt mid-write can never leave a half-written (corrupt) file behind.
function writeFileAtomic(dest, contents) {
  const tmp = `${dest}.tmp-${process.pid}`;
  fs.writeFileSync(tmp, contents);
  fs.renameSync(tmp, dest);
}

writeFileAtomic(
  path.join(pluginRoot, "kimi.plugin.json"),
  `${JSON.stringify(manifest, null, 2)}\n`
);

writeFileAtomic(
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
    "- `mcp__plugin-slate-agent-kit-mcp_aside__aside_copilot`",
    "- `mcp__plugin-slate-agent-kit-mcp_aside__aside_claude`",
    "- `mcp__plugin-slate-agent-kit-mcp_dispatch__dispatch_submit`",
    "- `mcp__plugin-slate-agent-kit-mcp_dispatch__dispatch_status`",
    ""
  ].join("\n")
);

// Update the plugin registry. If installed.json exists but does not parse, do
// NOT silently reset it to empty — that would drop every OTHER installed plugin.
// Back up the unreadable file first so nothing is lost, then start fresh.
let registry = { version: 1, plugins: [] };
if (fs.existsSync(installedPath)) {
  const raw = fs.readFileSync(installedPath, "utf8");
  try {
    registry = JSON.parse(raw);
  } catch {
    const bak = `${installedPath}.corrupt-${Date.now()}`;
    fs.writeFileSync(bak, raw);
    console.error(
      `warning: ${installedPath} was unreadable; backed up to ${bak} and rebuilt (other plugin entries could not be preserved).`
    );
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

writeFileAtomic(installedPath, `${JSON.stringify(registry, null, 2)}\n`);
