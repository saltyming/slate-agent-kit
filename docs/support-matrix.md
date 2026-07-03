# Support Matrix

| Capability | Claude | Codex | Kimi |
|---|---|---|---|
| Common rules | yes | yes | yes |
| Formal language rule | yes | yes | yes |
| Palette | yes | yes | yes |
| Aside policy rules | yes | yes | yes |
| Aside MCP implementation | shared MCP | shared MCP | shared MCP when configured |
| Dispatch policy rules | yes | yes | yes |
| Dispatch MCP implementation | shared MCP | shared MCP | shared MCP when configured |
| Workslate | Claude-only | no | no |
| Hooks | Claude hooks | Codex command hooks | no default support |
| Task tracker | workslate | update_plan plus goal tracking | TodoList |
| Read-only delegation | Explore/Plan | explorer/custom subagent | Agent explore/plan |
| Write-capable delegation | subagent/team/dispatch | worker/custom/dispatch | coder/AgentSwarm |

Unsupported entries must remain explicit. A missing support declaration is a
bug because it hides scope decisions.
