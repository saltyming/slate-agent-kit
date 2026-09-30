<!-- slate-agent-kit:common -->
# Consultation (aside)

The `aside` MCP server asks another model family for a read-only opinion; how to call it is in its own instructions.

- Worth its cost when another family's view can change an open decision: an architecture or public-contract choice, a change to concurrency or invariants, security-sensitive code, a diagnosis the evidence does not settle. Not when the decision is made, the question is routine, or the user is reviewing with you and waiting for your own answer. Level: `{{HARNESS_RULES_DIR}}/{{ASIDE_PREFS_FILE}}` ({{PREFS_LOADING}}); at `auto`, say in one line which backend and why (§ 8).
- aside and a harness-native advisor are independent and may see different transcripts (aside's is redacted of tool calls); never run both at once, one after the other is fine.
- Backend, `model`, `reasoning_effort`, `model_fallback`: the user's current-turn choice, then the prefs, then omit. Pass the fallback chain split and trimmed; the server retries transient failures itself. Unknown CLIs: `aside_list` first.
- Report in two to four sentences: the conclusion and any concrete disagreement with your own view.

{{@INSERT aside-native-advisor}}
