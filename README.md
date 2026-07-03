# slate-agent-kit

`slate-agent-kit` is the meta repository for the agent-kit family. It keeps the
three harness-specific kits as submodules and owns the shared rules, workflows,
and portable MCP servers that should not drift across harnesses.

This is not a monorepo replacement for the harness kits. The harness repos stay
separate:

- `kits/claude-agent-kit`
- `kits/codex-agent-kit`
- `kits/kimi-agent-kit`

The shared source lives here:

- `shared/rules/common` - high-fidelity common rules lifted from the existing
  kits, with harness-specific tool names tokenized or isolated.
- `shared/rules/mcp` - shared `aside` and `dispatch` policy rules.
- `shared/workflows/palette` - common palette rules and palette skills.
- `shared/mcp-servers/aside` - portable read-only consultation MCP server.
- `shared/mcp-servers/dispatch` - portable external execution MCP server.
- `adapters/{claude,codex,kimi}` - render mappings from shared source to each
  harness's native filenames and tool names.

Harness-specific code stays in the harness submodules. In particular,
`workslate` remains Claude-only and does not move into `shared`.

## Structure

```text
slate-agent-kit/
  kits/
    claude-agent-kit/       # git submodule
    codex-agent-kit/        # git submodule
    kimi-agent-kit/         # git submodule

  shared/
    rules/common/           # common rule source
    rules/mcp/              # aside/dispatch policy source
    workflows/palette/      # common palette source
    mcp-servers/aside/      # common MCP
    mcp-servers/dispatch/   # common MCP

  adapters/
    claude/
    codex/
    kimi/

  docs/
    architecture.md
    support-matrix.md

  tooling/
    render-kit.sh
    validate.sh

  Cargo.toml                # shared MCP workspace
```

## Rule Extraction Standard

Common rules are not summaries. They must preserve the operational detail of
the existing kit rules and remove only harness-specific surfaces. When a rule
depends on a harness tool, use an explicit token or move that paragraph to the
harness repo.

Examples:

- Common rule: "multi-step work must be tracked before edits."
- Claude mapping: `workslate_task_*`.
- Codex mapping: plan/goal/update-plan surface.
- Kimi mapping: `TodoList`.

## Communication Requirement

All harnesses must render the formal-language rule. In Korean, the default
register is polite formal language (`합니다`, `습니다`, `드립니다`). Casual
banmal endings are not used unless the user explicitly asks for casual speech in
the current conversation.

## Rendering

Render a harness from shared source with:

```sh
tooling/render-kit.sh codex
tooling/render-kit.sh kimi
tooling/render-kit.sh claude
```

The rendered harness repos are committed independently. The shared source in
this repo remains the place to edit common behavior.

## License

[MIT](LICENSE.md) © 2026 Hamin Sung.
