# Support Matrix

| Capability | Claude | Codex | Kimi |
|---|---|---|---|
| Common rules | yes | yes | yes |
| Formal language rule | yes | yes | yes |
| Palette | yes | yes | yes |
| Aside policy rules | yes | yes | yes |
| Aside MCP implementation | shared MCP (native transcripts) | shared MCP (native rollout transcripts) | shared MCP plugin (native wire transcripts) |
| Dispatch policy rules | yes | yes | yes |
| Dispatch MCP implementation | shared MCP | shared MCP | shared MCP plugin (`--roots` required) |
| Dispatch backends | codex / opencode / claude | codex / opencode / claude | codex / opencode / claude |
| Harness surface rule | (inserts into shared files) | codex-surface | kimi-surface |
| Prefs files (aside/dispatch) | generated (configure-prefs.sh) | generated (configure-prefs.sh) | generated (configure-prefs.sh) |
| Prefs file (git) | installed `unset`; the agent asks and records | installed `unset`; the agent asks and records | installed `unset`; the agent asks and records |
| Rule delivery | CLAUDE.md + rules dir (auto-loaded) | single concatenated AGENTS.md | single concatenated AGENTS.md |
| Hooks | Claude hooks | Codex command hooks | no default support |
| Task tracker rule | none (removed in 12.0.0 / 0.7.0) | none | none |
| Read-only delegation | Explore/Plan | explorer/custom subagent | Agent explore/plan |
| Write-capable delegation | subagent/Workflow/dispatch | worker/custom/dispatch | coder/AgentSwarm |

Unsupported entries must remain explicit. A missing support declaration is a
bug because it hides scope decisions.
