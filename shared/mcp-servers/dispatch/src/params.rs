//! MCP tool parameter schemas.
//!
//! `SubmitParams` carries the structured task spec (objective / target_files /
//! constraints / acceptance) alongside free-form prose (context / details), plus
//! the execution knobs (backend / model / sandbox / …). `ConsultParams` carries
//! a consultation's question, context and transcript settings. Array, boolean
//! and integer fields use the lenient deserializers so a stringified value from
//! a calling agent is tolerated with a corrective error.

use schemars::JsonSchema;
use serde::Deserialize;

use crate::lenient::{lenient_opt_bool, lenient_opt_u32, lenient_opt_vec_string};

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct SubmitParams {
    /// Required. What the delegated step must accomplish — a clear, self-contained
    /// objective the backend agent will implement.
    pub objective: String,

    /// Required. Absolute path to the directory the backend runs in. Codex runs
    /// WRITE-CAPABLE here and may modify files. The server rejects any path that
    /// does not canonicalize within the project root (or a configured extra root).
    pub working_dir: String,

    /// Optional short label for the step (shown in lists).
    pub title: Option<String>,

    /// Optional list of files expected to change.
    #[serde(default, deserialize_with = "lenient_opt_vec_string")]
    pub target_files: Option<Vec<String>>,

    /// Optional hard do / don't rules the backend must respect.
    #[serde(default, deserialize_with = "lenient_opt_vec_string")]
    pub constraints: Option<Vec<String>>,

    /// Optional acceptance / verification criteria — how to know the step is done.
    #[serde(default, deserialize_with = "lenient_opt_vec_string")]
    pub acceptance: Option<Vec<String>>,

    /// Optional free-form background context.
    pub context: Option<String>,

    /// Optional free-form additional instructions.
    pub details: Option<String>,

    /// Backend to delegate to: "codex" (default), "opencode" or "claude".
    pub backend: Option<String>,

    /// Optional model override forwarded to the backend. OpenCode expects provider/model.
    pub model: Option<String>,

    /// Optional reasoning effort (low / medium / high / xhigh); OpenCode maps this to variant.
    pub reasoning_effort: Option<String>,

    /// Sandbox mode: "read-only" | "workspace-write" (default) | "danger-full-access"
    /// (the last is rejected unless the server enables it via DISPATCH_ALLOW_DANGER).
    pub sandbox: Option<String>,

    /// Optional grouping label so a plan's steps can be listed / cancelled as a unit.
    pub plan_id: Option<String>,

    /// Permit the backend to run when working_dir is not a git repository.
    #[serde(default, deserialize_with = "lenient_opt_bool")]
    pub skip_git_repo_check: Option<bool>,

    /// Override the one-active-dispatch-per-working_dir guard (allow a second
    /// concurrent run against the same directory). Default false.
    #[serde(default, deserialize_with = "lenient_opt_bool")]
    pub allow_concurrent: Option<bool>,

    /// Optional ordered fallback chain: if `model` (or the backend default) fails
    /// with a transient backend error (rate limit, quota exceeded, model
    /// unavailable), the server automatically retries the SAME task against the
    /// next model here, in order, until one succeeds or the chain is exhausted. A
    /// non-transient failure (bad prompt, sandbox violation, auth/permission) is
    /// never retried. Total attempts = 1 + this list's length. Not honored by
    /// dispatch_steer — a resumed session stays on one model.
    #[serde(default, deserialize_with = "lenient_opt_vec_string")]
    pub model_fallback: Option<Vec<String>>,
}

