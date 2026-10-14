## Antigravity models (as of 2026-10-14)

| Model (`--model`) | Tier | Use for | Start effort |
|---|---|---|---|
| `claude-opus-5-5` | default | design, architecture, review, multi-module implementation | medium; high when design-bound |
| `claude-sonnet-5-5` | workhorse | daily coding and agentic tool use; write-capable delegates | medium; high for harder or longer work |
| `gemini-3.8-flash` | fast | read-only search and summaries, scoped subtasks, extraction | medium |
| `gpt-oss-120b-medium` | open-weight | routine edits and summaries when the vendor models are unavailable | fixed |

- `agy models` lists the slugs; each Claude and Gemini model also exists as `<slug>-low`, `<slug>-medium` and `<slug>-high`, which fixes the effort in the name; `--effort` applies only to a bare slug and only where the model exposes a setting.
- The Claude tiers follow Anthropic's own placement of Opus and Sonnet; the Gemini and gpt-oss rows follow Google's listing in `agy models`, which ranks no vendor against another.
