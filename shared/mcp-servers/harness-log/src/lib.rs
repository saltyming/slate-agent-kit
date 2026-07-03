//! Shared harness session-log discovery and schema knowledge.
//!
//! Multiple MCP servers in this workspace (`aside`, `dispatch`) need to find
//! and identify harness/backend session logs — most importantly the Codex
//! rollout JSONL files under `$CODEX_HOME/sessions`. The schema folklore
//! (e.g. `session_id` vs the legacy `id` field, headless-child detection via
//! `originator`/`source`) lives here exactly once so the consuming crates
//! cannot drift apart.

pub mod codex;
