## Claude Code models (as of 2026-10-08)

| Model (`model`) | Tier | Use for | Start effort |
|---|---|---|---|
| Fable 5.1 (`fable`) | frontier | autonomous sessions of hours, deep multistep research | high |
| Opus 5.5 (`opus`) | default | design, architecture, review, multi-module implementation | medium; high when design-bound |
| Sonnet 5.5 (`sonnet`) | workhorse | daily coding and agentic tool use; write-capable delegates | medium; high for harder or longer work |
| Haiku 5.5 (`haiku`) | fast | read-only search and summaries, scoped coding subtasks, extraction | medium |

- Vendor figures, Terminal-Bench 4.0: Sonnet 70.6%, Haiku 39.2% (Haiku page); Opus 66.4% at xhigh, Fable 55.8% (Opus page). The vendor places Haiku as a subagent on coding work and Sonnet or Opus for long agentic coding.
- At low effort Haiku may skip a search or a check and may stop early on a long prompt.
