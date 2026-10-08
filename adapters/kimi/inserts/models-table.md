## Kimi Code models (as of 2026-10-08)

| Model | Tier | Use for | Start effort |
|---|---|---|---|
| `k3` | frontier, 1M | hard problems: complex reasoning, algorithm design, deep debugging | high |
| `k3-256k` | frontier, 256K | as `k3` when 256K is enough, at half the quota | high |
| `kimi-for-coding` (K2.8 Preview) | workhorse | most feature work and code changes; routine edits, explanations, summaries | high |
| `kimi-for-coding-highspeed` (K2.7 Code) | fast | the workhorse's work when latency matters | thinking on |

- Highspeed costs about 3x the quota: choose it for speed, never to save cost.
- A subagent names a model by its alias in the `[secondary_model]` pool (for example `kimi-code/k3`); a model outside the pool cannot be named, and without a pool a subagent runs on the session's model.
- Thinking off serves `k3` and K2.8 requests from K2.8 Preview without thinking. Switching model or effort mid-session drops the prompt cache.