/// Parameters of `dispatch_consult`.
#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct ConsultParams {
    /// Required. The question to put to the backend.
    pub question: String,

    /// Backend to consult: "codex" (default) or "claude". "opencode" serves
    /// executions only and is refused.
    pub backend: Option<String>,

    /// Optional additional framing on top of the transcript. Use this to tell
    /// the backend something that is not in the conversation yet (e.g. "focus
    /// on correctness, not style"); prefer naming file paths here and letting
    /// the backend read them over embedding their content.
    pub context: Option<String>,

    /// Forward the current harness transcript (redacted: text passes, tool
    /// calls, tool results and thinking become placeholders) when a transcript
    /// source is available. Defaults to true; pass false for decontextualised
    /// questions. Pass raw `true` / `false`, not strings.
    #[serde(default, deserialize_with = "lenient_opt_bool")]
    pub include_transcript: Option<bool>,

    /// When transcript forwarding is on, keep only this many recent messages
    /// (default 80). The rendered transcript is further capped at 100 KB by
    /// trimming from the front. Pass a raw JSON number, not a string.
    #[serde(default, deserialize_with = "lenient_opt_u32")]
    pub transcript_tail: Option<u32>,

    /// Optional model forwarded to the backend CLI (`-m` / `--model`); omitted,
    /// the CLI uses its own default.
    pub model: Option<String>,

    /// Optional reasoning effort: codex `-c model_reasoning_effort=<val>`,
    /// claude `--effort <val>` (low / medium / high / xhigh / max where
    /// supported).
    pub reasoning_effort: Option<String>,

    /// Optional ordered fallback chain: if `model` (or the backend default)
    /// fails with a transient backend error (rate limit, quota, model
    /// unavailable), the server retries the SAME question against the next
    /// model here, in order. A non-transient failure is never retried. The
    /// reply notes when a fallback model answered.
    #[serde(default, deserialize_with = "lenient_opt_vec_string")]
    pub model_fallback: Option<Vec<String>>,

    /// Optional short label for the consultation (shown in lists).
    pub title: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct StatusParams {
    /// Task id, e.g. "d-7".
    pub id: String,
    /// Also return the accepted `spec`, the rendered `prompt`, and `argv`. Default
    /// false — status is compact by default so repeated polling of a long run does
    /// not re-echo the whole spec every call. Set true to re-read the submitted
    /// spec (e.g. for a task this session did not submit).
    #[serde(default, deserialize_with = "lenient_opt_bool")]
    pub include_spec: Option<bool>,
}

/// Parameters of `dispatch_list`.
#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct ListParams {
    /// Optional plan_id filter.
    pub plan_id: Option<String>,
    /// Optional status filter (queued / running / succeeded / failed / cancelled / interrupted).
    pub status: Option<String>,
    /// Optional kind filter: "execute" or "consult".
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct CancelParams {
    /// Cancel a single task by id.
    pub id: Option<String>,
    /// Or cancel every active task in a plan. Exactly one of id / plan_id is required.
    pub plan_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct BackendsParams {}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct LogsParams {
    /// Task id, e.g. "d-7".
    pub id: String,
    /// 1-based first rendered line to return. Omitted = tail (last lines).
    #[serde(default, deserialize_with = "lenient_opt_u32")]
    pub line_start: Option<u32>,
    /// 1-based last rendered line (inclusive). Omitted = through the end.
    #[serde(default, deserialize_with = "lenient_opt_u32")]
    pub line_end: Option<u32>,
    /// Curation categories to include: any of lifecycle / messages / tools / edits /
    /// reasoning. Default is backend-aware: codex and unknown backends use lifecycle +
    /// messages + tools + edits because codex reasoning is encrypted; opencode also
    /// includes reasoning because OpenCode exposes plaintext reasoning text.
    #[serde(default, deserialize_with = "lenient_opt_vec_string")]
    pub kinds: Option<Vec<String>>,
    /// Return raw JSONL (within the line range) instead of the curated view.
    #[serde(default, deserialize_with = "lenient_opt_bool")]
    pub raw: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct SteerParams {
    /// Task id to steer — its backend session is resumed with the new instruction.
    pub id: String,
    /// The new instruction to send to the resumed session.
    pub instruction: String,
    /// Optional model override for the follow-up run.
    pub model: Option<String>,
    /// Optional reasoning effort for the follow-up run.
    pub reasoning_effort: Option<String>,
    /// Override the one-active-dispatch-per-working_dir guard for this resumed run.
    /// Omitted → inherit the parent task's persisted allow_concurrent; set → override
    /// it (so `false` re-enforces the guard even if the parent bypassed it).
    #[serde(default, deserialize_with = "lenient_opt_bool")]
    pub allow_concurrent: Option<bool>,
}
