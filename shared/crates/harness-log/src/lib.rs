//! Shared harness session-log discovery, schema knowledge and token usage.
//!
//! Multiple crates in this workspace (`aside`, `dispatch`, `agent-exec`) need
//! to find and identify harness/backend session logs — most importantly the
//! Codex rollout JSONL files under `$CODEX_HOME/sessions` and the Claude Code
//! session files under `~/.claude/projects` — and to read the token usage a
//! backend reports. The schema folklore (e.g. `session_id` vs the legacy `id`
//! field, headless-child detection via `originator`/`source`, which usage
//! field holds which count) lives here exactly once so the consuming crates
//! cannot drift apart.
//!
//! - [`codex`]: rollout location, session metadata and message text.
//! - [`claude`]: Claude Code session file location.
//! - [`usage`]: the normalized [`Usage`] buckets and one parser per source.

pub mod claude;
pub mod codex;
pub mod usage;

pub use usage::{Usage, UsageSource};
