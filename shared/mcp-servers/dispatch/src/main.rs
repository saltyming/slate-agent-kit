//! dispatch — delegation and consultation MCP server.
//!
//! dispatch runs one backend CLI per run, in two kinds. An *execution*
//! (`dispatch_submit`) runs a coding-agent backend as a headless, WRITE-CAPABLE
//! subprocess that modifies files in a target directory, so a planning agent
//! can offload individual plan steps and keep working. A *consultation*
//! (`dispatch_consult`) asks another model family a read-only question with the
//! harness's redacted transcript (`consult.rs`, `transcript/`). MCP calls are
//! request/response but a run takes minutes, so every run is asynchronous:
//! the tool returns an id and a `wait` command line immediately, the run
//! continues detached, and `dispatch wait <id>` (`wait.rs`, the binary's CLI
//! mode) blocks another process until the run is terminal.
//!
//! Safety is split: the server enforces mechanical invariants the model cannot
//! talk its way past (working_dir containment, sandbox ceiling, one active
//! execution per dir, re-entry per kind); the *when to ask the user* policy
//! lives in the harness-rendered dispatch and consultation rules and their
//! preferences files.

mod agy;
mod backend;
mod consult;
mod executor;
mod handshake;
mod lenient;
mod liveness;
mod opencode;
mod params;
mod render;
mod rollout;
mod store;
mod transcript;
mod wait;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime};

use rmcp::{
    RoleServer, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, tool::ToolCallContext, wrapper::Parameters},
    model::{
        CallToolRequestParams, CallToolResult, Content, ListToolsResult, PaginatedRequestParams,
        ServerCapabilities, ServerInfo, Tool,
    },
    service::RequestContext,
    tool, tool_router,
};
use serde_json::{Value, json};

use backend::Backend;
use liveness::process_alive;
use params::{
    BackendsParams, CancelParams, ConsultParams, ListParams, LogsParams, StatusParams, SteerParams,
    SubmitParams,
};
use store::Kind;

/// How long `dispatch_steer` waits for the parent run to actually terminate after
/// cancel before giving up (so the working_dir is free for the resume run).
const STEER_TERMINATE_WAIT_MS: u64 = 30_000;

/// Tolerance subtracted from a legacy task's start time when validating its rollout by
/// mtime — absorbs clock / ordering skew between the DB timestamp and the file.
const FLOOR_TOLERANCE_SECS: u64 = 60;

// ── server ────────────────────────────────────────────────

#[derive(Clone)]
struct Dispatch {
    db: executor::DbHandle,
    registry: executor::Registry,
    /// Canonical project root — the default containment boundary for working_dir.
    /// `None` when no explicit `*_PROJECT_DIR` env was set and the process cwd is
    /// recognizably not a project (e.g. a harness plugin/state dir): containment
    /// then relies on `extra_roots` alone instead of silently trusting a bogus cwd.
    project_root: Option<PathBuf>,
    /// Canonical extra roots from DISPATCH_EXTRA_ROOTS — the user's explicit opt-in
    /// to delegate outside the project tree.
    extra_roots: Arc<Vec<PathBuf>>,
    /// Whether `danger-full-access` is permitted (DISPATCH_ALLOW_DANGER).
    allow_danger: bool,
    /// This server process — written on every task it owns, read at peer startup
    /// to decide which stranded rows are safe to reconcile.
    owner_pid: i64,
    owner_instance: String,
    /// Backend --version strings probed once at boot, recorded on each run for audit.
    backend_versions: Arc<HashMap<String, Option<String>>>,
    /// Per-project state directory (dispatch.db plus dispatch-owned logs).
    state_dir: PathBuf,
    /// The server's canonical directory: where a consultation runs, and the
    /// directory its transcript is located for.
    cwd: PathBuf,
    /// The user's home directory, for locating the harness transcript.
    home: PathBuf,
    /// The absolute path of this binary, as the `wait` command line names it.
    wait_exe: String,
    /// The absolute path of `dispatch.db`, as the `wait` command line names it.
    db_path: PathBuf,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl Dispatch {
    #[tool(
        description = "Delegate ONE execution step to a coding-agent backend (codex, opencode, claude, or agy) running headless and WRITE-CAPABLE in `working_dir` — it may modify files there. Runs ASYNCHRONOUSLY: returns a task id and a `wait` command line immediately; run that command with the harness's background means to block until the run is terminal (exit 0 terminal, 2 still active at --timeout, 3 unknown id, 1 error), then read the result with dispatch_status(id). Provide a structured spec — objective (required), working_dir (required, absolute), and optional target_files / constraints / acceptance — plus optional free-form context / details; the server renders them into the backend prompt. working_dir is rejected unless it canonicalizes within the project root (widen with the DISPATCH_EXTRA_ROOTS env var). sandbox defaults to workspace-write; danger-full-access is rejected unless the server enables it; agy refuses read-only (headless agy runs only with every tool approved). One active run per working_dir unless allow_concurrent=true. model_fallback: an optional ordered list of models tried in turn on a transient backend error (rate limit, quota, model unavailable) — dispatch_status reports final_model/fallback_history when a retry occurred; not honored by dispatch_steer. Watchdog: a fresh codex submit whose backend log never associates within ~30s (DISPATCH_RESTART_UNASSOCIATED_SECS; 0=off) AND whose working_dir shows no writes is killed and auto-resubmitted ONCE as a fresh task — the old row goes interrupted with restarted_as pointing at the successor. POLICY: initiate dispatch according to the harness-rendered dispatch preferences file (`conservative` / `preference-only` / `proactive`) — under `proactive` + `auto`, submit directly for suitable steps; this policy governs dispatch specifically and is not subject to the general write-capable delegation propose-and-wait default used elsewhere. APPROVAL: before the FIRST dispatch in a session, confirm working_dir + the step scope + the approval granularity (per-step vs batch) with the user when approval mode is ask; skip that confirmation only when approval mode is auto."
    )]
    async fn dispatch_submit(
        &self,
        Parameters(p): Parameters<SubmitParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        if let Some(refusal) = reentry_refusal(Kind::Execute) {
            return Ok(refusal);
        }
        if p.objective.trim().is_empty() {
            return Ok(err_struct(ErrCode::InvalidParams, "objective is required"));
        }
        if p.working_dir.trim().is_empty() {
            return Ok(err_struct(
                ErrCode::InvalidParams,
                "working_dir is required",
            ));
        }

        let backend = match Backend::parse(p.backend.as_deref().unwrap_or("")) {
            Some(b) => b,
            None => {
                return Ok(err_struct(
                    ErrCode::UnknownBackend,
                    format!(
                        "unknown backend {:?}; supported: codex, opencode, claude, agy",
                        p.backend.as_deref().unwrap_or("")
                    ),
                ));
            }
        };

        let canon = match self.check_working_dir(&p.working_dir) {
            Ok(c) => c,
            Err((code, e)) => return Ok(err_struct(code, e)),
        };
        let canon_str = canon.to_string_lossy().to_string();

        let sandbox = match self.check_sandbox(p.sandbox.as_deref()) {
            Ok(s) => s,
            Err(e) => return Ok(err_struct(ErrCode::SandboxForbidden, e)),
        };
        if let Some(refusal) = sandbox_refusal(backend, &sandbox) {
            return Ok(err_struct(ErrCode::InvalidParams, refusal));
        }

        let allow_concurrent = p.allow_concurrent.unwrap_or(false);
        let nonce = make_nonce(&self.owner_instance);
        let prompt = render::render_prompt(&p, &nonce);
        let spec_json = render::spec_json(&p);
        let model_fallback = p.model_fallback.clone().filter(|v| !v.is_empty());
        let model_fallback_json = model_fallback
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap_or_default());

        let id = {
            let mut conn = match self.lock_db() {
                Ok(c) => c,
                Err(e) => return Ok(e),
            };
            let new = store::NewTask {
                plan_id: nonempty(p.plan_id.clone()),
                backend: backend.as_str().to_string(),
                working_dir: canon_str.clone(),
                title: nonempty(p.title.clone()),
                spec_json,
                prompt: prompt.clone(),
                model: nonempty(p.model.clone()),
                reasoning_effort: nonempty(p.reasoning_effort.clone()),
                sandbox: sandbox.clone(),
                parent_id: None,
                nonce: Some(nonce.clone()),
                rollout_start_line: None,
                model_fallback: model_fallback_json,
                allow_concurrent,
                restart_of: None,
                kind: Kind::Execute,
            };
            let enforce_dir = if allow_concurrent {
                None
            } else {
                Some(canon_str.as_str())
            };
            match store::insert_queued(
                &mut conn,
                &new,
                self.owner_pid,
                &self.owner_instance,
                enforce_dir,
            ) {
                Ok(store::InsertOutcome::Created(id)) => id,
                Ok(store::InsertOutcome::Conflict(existing)) => {
                    return Ok(err_struct(
                        ErrCode::DirBusy,
                        format!(
                            "a dispatch ({existing}) is already active for {canon_str}; \
                         wait for it, cancel it, or pass allow_concurrent=true to override"
                        ),
                    ));
                }
                Err(e) => return Ok(err_struct(ErrCode::DbError, format!("db error: {e}"))),
            }
        };

        let job = executor::Job {
            id: id.clone(),
            kind: executor::JobKind::Execute,
            backend,
            working_dir: canon,
            sandbox: sandbox.clone(),
            model: nonempty(p.model),
            reasoning_effort: nonempty(p.reasoning_effort),
            skip_git_repo_check: p.skip_git_repo_check.unwrap_or(false),
            prompt,
            backend_version: self.backend_version(backend),
            state_dir: self.state_dir.clone(),
            home: self.home.clone(),
            resume_session: None,
            nonce: Some(nonce),
            rollout_path: None,
            model_fallback,
            allow_concurrent,
            restart_of: None,
        };
        executor::spawn(self.db.clone(), self.registry.clone(), job);

        Ok(json_ok(json!({
            "id": id,
            "kind": Kind::Execute.as_str(),
            "status": store::STATUS_QUEUED,
            "backend": backend.as_str(),
            "working_dir": canon_str,
            "sandbox": sandbox,
            "plan_id": nonempty(p.plan_id),
            "wait": self.wait_command(&id),
            "note": "running asynchronously — run the `wait` command with the harness's background means, then read the result with dispatch_status(id); cancel with dispatch_cancel(id)",
        })))
    }

    #[tool(
        description = "Ask another model family (OpenAI's codex CLI by default, or Anthropic's claude CLI with backend=\"claude\") a READ-ONLY question for an independent second opinion. Runs ASYNCHRONOUSLY like dispatch_submit: returns an id and a `wait` command line immediately; run that command with the harness's background means, then read the reply from dispatch_status(id) (`result` once succeeded, `error` once failed). A consultation is stopped only with dispatch_cancel(id). include_transcript defaults to true — the current harness conversation is read from the harness's own session log (Claude Code project transcripts, Codex rollouts, Kimi Code wire logs) when the consultation is submitted and forwarded in REDACTED form (text passes; tool_use / tool_result / thinking blocks become placeholders); a missing transcript is a note in the reply, never a failure. The backend runs in the server's directory with no MCP server of its own: codex under `-s read-only` with `exec --ignore-user-config`, claude in safe-mode with `--permission-mode plan` and only `Read,Grep,Glob,WebFetch` — it may read files but never writes or runs shells. PREFER passing file paths in `question` / `context` and letting the backend read them (cheaper, avoids the transcript's 100 KB cap); embed an excerpt only to focus on a line range or for transient tool output not on disk. Frame the question as an assessment to check, not a conclusion to confirm. model_fallback: an optional ordered list of models retried in turn on a transient backend error; the reply notes when a fallback model answered. A consultation takes no working_dir slot and is accepted even inside a dispatch-spawned backend. Costs third-party API quota."
    )]
    async fn dispatch_consult(
        &self,
        Parameters(p): Parameters<ConsultParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        if let Some(refusal) = reentry_refusal(Kind::Consult) {
            return Ok(refusal);
        }
        if p.question.trim().is_empty() {
            return Ok(err_struct(ErrCode::InvalidParams, "question is required"));
        }
        let backend = match Backend::parse(p.backend.as_deref().unwrap_or("")) {
            Some(Backend::Opencode) => {
                return Ok(err_struct(
                    ErrCode::InvalidParams,
                    "a consultation runs on codex or claude; opencode serves executions only",
                ));
            }
            Some(Backend::Agy) => {
                return Ok(err_struct(
                    ErrCode::InvalidParams,
                    "agy is not a consultation backend: it cannot hide the user's MCP servers \
                     from a run; consult codex or claude (agy serves executions only)",
                ));
            }
            Some(b) => b,
            None => {
                return Ok(err_struct(
                    ErrCode::UnknownBackend,
                    format!(
                        "unknown backend {:?}; a consultation runs on codex or claude",
                        p.backend.as_deref().unwrap_or("")
                    ),
                ));
            }
        };

        let include_transcript = p.include_transcript.unwrap_or(true);
        let (transcript_text, transcript_note) = if include_transcript {
            let (cwd, home, tail) = (self.cwd.clone(), self.home.clone(), p.transcript_tail);
            // The readers do blocking file IO over logs of up to many MB.
            let rendered = tokio::task::spawn_blocking(move || {
                transcript::render_transcript(&cwd, &home, tail)
            })
            .await;
            match rendered {
                Ok(transcript::TranscriptOutcome::Ok { rendered }) => (Some(rendered), None),
                Ok(transcript::TranscriptOutcome::Unavailable(reason)) => {
                    (None, Some(consult::transcript_note(&reason)))
                }
                Err(e) => (
                    None,
                    Some(consult::transcript_note(&format!(
                        "the transcript reader failed: {e}"
                    ))),
                ),
            }
        } else {
            (None, None)
        };
        let prompt = consult::compose_prompt(
            p.context.as_deref(),
            transcript_text.as_deref(),
            &p.question,
        );
        let spec_json = serde_json::to_string(&json!({
            "question": p.question,
            "context": p.context,
            "include_transcript": include_transcript,
            "transcript_tail": p.transcript_tail,
            "transcript_note": transcript_note,
        }))
        .unwrap_or_else(|_| "{}".to_string());
        let model_fallback = p.model_fallback.clone().filter(|v| !v.is_empty());
        let model_fallback_json = model_fallback
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap_or_default());
        let cwd_str = self.cwd.to_string_lossy().to_string();
        let sandbox = agent_exec::Sandbox::ReadOnly.as_str().to_string();

        let id = {
            let mut conn = match self.lock_db() {
                Ok(c) => c,
                Err(e) => return Ok(e),
            };
            let new = store::NewTask {
                plan_id: None,
                backend: backend.as_str().to_string(),
                working_dir: cwd_str,
                title: nonempty(p.title.clone()),
                spec_json,
                prompt: prompt.clone(),
                model: nonempty(p.model.clone()),
                reasoning_effort: nonempty(p.reasoning_effort.clone()),
                sandbox: sandbox.clone(),
                parent_id: None,
                nonce: None,
                rollout_start_line: None,
                model_fallback: model_fallback_json,
                allow_concurrent: false,
                restart_of: None,
                kind: Kind::Consult,
            };
            // A consultation takes no working-dir slot: no guard on insert.
            match store::insert_queued(&mut conn, &new, self.owner_pid, &self.owner_instance, None)
            {
                Ok(store::InsertOutcome::Created(id)) => id,
                Ok(store::InsertOutcome::Conflict(existing)) => {
                    return Ok(err_struct(
                        ErrCode::DbError,
                        format!("unexpected working-dir conflict with {existing}"),
                    ));
                }
                Err(e) => return Ok(err_struct(ErrCode::DbError, format!("db error: {e}"))),
            }
        };

        let job = executor::Job {
            id: id.clone(),
            kind: executor::JobKind::Consult {
                transcript_note: transcript_note.clone(),
                log_marker: consult::question_section(&p.question),
            },
            backend,
            working_dir: self.cwd.clone(),
            sandbox: sandbox.clone(),
            model: nonempty(p.model.clone()),
            reasoning_effort: nonempty(p.reasoning_effort.clone()),
            skip_git_repo_check: false,
            prompt,
            backend_version: self.backend_version(backend),
            state_dir: self.state_dir.clone(),
            home: self.home.clone(),
            resume_session: None,
            nonce: None,
            rollout_path: None,
            model_fallback: model_fallback.clone(),
            allow_concurrent: false,
            restart_of: None,
        };
        executor::spawn(self.db.clone(), self.registry.clone(), job);

        Ok(json_ok(json!({
            "id": id,
            "kind": Kind::Consult.as_str(),
            "status": store::STATUS_QUEUED,
            "backend": backend.as_str(),
            "model": nonempty(p.model),
            "reasoning_effort": nonempty(p.reasoning_effort),
            "model_fallback": model_fallback,
            "include_transcript": include_transcript,
            "transcript_note": transcript_note,
            "wait": self.wait_command(&id),
            "note": "consulting asynchronously — run the `wait` command with the harness's background means, then read the reply from dispatch_status(id); stop it only with dispatch_cancel(id)",
        })))
    }

    #[tool(
        description = "Get the status and (when terminal) the captured result / error of a run of either kind by id. For a consultation, `result` is the reply: a [backend] header, the reply text, and the truncation, fallback and transcript notes that apply; a failed consultation carries the backend's failure text in `error`. Compact by default: the accepted spec, the rendered prompt, and argv are OMITTED so repeated polling of a long run doesn't re-echo the whole spec every call — pass include_spec=true to include them (e.g. to read the accepted spec of a task this session did not submit). Statuses: queued, running, succeeded, failed, cancelled, interrupted. ACTIVE tasks also report child_process_alive, log_associated, and log_last_write_age_seconds. Read these carefully before judging a run stuck: a LIVE process with a quiet/old log is INCONCLUSIVE — the backend may be inside a long-running tool or MCP call (e.g. a multi-minute consultation at high reasoning effort) that emits no log events until it returns. Do not cancel or steer on silence alone; child_process_alive=false, or an interrupted/failed status, is the real failure signal. An interrupted row that the server auto-restarted carries restarted_as (the successor id); the successor carries restart_of."
    )]
    async fn dispatch_status(
        &self,
        Parameters(p): Parameters<StatusParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let id = p.id.trim();
        if id.is_empty() {
            return Ok(err_struct(ErrCode::InvalidParams, "id is required"));
        }
        let row = {
            let conn = match self.lock_db() {
                Ok(c) => c,
                Err(e) => return Ok(e),
            };
            match store::get(&conn, id) {
                Ok(r) => r,
                Err(e) => return Ok(err_struct(ErrCode::DbError, format!("db error: {e}"))),
            }
        };
        match row {
            Some(row) => {
                let row = self.live_reconcile(&row).unwrap_or(row);
                let mut v = row.to_json(true, p.include_spec.unwrap_or(false));
                self.augment_observability(&mut v, &row);
                Ok(json_ok(v))
            }
            None => Ok(err_struct(
                ErrCode::NoSuchTask,
                format!("no task with id {id:?}"),
            )),
        }
    }

    #[tool(
        description = "List runs (insertion order), optionally filtered by plan_id, status and/or kind (execute / consult). The captured result is omitted here — use dispatch_status(id) for a run's output."
    )]
    async fn dispatch_list(
        &self,
        Parameters(p): Parameters<ListParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let kind = match nonempty_ref(&p.kind) {
            None => None,
            Some(k) => match Kind::parse(k) {
                Some(k) => Some(k),
                None => {
                    return Ok(err_struct(
                        ErrCode::InvalidParams,
                        format!("invalid kind {k:?}; allowed: execute, consult"),
                    ));
                }
            },
        };
        let conn = match self.lock_db() {
            Ok(c) => c,
            Err(e) => return Ok(e),
        };
        match store::list(
            &conn,
            nonempty_ref(&p.plan_id),
            nonempty_ref(&p.status),
            kind,
        ) {
            Ok(rows) => {
                let tasks: Vec<Value> = rows.iter().map(|r| r.to_json(false, false)).collect();
                Ok(json_ok(json!({ "count": tasks.len(), "tasks": tasks })))
            }
            Err(e) => Ok(err_struct(ErrCode::DbError, format!("db error: {e}"))),
        }
    }

    #[tool(
        description = "Cancel an active run of either kind by id, or every active step of a plan by plan_id (pass exactly one). Fires the cancellation token; the backend's process group is killed and the run transitions to cancelled. This is the only way to stop a consultation once submitted. A run owned by a different session's server cannot be cancelled from here."
    )]
    async fn dispatch_cancel(
        &self,
        Parameters(p): Parameters<CancelParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        match (nonempty_ref(&p.id), nonempty_ref(&p.plan_id)) {
            (Some(id), None) => {
                let row = {
                    let conn = match self.lock_db() {
                        Ok(c) => c,
                        Err(e) => return Ok(e),
                    };
                    store::get(&conn, id)
                };
                match row {
                    Ok(Some(row)) => {
                        if !store::is_active(&row.status) {
                            return Ok(text_ok(format!(
                                "task {id} is already {} — nothing to cancel",
                                row.status
                            )));
                        }
                        if executor::request_cancel(&self.registry, id) {
                            Ok(text_ok(format!(
                                "cancellation requested for {id}; it will transition to cancelled shortly"
                            )))
                        } else {
                            Ok(text_ok(format!(
                                "task {id} is {} but is not running under this server instance \
                                 (another session may own it) — cannot cancel it from here",
                                row.status
                            )))
                        }
                    }
                    Ok(None) => Ok(err_struct(
                        ErrCode::NoSuchTask,
                        format!("no task with id {id:?}"),
                    )),
                    Err(e) => Ok(err_struct(ErrCode::DbError, format!("db error: {e}"))),
                }
            }
            (None, Some(plan)) => {
                let ids = {
                    let conn = match self.lock_db() {
                        Ok(c) => c,
                        Err(e) => return Ok(e),
                    };
                    store::active_ids_for_plan(&conn, plan)
                };
                match ids {
                    Ok(ids) if ids.is_empty() => {
                        Ok(text_ok(format!("no active tasks in plan {plan:?}")))
                    }
                    Ok(ids) => {
                        let mut cancelled = Vec::new();
                        let mut not_owned_here = Vec::new();
                        for id in ids {
                            if executor::request_cancel(&self.registry, &id) {
                                cancelled.push(id);
                            } else {
                                not_owned_here.push(id);
                            }
                        }
                        Ok(json_ok(json!({
                            "plan_id": plan,
                            "cancelled": cancelled,
                            "not_owned_here": not_owned_here,
                        })))
                    }
                    Err(e) => Ok(err_struct(ErrCode::DbError, format!("db error: {e}"))),
                }
            }
            (Some(_), Some(_)) => Ok(err_struct(
                ErrCode::InvalidParams,
                "pass exactly one of id or plan_id, not both",
            )),
            (None, None) => Ok(err_struct(
                ErrCode::InvalidParams,
                "pass either id or plan_id to cancel",
            )),
        }
    }

    #[tool(
        description = "List which backend CLIs (codex, opencode, claude, agy) are available on PATH, with their --version output and the run kinds each serves (codex and claude: execute and consult; opencode and agy: execute), plus this server's containment configuration (project_root, extra_roots). Call this when you're unsure a backend is installed on this machine, or to check why a working_dir is being rejected."
    )]
    async fn dispatch_backends(
        &self,
        Parameters(_p): Parameters<BackendsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut report = Vec::new();
        for b in Backend::all() {
            let found = match agent_exec::which(b.binary()) {
                Some(path) => {
                    let ver = backend::version(*b)
                        .await
                        .unwrap_or_else(|| "(unknown)".to_string());
                    Some((path.display().to_string(), ver))
                }
                None => None,
            };
            report.push(backend_entry(*b, found));
        }
        Ok(json_ok(json!({
            "backends": report,
            // Containment observability: what this server will accept as
            // working_dir (and the live probe for harness spawn-cwd issues).
            "project_root": self.project_root.as_ref().map(|p| p.display().to_string()),
            "extra_roots": self.extra_roots.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        })))
    }

    #[tool(
        description = "Show a curated, live-updating timeline of what a delegated run is doing. Codex logs are read from codex's rollout; OpenCode logs are dispatch-owned normalized JSONL; Claude logs are read from the session file under ~/.claude/projects (the session id is pinned at spawn); agy logs from ~/.gemini/antigravity-cli/brain/<conversation>/.system_generated/logs/transcript.jsonl of the conversation the run's init event named (no live log until then). Noise (system prompts, token counts, encrypted codex reasoning, raw tool-call output) is filtered out; signal (user/backend messages, tool-call invocations, file edits, lifecycle, and plaintext OpenCode/Claude reasoning) is kept, and never truncated per-field — only the total response is size-capped (see line_start/line_end). Works WHILE the task is still running. Page with line_start/line_end (1-based; omitted = the tail) to avoid output limits — total_lines tells you how to page. kinds filters categories (lifecycle/messages/tools/edits/reasoning/tool_results; by default codex excludes reasoning while opencode, claude and agy include it; tool_results — raw tool-call output — is excluded by default for every backend, request it explicitly). raw=true returns the underlying JSONL. A consultation shows its backend's own session log when one was located (a codex consultation's rollout); otherwise the response says that a consultation has no live log (a claude consultation keeps none) and its reply comes from dispatch_status. A quiet log on a RUNNING task is INCONCLUSIVE, not a hang: backends sit silent for minutes inside long tool/MCP calls (e.g. a consultation at high reasoning effort). Judge liveness by dispatch_status's child_process_alive + log_last_write_age_seconds, not by log silence."
    )]
    async fn dispatch_logs(
        &self,
        Parameters(p): Parameters<LogsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let id = p.id.trim();
        if id.is_empty() {
            return Ok(err_struct(ErrCode::InvalidParams, "id is required"));
        }
        let row = {
            let conn = match self.lock_db() {
                Ok(c) => c,
                Err(e) => return Ok(e),
            };
            store::get(&conn, id)
        };
        let row = match row {
            Ok(Some(r)) => r,
            Ok(None) => {
                return Ok(err_struct(
                    ErrCode::NoSuchTask,
                    format!("no task with id {id:?}"),
                ));
            }
            Err(e) => return Ok(err_struct(ErrCode::DbError, format!("db error: {e}"))),
        };
        let rollout_path = match self.resolve_rollout(&row) {
            Some(p) => p,
            None if row.kind() == Kind::Consult => {
                return Ok(json_ok(json!({
                    "id": id, "kind": row.kind, "status": row.status, "session_pending": false, "log": "",
                    "note": "a consultation has no live log: no backend session log was located for it (a claude consultation keeps none). Its reply is returned by dispatch_status once the run is terminal.",
                })));
            }
            None if row.backend == Backend::Agy.as_str() => {
                let note = match row.session_id.as_deref().filter(|s| !s.is_empty()) {
                    None => "no live log yet: an agy run's log is found by the conversation id its init event reports, and none has arrived; try again shortly".to_string(),
                    Some(sid) => format!(
                        "no live log yet: agy conversation {sid} has not written {} yet; try again shortly",
                        agy::transcript_path(&self.home, sid).display()
                    ),
                };
                return Ok(json_ok(json!({
                    "id": id, "status": row.status, "session_id": row.session_id,
                    "session_pending": true, "log": "", "note": note,
                })));
            }
            None => {
                return Ok(json_ok(json!({
                    "id": id, "status": row.status, "session_pending": true, "log": "",
                    "note": "no backend log associated yet — the run may not have written its log, or its association is still pending; try again shortly. A fresh codex submit that stays unassociated ~30s with no working-dir writes is auto-restarted once by the server (dispatch_status shows restarted_as when that happened).",
                })));
            }
        };
        let jsonl = match rollout::read_to_string(Path::new(&rollout_path)) {
            Ok(s) => s,
            Err(e) => {
                return Ok(err_struct(
                    ErrCode::RolloutUnreadable,
                    format!("could not read rollout {rollout_path}: {e}"),
                ));
            }
        };
        // For a steered task, codex appended its new turn to the inherited parent
        // rollout; skip the lines that predate the steer so logs show only this turn.
        let jsonl = trim_to_start_line(jsonl, row.rollout_start_line);
        let start = p.line_start.map(|n| n as usize);
        let end = p.line_end.map(|n| n as usize);

        if p.raw.unwrap_or(false) {
            let lines: Vec<String> = jsonl.lines().map(str::to_string).collect();
            let (text, s, e, capped) = rollout::window(&lines, start, end);
            return Ok(json_ok(json!({
                "id": id, "status": row.status, "raw": true, "session_pending": false,
                "rollout_path": rollout_path, "total_lines": lines.len(),
                "shown_lines": format!("{s}-{e}"), "byte_capped": capped, "log": text,
            })));
        }

        let kinds = p
            .kinds
            .unwrap_or_else(|| rollout::default_kinds(&row.backend));
        // Elide the dispatch-authored prompt echo only for fresh submits — a
        // steered task (parent_id set) carries a new user instruction that must
        // stay visible, and its pre-steer lines are already trimmed by
        // rollout_start_line. Never elides under raw=true (handled above).
        let elide = row.parent_id.is_none().then_some(row.prompt.as_str());
        let rendered = if row.backend == "claude" {
            rollout::curate_claude(&jsonl, &kinds, elide)
        } else if row.backend == Backend::Agy.as_str() {
            rollout::curate_agy(&jsonl, &kinds, elide)
        } else {
            rollout::curate(&jsonl, &kinds, elide)
        };
        let (text, s, e, capped) = rollout::window(&rendered.lines, start, end);
        Ok(json_ok(json!({
            "id": id, "status": row.status, "session_id": row.session_id,
            "session_pending": false, "rollout_path": rollout_path, "kinds": kinds,
            "total_lines": rendered.total, "shown_lines": format!("{s}-{e}"),
            "byte_capped": capped, "log": text,
        })))
    }

    #[tool(
        description = "Interrupt a delegated task and steer it with a NEW instruction, continuing the SAME backend session (its accumulated context + the files it already wrote are preserved when the backend supports sessions; agy resumes the conversation its init event named with --conversation). If the task is still running it is cancelled first; then the backend resumes the session with your instruction. Creates a new linked task (parent_id = the steered task) so the turn history shows in dispatch_list. Returns the new id and a `wait` command line, as dispatch_submit does. A consultation cannot be steered (invalid_params); submit a new one instead. Use this for mid-flight 'no, do it this way instead' redirection. Inherits the parent task's allow_concurrent by default; pass allow_concurrent to override the one-run-per-working_dir guard for the resumed run — needed to steer a task in a directory that has OTHER concurrent runs (allow_concurrent=false re-enforces the guard even if the parent bypassed it)."
    )]
    async fn dispatch_steer(
        &self,
        Parameters(p): Parameters<SteerParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // A steer starts a new execution run, so it is refused where a submit is.
        if let Some(refusal) = reentry_refusal(Kind::Execute) {
            return Ok(refusal);
        }
        let id = p.id.trim();
        if id.is_empty() {
            return Ok(err_struct(ErrCode::InvalidParams, "id is required"));
        }
        if p.instruction.trim().is_empty() {
            return Ok(err_struct(
                ErrCode::InvalidParams,
                "instruction is required",
            ));
        }
        let parent = {
            let conn = match self.lock_db() {
                Ok(c) => c,
                Err(e) => return Ok(e),
            };
            store::get(&conn, id)
        };
        let parent = match parent {
            Ok(Some(r)) => r,
            Ok(None) => {
                return Ok(err_struct(
                    ErrCode::NoSuchTask,
                    format!("no task with id {id:?}"),
                ));
            }
            Err(e) => return Ok(err_struct(ErrCode::DbError, format!("db error: {e}"))),
        };
        if parent.kind() == Kind::Consult {
            return Ok(err_struct(
                ErrCode::InvalidParams,
                format!(
                    "{id} is a consultation; a consultation keeps no resumable session and cannot \
                     be steered — submit a new dispatch_consult instead"
                ),
            ));
        }
        let session_id = match self.resolve_session_id(&parent) {
            Some(s) => s,
            None => {
                return Ok(err_struct(
                    ErrCode::SessionNotReady,
                    format!(
                        "no backend session recorded for {id} yet — it may not have started; check dispatch_status / dispatch_logs first"
                    ),
                ));
            }
        };

        // Interrupt if still active, then wait for it to actually terminate so the
        // working_dir is free before the resume run starts.
        if store::is_active(&parent.status) {
            executor::request_cancel(&self.registry, id);
            let mut waited_ms = 0u64;
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                waited_ms += 200;
                let still_active = if let Ok(conn) = self.db.lock() {
                    store::get(&conn, id)
                        .ok()
                        .flatten()
                        .map(|r| store::is_active(&r.status))
                        .unwrap_or(false)
                } else {
                    false
                };
                if !still_active {
                    break;
                }
                if waited_ms >= STEER_TERMINATE_WAIT_MS {
                    return Ok(err_struct(
                        ErrCode::DirBusy,
                        format!(
                            "{id} is still terminating after cancel; retry dispatch_steer shortly"
                        ),
                    ));
                }
            }
        }

        let instruction = p.instruction.trim().to_string();
        let prompt = format!(
            "Continue working in the same workspace on the SAME task. New instruction from the user:\n\n{instruction}\n\nWhen finished, end with a short summary of what you changed."
        );
        let spec_json = serde_json::to_string(&json!({
            "objective": instruction, "working_dir": parent.working_dir, "resume_of": id,
        }))
        .unwrap_or_else(|_| "{}".to_string());

        // The parent's rollout file: resumed backends append their new turn here. We use it
        // for the steered row's start-line boundary (logs show only the new turn) and to
        // set the steered row's identity immediately (below).
        let parent_rollout = self.resolve_rollout(&parent);
        let rollout_start_line = parent_rollout
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|s| s.lines().count() as i64);
        // Execution knobs the steered run inherits (overridable per call) — captured
        // once so they can be reused for the row, the job, and the echoed response.
        let eff_model = nonempty(p.model.clone()).or_else(|| parent.model.clone());
        let eff_effort =
            nonempty(p.reasoning_effort.clone()).or_else(|| parent.reasoning_effort.clone());
        let eff_sandbox = parent.sandbox.clone();
        // A steer inherits the parent's persisted allow_concurrent unless the steer
        // call sets it explicitly (Some(false) is a real override, not "unset").
        let eff_allow_concurrent =
            effective_allow_concurrent(p.allow_concurrent, parent.allow_concurrent);

        let new_id = {
            let mut conn = match self.lock_db() {
                Ok(c) => c,
                Err(e) => return Ok(e),
            };
            let new = store::NewTask {
                plan_id: parent.plan_id.clone(),
                backend: parent.backend.clone(),
                working_dir: parent.working_dir.clone(),
                title: Some(format!("steer of {id}")),
                spec_json,
                prompt: prompt.clone(),
                model: eff_model.clone(),
                reasoning_effort: eff_effort.clone(),
                sandbox: eff_sandbox.clone(),
                parent_id: Some(id.to_string()),
                nonce: None,
                rollout_start_line,
                // A steer/resume run stays on one model — switching models
                // mid-resumed-session is a materially harder problem than a
                // fresh-attempt fallback and is out of scope here.
                model_fallback: None,
                // Persist the *effective* value so a chain of steers keeps inheriting.
                allow_concurrent: eff_allow_concurrent,
                restart_of: None,
                kind: Kind::Execute,
            };
            match store::insert_queued(
                &mut conn,
                &new,
                self.owner_pid,
                &self.owner_instance,
                if eff_allow_concurrent {
                    None
                } else {
                    Some(parent.working_dir.as_str())
                },
            ) {
                Ok(store::InsertOutcome::Created(nid)) => nid,
                Ok(store::InsertOutcome::Conflict(existing)) => {
                    return Ok(err_struct(
                        ErrCode::DirBusy,
                        format!(
                            "another dispatch ({existing}) is active for {}; cancel it first",
                            parent.working_dir
                        ),
                    ));
                }
                Err(e) => return Ok(err_struct(ErrCode::DbError, format!("db error: {e}"))),
            }
        };

        // Record the steered row's identity now — we already know the resumed session and
        // the inherited rollout file — so dispatch_logs / dispatch_steer never fall back to
        // a cwd guess in the window before the executor records it.
        if let Some(rp) = parent_rollout.as_deref()
            && let Ok(conn) = self.db.lock()
        {
            let _ = store::set_session(&conn, &new_id, &session_id, rp);
        }

        let job = executor::Job {
            id: new_id.clone(),
            kind: executor::JobKind::Execute,
            backend: Backend::parse(&parent.backend).unwrap_or(Backend::Codex),
            working_dir: PathBuf::from(&parent.working_dir),
            sandbox: eff_sandbox.clone(),
            model: eff_model.clone(),
            reasoning_effort: eff_effort.clone(),
            skip_git_repo_check: true,
            prompt,
            backend_version: self
                .backend_version(Backend::parse(&parent.backend).unwrap_or(Backend::Codex)),
            state_dir: self.state_dir.clone(),
            home: self.home.clone(),
            resume_session: Some(session_id.clone()),
            nonce: None,
            rollout_path: parent_rollout.as_deref().map(PathBuf::from),
            model_fallback: None,
            allow_concurrent: eff_allow_concurrent,
            restart_of: None,
        };
        executor::spawn(self.db.clone(), self.registry.clone(), job);

        Ok(json_ok(json!({
            "id": new_id,
            "parent_id": id,
            "status": store::STATUS_QUEUED,
            "resumed_session": session_id,
            "working_dir": parent.working_dir,
            "sandbox": eff_sandbox,
            "model": eff_model,
            "reasoning_effort": eff_effort,
            "allow_concurrent": eff_allow_concurrent,
            "wait": self.wait_command(&new_id),
            "note": "steering: the backend session was resumed with your new instruction (it inherits the echoed sandbox/model/reasoning_effort/allow_concurrent unless you overrode them) — run the `wait` command with the harness's background means, then read dispatch_status / dispatch_logs",
        })))
    }
}

// ── guards + helpers (non-tool impl) ──────────────────────

impl Dispatch {
    /// The command line that blocks until run `id` is terminal: this binary's
    /// absolute path, `wait <id>` and `--db` with the store's absolute path.
    fn wait_command(&self, id: &str) -> String {
        wait_command_line(&self.wait_exe, id, &self.db_path)
    }

    fn backend_version(&self, backend: Backend) -> Option<String> {
        self.backend_versions
            .get(backend.as_str())
            .and_then(|v| v.clone())
    }

    fn lock_db(&self) -> Result<std::sync::MutexGuard<'_, rusqlite::Connection>, CallToolResult> {
        self.db
            .lock()
            .map_err(|e| err_struct(ErrCode::DbError, format!("database lock poisoned: {e}")))
    }

    /// Live reconcile at read time: an ACTIVE row whose owning dispatch server
    /// process is gone gets conditionally marked interrupted right now, rather
    /// than lingering as a phantom `running` until some server's next boot.
    /// Returns the refreshed row when a transition happened. The conditional
    /// `mark_interrupted` makes the race with a normal terminal write safe.
    fn live_reconcile(&self, row: &store::TaskRow) -> Option<store::TaskRow> {
        if !store::is_active(&row.status) {
            return None;
        }
        // owner_pid is always recorded at insert; a missing one is a legacy row
        // that boot reconciliation owns — don't guess here.
        let owner_dead = matches!(row.owner_pid, Some(p) if !process_alive(p as i32));
        if !owner_dead {
            return None;
        }
        let conn = self.db.lock().ok()?;
        let n = store::mark_interrupted(
            &conn,
            &row.id,
            "owning dispatch server exited while the task was active (reconciled at read time)",
        )
        .unwrap_or(0);
        if n == 1 {
            store::get(&conn, &row.id).ok().flatten()
        } else {
            None
        }
    }

    /// Observability fields for status/wait responses. For an ACTIVE row:
    /// `child_process_alive` (the spawned process-group leader — on Linux and
    /// macOS the `agent-guard` process, which exits when the backend it
    /// started exits; descendants of the backend may outlive it),
    /// `log_associated`, and `log_last_write_age_seconds` (mtime age of the
    /// associated backend log — a liveness-of-output signal, NOT proof of
    /// semantic progress or of a hang: a live process with an old log may be
    /// inside a long tool/MCP call). For an interrupted row that was
    /// auto-restarted: `restarted_as`, the successor's id.
    fn augment_observability(&self, v: &mut Value, row: &store::TaskRow) {
        if store::is_active(&row.status) {
            if let Some(pid) = row.child_pid {
                v["child_process_alive"] = json!(process_alive(pid as i32));
            }
            let rollout = self.resolve_rollout(row);
            v["log_associated"] = json!(rollout.is_some());
            if let Some(age) = rollout
                .as_deref()
                .and_then(|p| mtime_age_seconds(Path::new(p)))
            {
                v["log_last_write_age_seconds"] = json!(age);
            }
        }
        if row.status == store::STATUS_INTERRUPTED
            && let Ok(conn) = self.db.lock()
            && let Ok(Some(succ)) = store::restart_successor(&conn, &row.id)
        {
            v["restarted_as"] = json!(succ);
        }
    }

    /// Resolve a task's backend log path. A cached value is trusted only if it still
    /// validates as this task's (`rollout_is_ours`) — self-healing a row poisoned by the
    /// old cwd-guessing code. Otherwise it re-locates by identity (`locate_validated`).
    /// It never returns a bare cwd guess: with nothing matching it returns None (fail
    /// closed) rather than hand back another session's rollout.
    fn resolve_rollout(&self, row: &store::TaskRow) -> Option<String> {
        if row.kind() == Kind::Consult {
            // Only the association the executor recorded, checked against its
            // session id; never re-located. A consultation runs in the server's
            // directory with no nonce, where a cwd guess could claim another
            // run's rollout.
            let p = row.rollout_path.as_deref()?;
            let sid = row.session_id.as_deref().filter(|s| !s.is_empty())?;
            return (Path::new(p).exists() && rollout::rollout_has_session_id(Path::new(p), sid))
                .then(|| p.to_string());
        }
        if let Some(p) = row.rollout_path.as_ref()
            && Path::new(p).exists()
            && self.rollout_is_ours(row, Path::new(p))
        {
            return Some(p.clone());
        }
        let (path, sid) = self.locate_validated(row)?;
        let path_str = path.to_string_lossy().to_string();
        if let Ok(conn) = self.db.lock() {
            let _ = store::set_session(&conn, &row.id, &sid, &path_str);
        }
        Some(path_str)
    }

    /// Resolve a task's backend session id (for `dispatch_steer`'s resume). The stored sid
    /// is trusted only if its cached rollout still validates as this task's — otherwise it
    /// is re-derived by identity, so a steer can never resume a poisoned / unrelated session.
    fn resolve_session_id(&self, row: &store::TaskRow) -> Option<String> {
        if row.backend == Backend::Agy.as_str() {
            // The conversation agy's own init event named is authoritative,
            // whether or not its log file exists yet.
            return row
                .session_id
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(str::to_string);
        }
        if let Some(s) = row.session_id.as_deref().filter(|s| !s.is_empty())
            && let Some(p) = row.rollout_path.as_deref()
            && Path::new(p).exists()
            && self.rollout_is_ours(row, Path::new(p))
        {
            return Some(s.to_string());
        }
        let (path, sid) = self.locate_validated(row)?;
        if let Ok(conn) = self.db.lock() {
            let _ = store::set_session(&conn, &row.id, &sid, &path.to_string_lossy());
        }
        Some(sid)
    }

    /// Whether the rollout at `path` belongs to this task, by an INDEPENDENT signal in
    /// priority order: the prompt nonce (fresh tasks); the inherited authoritative
    /// session id (steered tasks, identified by `parent_id`); else — a legacy row — cwd
    /// plus a not-older-than-start time floor. The floor is what avoids the circular case
    /// where a poisoned row's session_id was copied from the same stale file.
    fn rollout_is_ours(&self, row: &store::TaskRow, path: &Path) -> bool {
        if row.backend == Backend::Agy.as_str() {
            // The log of the conversation the run's init event named, never
            // the directory's latest conversation.
            return row
                .session_id
                .as_deref()
                .is_some_and(|sid| agy::is_transcript_of(path, sid));
        }
        if row.backend == "claude" {
            // Claude session ids are pinned at spawn (a dispatch-minted UUID),
            // so a filename match on <sid>.jsonl is positive identity — the
            // slug directory Claude chose is irrelevant.
            return row
                .session_id
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(|sid| {
                    path.file_name().and_then(|n| n.to_str()) == Some(&format!("{sid}.jsonl"))
                })
                .unwrap_or(false);
        }
        if let Some(n) = row.nonce.as_deref().filter(|n| !n.is_empty()) {
            return rollout::rollout_has_nonce(path, n);
        }
        if row.parent_id.is_some() {
            // Steered task: associate ONLY by the inherited session id — never a cwd
            // guess, even in the window before the session id is recorded (fail closed).
            return row
                .session_id
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(|sid| rollout::rollout_has_session_id(path, sid))
                .unwrap_or(false);
        }
        rollout::rollout_cwd_after(path, Path::new(&row.working_dir), self.task_floor(row))
    }

    /// Re-locate a task's rollout by an independent identity signal: the nonce (fresh —
    /// and ONLY the nonce, so a fresh task never falls back to a cwd guess before its
    /// rollout is written); the inherited session id (steered, by `parent_id`); else —
    /// a legacy row — the newest same-cwd rollout at or after the task's start.
    fn locate_validated(&self, row: &store::TaskRow) -> Option<(PathBuf, String)> {
        if row.backend == Backend::Agy.as_str() {
            let sid = row.session_id.as_deref().filter(|s| !s.is_empty())?;
            let p = agy::transcript_path(&self.home, sid);
            return p.exists().then(|| (p, sid.to_string()));
        }
        if row.backend == "claude" {
            let sid = row.session_id.as_deref().filter(|s| !s.is_empty())?;
            let p = harness_log::claude::claude_session_path(Path::new(&row.working_dir), sid);
            if p.exists() {
                return Some((p, sid.to_string()));
            }
            // Slug-formula miss (Claude's exact slugging has edge cases) —
            // find the pinned sid's file wherever Claude put it.
            return harness_log::claude::find_claude_session(sid).map(|p| (p, sid.to_string()));
        }
        if let Some(n) = row.nonce.as_deref().filter(|n| !n.is_empty()) {
            return rollout::locate_by_nonce(Path::new(&row.working_dir), n);
        }
        if row.parent_id.is_some() {
            // Steered task: only the inherited session id; fail closed otherwise.
            return row
                .session_id
                .as_deref()
                .filter(|s| !s.is_empty())
                .and_then(|sid| {
                    harness_log::codex::locate_by_session_id(sid).map(|p| (p, sid.to_string()))
                });
        }
        rollout::locate_new_by_cwd(
            Path::new(&row.working_dir),
            &HashSet::new(),
            self.task_floor(row),
        )
    }

    /// The lower time bound for a legacy row's rollout: the task's start (or creation),
    /// minus a tolerance for clock / ordering skew. None if it can't be parsed.
    fn task_floor(&self, row: &store::TaskRow) -> Option<SystemTime> {
        let raw = row.started_at.as_deref().unwrap_or(row.created_at.as_str());
        parse_sqlite_utc(raw)?.checked_sub(Duration::from_secs(FLOOR_TOLERANCE_SECS))
    }

    /// Containment guard: working_dir must be absolute, exist, be a directory, and
    /// canonicalize within the project root OR a user-allowlisted extra root. This
    /// is the real, model-proof boundary on a write-capable subprocess.
    fn check_working_dir(&self, raw: &str) -> Result<PathBuf, (ErrCode, String)> {
        let raw = raw.trim();
        let p = Path::new(raw);
        if !p.is_absolute() {
            return Err((
                ErrCode::InvalidWorkingDir,
                format!("working_dir must be an absolute path, got {raw:?}"),
            ));
        }
        let canon = p.canonicalize().map_err(|e| {
            (
                ErrCode::InvalidWorkingDir,
                format!("working_dir {raw:?} cannot be resolved (does it exist?): {e}"),
            )
        })?;
        if !canon.is_dir() {
            return Err((
                ErrCode::InvalidWorkingDir,
                format!("working_dir {} is not a directory", canon.display()),
            ));
        }
        containment_check(&canon, self.project_root.as_deref(), &self.extra_roots)?;
        Ok(canon)
    }

    /// Sandbox ceiling: workspace-write (default) and read-only are always allowed;
    /// danger-full-access requires the server to opt in via DISPATCH_ALLOW_DANGER.
    fn check_sandbox(&self, s: Option<&str>) -> Result<String, String> {
        let s = s
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .unwrap_or("workspace-write");
        match s {
            "read-only" | "workspace-write" => Ok(s.to_string()),
            "danger-full-access" => {
                if self.allow_danger {
                    Ok(s.to_string())
                } else {
                    Err(
                        "sandbox 'danger-full-access' is disabled on this server. Set \
                         DISPATCH_ALLOW_DANGER=1 to permit running codex with no sandbox."
                            .to_string(),
                    )
                }
            }
            other => Err(format!(
                "invalid sandbox {other:?}; allowed: read-only, workspace-write \
                 (or danger-full-access if the server enables it)"
            )),
        }
    }
}

// ── free helpers ──────────────────────────────────────────

/// The containment decision proper: a canonical working_dir is allowed inside
/// the project root or any allowlisted extra root. With no project root and no
/// extra roots there is no boundary to check against — that is a configuration
/// error (`no_project_root`), not a rejection of this particular directory.
fn containment_check(
    canon: &Path,
    project_root: Option<&Path>,
    extra_roots: &[PathBuf],
) -> Result<(), (ErrCode, String)> {
    let in_project = project_root.map(|r| canon.starts_with(r)).unwrap_or(false);
    if in_project || extra_roots.iter().any(|r| canon.starts_with(r)) {
        return Ok(());
    }
    if project_root.is_none() && extra_roots.is_empty() {
        return Err((
            ErrCode::NoProjectRoot,
            format!(
                "no project root is configured for this dispatch server, so working_dir {} \
                 cannot be containment-checked. This harness spawns MCP servers outside the \
                 project (detected cwd is not a project directory). Fix: set \
                 DISPATCH_EXTRA_ROOTS to your workspace root(s) at registration time \
                 (e.g. re-run install-mcp.sh with --roots ~/Workspace), or set \
                 SLATE_PROJECT_DIR for this server.",
                canon.display()
            ),
        ));
    }
    let root_desc = project_root
        .map(|r| r.display().to_string())
        .unwrap_or_else(|| "(no project root)".to_string());
    Err((
        ErrCode::InvalidWorkingDir,
        format!(
            "working_dir {} is outside the project root ({}) and any allowlisted root. \
             dispatch only delegates within the project tree by default; if you intend this, \
             add the root to the DISPATCH_EXTRA_ROOTS env var (an OS-path-list of absolute \
             paths) for this server.",
            canon.display(),
            root_desc
        ),
    ))
}

fn text_ok(msg: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![Content::text(msg.into())])
}

fn json_ok(v: Value) -> CallToolResult {
    CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".to_string()),
    )])
}

/// Stable, machine-readable error categories returned alongside the human message, so a
/// calling agent can branch on `error.code` instead of parsing prose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ErrCode {
    InvalidParams,
    NoSuchTask,
    InvalidWorkingDir,
    NoProjectRoot,
    SandboxForbidden,
    DirBusy,
    SessionNotReady,
    UnknownBackend,
    RolloutUnreadable,
    DbError,
    Reentrant,
}

impl ErrCode {
    fn as_str(self) -> &'static str {
        match self {
            ErrCode::InvalidParams => "invalid_params",
            ErrCode::NoSuchTask => "no_such_task",
            ErrCode::InvalidWorkingDir => "invalid_working_dir",
            ErrCode::NoProjectRoot => "no_project_root",
            ErrCode::SandboxForbidden => "sandbox_forbidden",
            ErrCode::DirBusy => "dir_busy",
            ErrCode::SessionNotReady => "session_not_ready",
            ErrCode::UnknownBackend => "unknown_backend",
            ErrCode::RolloutUnreadable => "rollout_unreadable",
            ErrCode::DbError => "db_error",
            ErrCode::Reentrant => "reentrant",
        }
    }
}

/// A structured error result — the `isError` content variant, carrying a stable `code`
/// plus the human-readable `message`.
fn err_struct(code: ErrCode, msg: impl Into<String>) -> CallToolResult {
    let msg = msg.into();
    let body = serde_json::to_string_pretty(&json!({
        "error": { "code": code.as_str(), "message": msg }
    }))
    .unwrap_or_else(|_| msg.clone());
    CallToolResult::error(vec![Content::text(body)])
}

/// Refuse a request of `kind` if this dispatch server is itself running inside
/// a dispatch-spawned backend (the `DISPATCH_REENTRY_DEPTH` env marker is
/// present). An execution (`dispatch_submit`, and `dispatch_steer`, which starts
/// a new execution run) is refused, so the handler short-circuits before any
/// enqueue or spawn — which is what stops `backend → dispatch → backend → …`
/// recursion. A consultation is accepted at any depth: its backend runs with
/// no MCP server, so nothing it starts can reach dispatch again.
fn reentry_refusal(kind: Kind) -> Option<CallToolResult> {
    reentry_refusal_at(kind, agent_exec::reentry::depth(backend::REENTRY_ENV))
}

/// Pure core of `reentry_refusal` at a given server depth.
fn reentry_refusal_at(kind: Kind, depth: u32) -> Option<CallToolResult> {
    if kind == Kind::Execute && depth >= backend::REENTRY_CEILING {
        Some(err_struct(
            ErrCode::Reentrant,
            format!(
                "this dispatch server is running inside a dispatch-spawned backend ({}={}); \
                 nested executions (dispatch_submit, dispatch_steer) are refused to prevent \
                 recursive backend spawning. Do the work in the current backend session \
                 instead; dispatch_consult stays available.",
                backend::REENTRY_ENV,
                depth
            ),
        ))
    } else {
        None
    }
}

/// The run kinds `backend` serves: codex and claude both kinds, opencode and
/// agy executions only.
fn kinds_served(backend: Backend) -> Vec<&'static str> {
    match backend {
        Backend::Codex | Backend::Claude => vec![Kind::Execute.as_str(), Kind::Consult.as_str()],
        Backend::Opencode | Backend::Agy => vec![Kind::Execute.as_str()],
    }
}

/// One `dispatch_backends` entry: the backend, whether its CLI was found
/// (with its path and `--version`), and the run kinds it serves.
fn backend_entry(backend: Backend, found: Option<(String, String)>) -> Value {
    match found {
        Some((path, version)) => json!({
            "backend": backend.as_str(),
            "available": true,
            "path": path,
            "version": version,
            "kinds": kinds_served(backend),
        }),
        None => json!({
            "backend": backend.as_str(),
            "available": false,
            "path": null,
            "version": null,
            "kinds": kinds_served(backend),
        }),
    }
}

/// Why `backend` cannot run an execution under `sandbox` (agy: read-only),
/// or `None` when it can.
fn sandbox_refusal(backend: Backend, sandbox: &str) -> Option<&'static str> {
    let process = backend.process()?;
    process.sandbox_refusal(agent_exec::Sandbox::parse(sandbox)?)
}

/// `<exe> wait <id> --db <db>`: the command line a harness runs to block on a
/// run. Both paths are absolute, because the harness's shell carries neither
/// the server's state home nor its binary folder; each is double-quoted when
/// it holds whitespace or a shell-special character, so the line survives sh,
/// zsh, pwsh and cmd.
fn wait_command_line(exe: &str, id: &str, db: &Path) -> String {
    format!(
        "{} wait {id} --db {}",
        shell_word(exe),
        shell_word(&path_text(db))
    )
}

/// The absolute path of the running binary, canonicalized, as text for a
/// command line or a configuration value; `dispatch` when it cannot be read.
fn current_exe_path() -> String {
    std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .map(|p| path_text(&p))
        .unwrap_or_else(|_| "dispatch".to_string())
}

/// A path as text, without the `\\?\` verbatim prefix Windows'
/// `canonicalize` adds (a shell or a user reading the line expects a plain
/// drive or UNC path).
fn path_text(p: &Path) -> String {
    let text = p.display().to_string();
    if cfg!(windows) {
        strip_verbatim(&text)
    } else {
        text
    }
}

/// `\\?\C:\x` → `C:\x`; `\\?\UNC\server\share` → `\\server\share`; any other
/// text unchanged.
fn strip_verbatim(text: &str) -> String {
    match text.strip_prefix(r"\\?\") {
        Some(rest) => match rest.strip_prefix(r"UNC\") {
            Some(unc) => format!(r"\\{unc}"),
            None => rest.to_string(),
        },
        None => text.to_string(),
    }
}

/// `word` as one shell word: unchanged when it holds only characters no
/// supported shell treats specially, else wrapped in double quotes. A
/// backslash is the path separator on Windows and stays bare there; elsewhere
/// it is special. A word containing `"` cannot be quoted this way and is
/// wrapped as is.
fn shell_word(word: &str) -> String {
    let special = |c: char| {
        c.is_whitespace() || "\"'`$!&|;<>()*?[]{}#^%".contains(c) || (c == '\\' && !cfg!(windows))
    };
    // `~` expands only at the start of a word (Windows short names such as
    // `RUNNER~1` carry it mid-path).
    if !word.is_empty() && !word.starts_with('~') && !word.chars().any(special) {
        word.to_string()
    } else {
        format!("\"{word}\"")
    }
}

/// A steer inherits the parent task's persisted allow_concurrent unless the steer
/// call sets it explicitly. `Some(false)` is a real override (re-enforce the guard),
/// distinct from `None` (inherit) — hence `unwrap_or(parent)`, NOT `unwrap_or(false)`.
fn effective_allow_concurrent(param: Option<bool>, parent: bool) -> bool {
    param.unwrap_or(parent)
}

/// Seconds since `p` was last written. A future mtime (clock skew) reads as 0
/// — "just now" — rather than an error.
fn mtime_age_seconds(p: &Path) -> Option<u64> {
    let m = std::fs::metadata(p).ok()?.modified().ok()?;
    Some(
        SystemTime::now()
            .duration_since(m)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    )
}

fn nonempty(o: Option<String>) -> Option<String> {
    o.filter(|s| !s.trim().is_empty())
}

fn nonempty_ref(o: &Option<String>) -> Option<&str> {
    o.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// Monotonic counter for `make_nonce`.
static NONCE_SEQ: AtomicU64 = AtomicU64::new(0);

/// A per-task identity nonce: the server instance id (unique per process) plus a
/// monotonic counter, so it is unique across tasks and servers and effectively never
/// collides with prompt text. `render::render_prompt` embeds it; `rollout::locate_by_nonce`
/// matches it back to the rollout this task produced.
fn make_nonce(instance: &str) -> String {
    let n = NONCE_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{instance}-{n}")
}

/// Drop the first `start_line` raw rollout lines — a steered task's inherited parent
/// turns — so `dispatch_logs` shows only the new turn. A no-op when unset / non-positive.
fn trim_to_start_line(jsonl: String, start_line: Option<i64>) -> String {
    match start_line {
        Some(n) if n > 0 => jsonl
            .lines()
            .skip(n as usize)
            .collect::<Vec<_>>()
            .join("\n"),
        _ => jsonl,
    }
}

/// Parse a SQLite `datetime('now')` string ("YYYY-MM-DD HH:MM:SS", UTC) into a
/// `SystemTime`, without pulling in a date crate. Returns None on any malformed field.
/// Uses Howard Hinnant's days-from-civil algorithm.
fn parse_sqlite_utc(s: &str) -> Option<SystemTime> {
    let (date, time) = s.trim().split_once(' ')?;
    let mut d = date.split('-');
    let y: i64 = d.next()?.parse().ok()?;
    let mo: i64 = d.next()?.parse().ok()?;
    let da: i64 = d.next()?.parse().ok()?;
    let mut t = time.split(':');
    let h: i64 = t.next()?.parse().ok()?;
    let mi: i64 = t.next()?.parse().ok()?;
    let se: i64 = t.next()?.parse().ok()?;
    if !(1..=12).contains(&mo)
        || !(1..=31).contains(&da)
        || !(0..=23).contains(&h)
        || !(0..=59).contains(&mi)
        || !(0..=60).contains(&se)
    {
        return None;
    }
    let yy = if mo <= 2 { y - 1 } else { y };
    let era = (if yy >= 0 { yy } else { yy - 399 }) / 400;
    let yoe = yy - era * 400;
    let mp = (mo + 9) % 12; // Mar=0 … Feb=11
    let doy = (153 * mp + 2) / 5 + da - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let secs = days * 86400 + h * 3600 + mi * 60 + se;
    if secs < 0 {
        return None;
    }
    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs as u64))
}

fn env_truthy(key: &str) -> bool {
    std::env::var(key)
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// State dir for dispatch.db.
///
/// `DISPATCH_STATE_DIR` is the explicit override. Without it, the directory is
/// anchored under `SLATE_AGENT_STATE_HOME` / `AGENT_KIT_STATE_HOME`, or
/// `~/.slate-agent-kit/projects/{dashed-project}/dispatch`. When no project
/// root could be resolved, state is keyed to the `_no-project` slug instead of
/// a bogus (plugin/state) directory path.
fn resolve_state_dir(project_root: Option<&Path>) -> PathBuf {
    if let Some(dir) = std::env::var_os("DISPATCH_STATE_DIR") {
        return PathBuf::from(dir);
    }
    let project_path = match project_root {
        // Separators and the Windows drive colon must all flatten to `-` —
        // `:` is invalid in Windows file names and `\` would nest directories.
        Some(p) => p.to_string_lossy().replace(['/', '\\', ':'], "-"),
        None => "_no-project".to_string(),
    };
    let state_home = std::env::var_os("SLATE_AGENT_STATE_HOME")
        .or_else(|| std::env::var_os("AGENT_KIT_STATE_HOME"))
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|h| h.join(".slate-agent-kit")))
        .unwrap_or_else(std::env::temp_dir);
    state_home
        .join("projects")
        .join(&project_path)
        .join("dispatch")
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .ok()
}

/// Resolve the canonical project root, or `None` when it cannot be trusted.
///
/// An explicit `SLATE_PROJECT_DIR` / `AGENT_KIT_PROJECT_DIR` / `CLAUDE_PROJECT_DIR`
/// always wins. Otherwise the process cwd is used **only if it plausibly is a
/// project directory** — harnesses that spawn MCP servers elsewhere (e.g. the
/// Kimi plugin pins cwd to the plugin dir) would otherwise silently turn the
/// containment boundary into a directory no delegation ever targets.
fn resolve_project_root(cwd: &Path) -> Option<PathBuf> {
    for key in [
        "SLATE_PROJECT_DIR",
        "AGENT_KIT_PROJECT_DIR",
        "CLAUDE_PROJECT_DIR",
    ] {
        if let Ok(v) = std::env::var(key)
            && !v.trim().is_empty()
        {
            let p = PathBuf::from(v);
            // Canonicalize so symlink / case / `..` aliases of one project don't
            // split it into multiple state dirs (which would defeat
            // reconciliation + the dir guard).
            return Some(p.canonicalize().unwrap_or(p));
        }
    }
    let canon = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    if plausible_fallback_root(&canon, &implausible_roots()) {
        Some(canon)
    } else {
        None
    }
}

/// Directories a project cwd can never be: filesystem root, $HOME itself, and
/// anything under a harness home / Slate state home. Each root is
/// canonicalized so a symlinked home (e.g. macOS `/var` → `/private/var` for a
/// temp state home) still matches the canonicalized cwd.
fn implausible_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let home = home_dir();
    if let Some(h) = &home {
        roots.push(h.join(".claude"));
        roots.push(
            std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| h.join(".codex")),
        );
        roots.push(
            std::env::var_os("KIMI_CODE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| h.join(".kimi-code")),
        );
        roots.push(
            std::env::var_os("SLATE_AGENT_STATE_HOME")
                .or_else(|| std::env::var_os("AGENT_KIT_STATE_HOME"))
                .map(PathBuf::from)
                .unwrap_or_else(|| h.join(".slate-agent-kit")),
        );
    }
    roots
        .into_iter()
        .map(|r| r.canonicalize().unwrap_or(r))
        .collect()
}

fn plausible_fallback_root(canon: &Path, denied: &[PathBuf]) -> bool {
    if canon == Path::new("/") || canon.parent().is_none() {
        return false;
    }
    if let Some(h) = home_dir()
        && canon == h.as_path()
    {
        return false;
    }
    !denied.iter().any(|d| canon.starts_with(d))
}

fn parse_extra_roots() -> Vec<PathBuf> {
    match std::env::var_os("DISPATCH_EXTRA_ROOTS") {
        Some(v) => std::env::split_paths(&v)
            .filter_map(|p| p.canonicalize().ok())
            .collect(),
        None => Vec::new(),
    }
}

/// Boot reconciliation: a freshly started server owns no running child, so any
/// `queued`/`running` row whose owner process is gone is stranded — mark it
/// interrupted. Rows owned by a still-live peer server are left untouched.
fn reconcile(conn: &rusqlite::Connection) {
    let actives = match store::active_owners(conn) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("dispatch: reconcile read failed: {e}");
            return;
        }
    };
    let mut reconciled = 0usize;
    for (id, owner_pid) in actives {
        let dead = match owner_pid {
            Some(pid) => !process_alive(pid as i32),
            None => true,
        };
        if dead {
            match store::mark_interrupted(
                conn,
                &id,
                "owning dispatch server is no longer running (reconciled at startup)",
            ) {
                Ok(n) => reconciled += n,
                Err(e) => tracing::warn!("dispatch: reconcile mark_interrupted({id}) failed: {e}"),
            }
        }
    }
    if reconciled > 0 {
        tracing::info!("dispatch: reconciled {reconciled} stranded task(s) to interrupted");
    }
}

// ── ServerHandler ─────────────────────────────────────────

impl ServerHandler for Dispatch {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Delegation and consultation tools. dispatch runs one backend CLI per run, in two \
             kinds. An EXECUTION (dispatch_submit) hands a coding-agent backend (codex, opencode, \
             claude, or agy) a task: the backend runs headless and WRITE-CAPABLE, modifying files \
             under a target directory. A CONSULTATION (dispatch_consult) asks another model \
             family (codex or claude) a read-only question for an independent second opinion: \
             the backend runs in the server's directory with no MCP server of its own (codex \
             `-s read-only` with `exec --ignore-user-config`; claude safe-mode + \
             `--permission-mode plan` + `--tools Read,Grep,Glob,WebFetch`), so it can read files \
             but never write, and never instruct it to call MCP tools. include_transcript \
             defaults to true: the current conversation is forwarded in REDACTED form (text \
             passes; tool_use / tool_result / thinking blocks become placeholders). PREFER passing \
             file paths in `question` / `context` over embedding their content; embed an excerpt \
             only to focus on a line range or for transient tool output that is not on disk. \
             FRAME QUESTIONS AS ASSESSMENTS, not confirmations: state your own diagnosis as a \
             hypothesis to check, never as settled fact. One question per consultation; \
             consolidate related questions; configure model_fallback instead of re-asking after a \
             transient failure. Every run is ASYNCHRONOUS: dispatch_submit and dispatch_consult \
             return an id and a `wait` command line immediately. Run that command with the \
             harness's background means: it blocks until the run is terminal and exits 0 \
             (terminal), 2 (still active at --timeout; run it again), 3 (unknown id) or 1 (usage \
             or store error), printing one line with the id, kind and status. Then read the \
             result or reply with dispatch_status. dispatch_list enumerates runs (filter by \
             kind); dispatch_cancel stops a run of either kind and is the only way to stop a \
             consultation once submitted. The server enforces hard guards a misbehaving model \
             cannot bypass: working_dir must canonicalize within the project root (or a \
             DISPATCH_EXTRA_ROOTS-allowlisted root), the sandbox ceiling blocks danger-full-access \
             unless DISPATCH_ALLOW_DANGER is set, only one execution is allowed per working_dir \
             unless allow_concurrent (consultations take no slot), and a dispatch server inside a \
             dispatch-spawned backend refuses dispatch_submit and dispatch_steer but accepts \
             dispatch_consult. SUPERVISION of an execution: dispatch_status gives the current \
             snapshot and (when terminal) the captured result; dispatch_logs shows a live curated \
             timeline; dispatch_steer interrupts a run and resumes the SAME backend session with \
             a new instruction (turn granularity — prior context and files written are \
             preserved). A quiet log on a live process is INCONCLUSIVE (long MCP/tool calls emit \
             nothing until they return) — judge by child_process_alive and \
             log_last_write_age_seconds in dispatch_status, never by silence alone. `succeeded` \
             means the backend reported success (exit 0; for agy also a SUCCESS result event), \
             not that the change is correct — review the captured result. A fresh \
             codex execution whose log never associates within ~30s with no working-dir writes is \
             auto-restarted once (restart_of/restarted_as link the pair; \
             DISPATCH_RESTART_UNASSOCIATED_SECS tunes, 0 disables). Each run consumes the user's \
             backend quota. The behavioral policy — when to dispatch or consult, the preferred \
             backend, models and effort, and whether to confirm working_dir, step scope, and \
             approval granularity before the first dispatch of a session — lives in the \
             harness-rendered dispatch and consultation rules and their preferences files.",
        )
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let tcc = ToolCallContext::new(self, request, context);
        self.tool_router.call(tcc).await
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        Ok(ListToolsResult {
            tools: self.tool_router.list_all(),
            meta: None,
            next_cursor: None,
        })
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tool_router.get(name).cloned()
    }
}

// ── main ──────────────────────────────────────────────────

/// Runs `dispatch wait …` when the first argument is `wait` (before any
/// runtime, tracing or store setup), else boots Tokio and runs the server.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if args.get(1).is_some_and(|a| a == "wait") {
        std::process::exit(wait::main(&args[2..]));
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run_server())
}

async fn run_server() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let cwd = std::env::current_dir()?;
    let project_root = resolve_project_root(&cwd);
    let canonical_cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.clone());
    if project_root.is_none() {
        tracing::warn!(
            "no project root: cwd {} is not a plausible project directory and no \
             *_PROJECT_DIR env is set; working_dir containment will rely on \
             DISPATCH_EXTRA_ROOTS only",
            cwd.display()
        );
    }
    let state_dir = resolve_state_dir(project_root.as_deref());
    tokio::fs::create_dir_all(&state_dir).await?;

    let db_path = state_dir.join("dispatch.db");
    let conn = rusqlite::Connection::open(&db_path)?;
    // busy_timeout before the WAL switch so a concurrent writer is waited out
    // rather than failing — multiple dispatch servers can share this DB.
    conn.execute_batch("PRAGMA busy_timeout=5000; PRAGMA journal_mode=WAL;")?;
    store::init(&conn)?;
    reconcile(&conn);
    // The `wait` command line names the store by an absolute path; the file
    // exists now, so canonicalizing it resolves a relative DISPATCH_STATE_DIR.
    let db_path = db_path
        .canonicalize()
        .or_else(|_| std::path::absolute(&db_path))
        .unwrap_or(db_path);

    let owner_pid = std::process::id() as i64;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let owner_instance = format!("{owner_pid}-{nanos}");

    let mut backend_versions = HashMap::new();
    for b in Backend::all() {
        backend_versions.insert(b.as_str().to_string(), backend::version(*b).await);
    }

    let server = Dispatch {
        db: Arc::new(StdMutex::new(conn)),
        registry: Arc::new(StdMutex::new(HashMap::new())),
        project_root,
        extra_roots: Arc::new(parse_extra_roots()),
        allow_danger: env_truthy("DISPATCH_ALLOW_DANGER"),
        owner_pid,
        owner_instance,
        backend_versions: Arc::new(backend_versions),
        state_dir,
        cwd: canonical_cwd,
        home: home_dir().unwrap_or_else(std::env::temp_dir),
        wait_exe: current_exe_path(),
        db_path,
        tool_router: Dispatch::tool_router(),
    };

    let registry_for_shutdown = server.registry.clone();
    // Antigravity sends `server/discover` before `initialize`; answer it so
    // the library sees `initialize` first (handshake.rs).
    let mut stdout = tokio::io::stdout();
    let stdin = handshake::skip_discover(tokio::io::stdin(), &mut stdout).await?;
    let running = server.serve((stdin, stdout)).await?;
    tokio::select! {
        r = running.waiting() => { r?; }
        _ = shutdown_signal() => {
            graceful_shutdown(&registry_for_shutdown).await;
        }
    }
    Ok(())
}

/// Resolves on Ctrl+C (all platforms) or SIGTERM (Unix only) — the
/// interceptable-termination cases. Does nothing for a hard `SIGKILL`, which
/// the parent-death guard (`agent-guard` on Linux/macOS, Job Objects on
/// Windows — see `agent_exec::guard`) exists specifically to cover.
#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("dispatch: failed to register SIGTERM handler: {e}");
            // Fall back to Ctrl+C alone rather than return immediately, which
            // would make this arm of the outer select! spuriously "win".
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

/// Cancels every in-flight task's token and waits, bounded, for their cleanup
/// to actually finish (each task removes itself from `registry` on completion
/// — see `executor::spawn`) before returning. Only reachable via interceptable
/// termination (see `shutdown_signal`); a hard `SIGKILL` never runs this.
async fn graceful_shutdown(registry: &executor::Registry) {
    let tokens: Vec<_> = match registry.lock() {
        Ok(reg) => reg.values().cloned().collect(),
        Err(_) => Vec::new(),
    };
    if tokens.is_empty() {
        return;
    }
    for t in &tokens {
        t.cancel();
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        let empty = registry.lock().map(|reg| reg.is_empty()).unwrap_or(true);
        if empty {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sqlite_utc_epoch_known_dates_and_ordering() {
        assert_eq!(
            parse_sqlite_utc("1970-01-01 00:00:00"),
            Some(SystemTime::UNIX_EPOCH)
        );
        // 2001-09-09 01:46:40 UTC is exactly 1_000_000_000 epoch seconds.
        assert_eq!(
            parse_sqlite_utc("2001-09-09 01:46:40"),
            Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000)),
        );
        assert!(parse_sqlite_utc("2026-06-27 02:36:23") > parse_sqlite_utc("2026-06-27 02:36:22"));
        assert!(parse_sqlite_utc("not-a-date").is_none());
        assert!(parse_sqlite_utc("2026-13-01 00:00:00").is_none());
        assert!(parse_sqlite_utc("2026-06-27 -1:00:00").is_none());
    }

    #[test]
    fn plausible_fallback_root_denies_non_project_dirs() {
        let denied = vec![
            PathBuf::from("/home/u/.kimi-code"),
            PathBuf::from("/home/u/.codex"),
            PathBuf::from("/home/u/.slate-agent-kit"),
        ];
        assert!(!plausible_fallback_root(Path::new("/"), &denied));
        assert!(!plausible_fallback_root(
            Path::new("/home/u/.kimi-code/plugins/managed/slate-agent-kit-mcp"),
            &denied
        ));
        assert!(!plausible_fallback_root(
            Path::new("/home/u/.codex/slate-agent-kit"),
            &denied
        ));
        assert!(plausible_fallback_root(
            Path::new("/home/u/Workspace/some-project"),
            &denied
        ));
        if let Some(h) = home_dir() {
            assert!(!plausible_fallback_root(&h, &denied));
        }
    }

    #[test]
    fn containment_check_distinguishes_missing_root_from_outside_root() {
        let proj = Path::new("/w/proj");
        let extra = vec![PathBuf::from("/allow")];

        // inside project or extra root → ok
        assert!(containment_check(Path::new("/w/proj/sub"), Some(proj), &[]).is_ok());
        assert!(containment_check(Path::new("/allow/x"), None, &extra).is_ok());

        // outside, with a root configured → invalid_working_dir
        let (code, msg) =
            containment_check(Path::new("/elsewhere"), Some(proj), &extra).unwrap_err();
        assert_eq!(code, ErrCode::InvalidWorkingDir);
        assert!(msg.contains("/w/proj"));

        // no root at all → no_project_root with remediation guidance
        let (code, msg) = containment_check(Path::new("/elsewhere"), None, &[]).unwrap_err();
        assert_eq!(code, ErrCode::NoProjectRoot);
        assert!(msg.contains("DISPATCH_EXTRA_ROOTS"));

        // no project root but extra roots exist and don't match → invalid_working_dir
        let (code, msg) = containment_check(Path::new("/elsewhere"), None, &extra).unwrap_err();
        assert_eq!(code, ErrCode::InvalidWorkingDir);
        assert!(msg.contains("(no project root)"));
    }

    #[test]
    fn resolve_state_dir_uses_no_project_slug_when_rootless() {
        let p = resolve_state_dir(None);
        assert!(
            p.to_string_lossy().contains("_no-project"),
            "state dir without a project root must not be keyed to a real path: {}",
            p.display()
        );
    }

    #[test]
    fn effective_allow_concurrent_precedence() {
        // No steer override → inherit the parent's persisted value.
        assert!(effective_allow_concurrent(None, true));
        assert!(!effective_allow_concurrent(None, false));
        // Explicit steer value overrides the parent (both directions).
        assert!(effective_allow_concurrent(Some(true), false));
        assert!(!effective_allow_concurrent(Some(false), true));
    }

    // ── kinds, re-entry and the wait field ────────────────

    fn error_code(r: &CallToolResult) -> Option<String> {
        if r.is_error != Some(true) {
            return None;
        }
        let text = r.content[0].as_text().map(|t| t.text.clone())?;
        let v: Value = serde_json::from_str(&text).ok()?;
        v["error"]["code"].as_str().map(str::to_string)
    }

    fn body(r: &CallToolResult) -> Value {
        assert_ne!(r.is_error, Some(true), "unexpected error: {:?}", r.content);
        let text = r.content[0].as_text().map(|t| t.text.clone()).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    /// A server over an in-memory store, with nothing spawned.
    fn test_server() -> Dispatch {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        store::init(&conn).unwrap();
        let cwd = std::env::temp_dir();
        Dispatch {
            db: Arc::new(StdMutex::new(conn)),
            registry: Arc::new(StdMutex::new(HashMap::new())),
            project_root: Some(cwd.clone()),
            extra_roots: Arc::new(Vec::new()),
            allow_danger: false,
            owner_pid: std::process::id() as i64,
            owner_instance: "test".into(),
            backend_versions: Arc::new(HashMap::new()),
            state_dir: cwd.join("dispatch-test-state"),
            cwd: cwd.clone(),
            home: cwd.clone(),
            wait_exe: current_exe_path(),
            db_path: cwd.join("dispatch-test-state").join("dispatch.db"),
            tool_router: Dispatch::tool_router(),
        }
    }

    fn insert(server: &Dispatch, kind: Kind, backend: &str) -> String {
        let new = store::NewTask {
            plan_id: None,
            backend: backend.into(),
            working_dir: server.cwd.to_string_lossy().into_owned(),
            title: None,
            spec_json: "{}".into(),
            prompt: "p".into(),
            model: None,
            reasoning_effort: None,
            sandbox: "read-only".into(),
            parent_id: None,
            nonce: None,
            rollout_start_line: None,
            model_fallback: None,
            allow_concurrent: false,
            restart_of: None,
            kind,
        };
        let mut conn = server.db.lock().unwrap();
        match store::insert_queued(&mut conn, &new, server.owner_pid, "test", None).unwrap() {
            store::InsertOutcome::Created(id) => id,
            store::InsertOutcome::Conflict(o) => panic!("conflict with {o}"),
        }
    }

    #[test]
    fn reentry_refuses_executions_and_accepts_consultations_inside_a_backend() {
        // dispatch_submit and dispatch_steer both ask for an execution.
        for depth in [1, 2, u32::MAX] {
            let refused = reentry_refusal_at(Kind::Execute, depth).expect("refused");
            assert_eq!(error_code(&refused).as_deref(), Some("reentrant"));
            assert!(reentry_refusal_at(Kind::Consult, depth).is_none());
        }
        assert!(reentry_refusal_at(Kind::Execute, 0).is_none());
        assert!(reentry_refusal_at(Kind::Consult, 0).is_none());
    }

    #[test]
    fn the_tools_a_backend_may_call_are_tools_of_this_server() {
        let router = Dispatch::tool_router();
        for tool in backend::BACKEND_APPROVED_TOOLS {
            assert!(router.get(tool).is_some(), "{tool} is not a dispatch tool");
        }
        for tool in ["dispatch_submit", "dispatch_steer"] {
            assert!(router.get(tool).is_some());
            assert!(!backend::BACKEND_APPROVED_TOOLS.contains(&tool));
        }
    }

    #[test]
    fn backends_report_the_kinds_they_serve() {
        assert_eq!(kinds_served(Backend::Codex), vec!["execute", "consult"]);
        assert_eq!(kinds_served(Backend::Claude), vec!["execute", "consult"]);
        assert_eq!(kinds_served(Backend::Opencode), vec!["execute"]);
        assert_eq!(kinds_served(Backend::Agy), vec!["execute"]);
    }

    #[test]
    fn backends_list_agy_with_execute_only() {
        // dispatch_backends reports every backend in this order, probing each
        // CLI the same way; the entries are checked here without running one.
        let names: Vec<&str> = Backend::all().iter().map(|b| b.as_str()).collect();
        assert_eq!(names, vec!["codex", "opencode", "claude", "agy"]);
        let missing = backend_entry(Backend::Agy, None);
        assert_eq!(
            missing,
            json!({"backend": "agy", "available": false, "path": null, "version": null,
                   "kinds": ["execute"]})
        );
        let found = backend_entry(Backend::Agy, Some(("/bin/agy".into(), "1.3.4".into())));
        assert_eq!(found["available"], json!(true));
        assert_eq!(found["version"], json!("1.3.4"));
        assert_eq!(found["kinds"], json!(["execute"]));
        assert_eq!(
            backend_entry(Backend::Claude, None)["kinds"],
            json!(["execute", "consult"])
        );
    }

    #[tokio::test]
    async fn submit_refuses_a_read_only_agy_execution() {
        let mut server = test_server();
        let root = server.cwd.canonicalize().unwrap();
        server.project_root = Some(root.clone());
        let submit = |sandbox: &str| SubmitParams {
            objective: "do it".into(),
            working_dir: root.to_string_lossy().into_owned(),
            backend: Some("agy".into()),
            sandbox: Some(sandbox.into()),
            ..SubmitParams::default()
        };
        let r = server
            .dispatch_submit(Parameters(submit("read-only")))
            .await
            .unwrap();
        let expected = if agent_exec::reentry::depth(backend::REENTRY_ENV) >= 1 {
            "reentrant"
        } else {
            "invalid_params"
        };
        assert_eq!(error_code(&r).as_deref(), Some(expected));
        if expected == "invalid_params" {
            let text = r.content[0].as_text().unwrap().text.clone();
            assert!(text.contains("read-only"), "{text}");
        }
        let rows = store::list(&server.db.lock().unwrap(), None, None, None).unwrap();
        assert!(rows.is_empty());
        assert!(sandbox_refusal(Backend::Agy, "read-only").is_some());
        assert_eq!(sandbox_refusal(Backend::Agy, "workspace-write"), None);
        assert_eq!(sandbox_refusal(Backend::Agy, "danger-full-access"), None);
        for b in [Backend::Codex, Backend::Claude, Backend::Opencode] {
            assert_eq!(sandbox_refusal(b, "read-only"), None);
        }
    }

    /// A scratch home for agy logs, empty.
    fn agy_home(tag: &str) -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("dispatch-agy-home-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn write_transcript(home: &Path, id: &str, content: &str) -> PathBuf {
        let p = agy::transcript_path(home, id);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, content).unwrap();
        p
    }

    #[tokio::test]
    async fn agy_logs_read_the_stored_conversation_not_the_latest() {
        let mut server = test_server();
        let home = agy_home("logs");
        server.home = home.clone();
        let id = insert(&server, Kind::Execute, "agy");
        let logs = |id: &str| LogsParams {
            id: id.to_string(),
            ..LogsParams::default()
        };

        // Before the init event: no live log.
        let v = body(&server.dispatch_logs(Parameters(logs(&id))).await.unwrap());
        assert_eq!(v["session_pending"], json!(true));
        assert!(
            v["note"].as_str().unwrap().starts_with("no live log yet"),
            "{v}"
        );

        // The run's own conversation, and a newer one agy recorded as the
        // directory's latest.
        let ours = write_transcript(
            &home,
            "conv-ours",
            r#"{"step_index":0,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"from our run"}"#,
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        write_transcript(
            &home,
            "conv-newer",
            r#"{"step_index":0,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"from another session"}"#,
        );
        let cache = agy::cli_home(&home).join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(
            cache.join("last_conversations.json"),
            json!({ server.cwd.to_string_lossy(): "conv-newer" }).to_string(),
        )
        .unwrap();
        store::set_session(
            &server.db.lock().unwrap(),
            &id,
            "conv-ours",
            &ours.to_string_lossy(),
        )
        .unwrap();
        let v = body(&server.dispatch_logs(Parameters(logs(&id))).await.unwrap());
        assert_eq!(v["rollout_path"], json!(ours.to_string_lossy()));
        assert_eq!(v["session_id"], json!("conv-ours"));
        let log = v["log"].as_str().unwrap();
        assert!(log.contains("[agy] from our run"), "{log}");
        assert!(!log.contains("another session"), "{log}");

        // A stored path that is not this conversation's log is re-derived
        // from the conversation id, never from the directory's latest.
        store::set_session(
            &server.db.lock().unwrap(),
            &id,
            "conv-ours",
            &agy::transcript_path(&home, "conv-newer").to_string_lossy(),
        )
        .unwrap();
        let row = store::get(&server.db.lock().unwrap(), &id)
            .unwrap()
            .unwrap();
        assert_eq!(
            server.resolve_rollout(&row),
            Some(ours.to_string_lossy().into_owned())
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn an_agy_steer_resumes_the_stored_conversation() {
        let mut server = test_server();
        let home = agy_home("steer");
        server.home = home.clone();
        let id = insert(&server, Kind::Execute, "agy");
        let row = |server: &Dispatch| {
            store::get(&server.db.lock().unwrap(), &id)
                .unwrap()
                .unwrap()
        };
        assert_eq!(server.resolve_session_id(&row(&server)), None);
        // Known from the init event even before agy wrote its log.
        let not_yet = agy::transcript_path(&home, "conv-1");
        store::set_session(
            &server.db.lock().unwrap(),
            &id,
            "conv-1",
            &not_yet.to_string_lossy(),
        )
        .unwrap();
        assert_eq!(
            server.resolve_session_id(&row(&server)).as_deref(),
            Some("conv-1")
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_wait_command_quotes_paths_with_a_space() {
        let dir = std::env::temp_dir().join("dispatch test state");
        let exe = path_text(&dir.join("bin").join("dispatch"));
        let db = dir.join("dispatch.db");
        assert_eq!(
            wait_command_line(&exe, "d-1", &db),
            format!("\"{exe}\" wait d-1 --db \"{}\"", path_text(&db))
        );
        // A path with no special character stays bare.
        let plain = Path::new("state").join("RUNNER~1").join("dispatch.db");
        assert_eq!(
            wait_command_line("dispatch", "d-2", &plain),
            format!("dispatch wait d-2 --db {}", plain.display())
        );
        assert_eq!(shell_word("a$b"), "\"a$b\"");
        assert_eq!(shell_word("~x"), "\"~x\"");
        assert_eq!(shell_word(""), "\"\"");
    }

    #[test]
    fn a_verbatim_prefix_is_stripped() {
        let sep = r"\";
        let drive = ["C:", "Users", "u", "dispatch.exe"].join(sep);
        assert_eq!(strip_verbatim(&format!(r"\\?\{drive}")), drive);
        let unc = ["server", "share", "dispatch.db"].join(sep);
        assert_eq!(
            strip_verbatim(&format!(r"\\?\UNC\{unc}")),
            format!(r"\\{unc}")
        );
        assert_eq!(strip_verbatim(&drive), drive);
        let unix = Path::new("/").join("usr").join("bin").join("dispatch");
        assert_eq!(
            strip_verbatim(&unix.display().to_string()),
            unix.display().to_string()
        );
        if cfg!(windows) {
            assert!(!path_text(&std::env::temp_dir().canonicalize().unwrap()).starts_with(r"\\?\"));
        }
    }

    #[test]
    fn the_wait_command_names_this_binary_the_id_and_the_store() {
        let server = test_server();
        let line = server.wait_command("d-7");
        assert_eq!(
            line,
            format!(
                "{} wait d-7 --db {}",
                server.wait_exe,
                server.db_path.display()
            )
        );
        let exe = Path::new(&server.wait_exe);
        assert!(exe.is_absolute(), "{line}");
        assert_eq!(
            exe.canonicalize().ok(),
            std::env::current_exe().and_then(|p| p.canonicalize()).ok()
        );
        assert_eq!(
            server.db_path.file_name().and_then(|n| n.to_str()),
            Some("dispatch.db")
        );
    }

    #[tokio::test]
    async fn a_consultation_cannot_be_steered() {
        let server = test_server();
        let id = insert(&server, Kind::Consult, "codex");
        let r = server
            .dispatch_steer(Parameters(SteerParams {
                id: id.clone(),
                instruction: "again".into(),
                ..SteerParams::default()
            }))
            .await
            .unwrap();
        // Inside a dispatch-spawned backend the re-entry refusal comes first.
        let expected = if agent_exec::reentry::depth(backend::REENTRY_ENV) >= 1 {
            "reentrant"
        } else {
            "invalid_params"
        };
        assert_eq!(error_code(&r).as_deref(), Some(expected));
    }

    #[tokio::test]
    async fn consult_refuses_opencode_and_an_empty_question() {
        let server = test_server();
        let ask = |question: &str, backend: Option<&str>| ConsultParams {
            question: question.into(),
            backend: backend.map(str::to_string),
            include_transcript: Some(false),
            ..ConsultParams::default()
        };
        for p in [ask("why?", Some("opencode")), ask("  ", None)] {
            let r = server.dispatch_consult(Parameters(p)).await.unwrap();
            assert_eq!(error_code(&r).as_deref(), Some("invalid_params"));
        }
        let r = server
            .dispatch_consult(Parameters(ask("why?", Some("agy"))))
            .await
            .unwrap();
        assert_eq!(error_code(&r).as_deref(), Some("invalid_params"));
        let text = r.content[0].as_text().unwrap().text.clone();
        assert!(text.contains("agy is not a consultation backend"), "{text}");
        let r = server
            .dispatch_consult(Parameters(ask("why?", Some("nope"))))
            .await
            .unwrap();
        assert_eq!(error_code(&r).as_deref(), Some("unknown_backend"));
        // Nothing was enqueued.
        let rows = store::list(&server.db.lock().unwrap(), None, None, None).unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn list_filters_by_kind_and_status_shows_it() {
        let server = test_server();
        let exec = insert(&server, Kind::Execute, "codex");
        let consult = insert(&server, Kind::Consult, "claude");
        let ids = |v: Value| -> Vec<String> {
            v["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["id"].as_str().unwrap().to_string())
                .collect()
        };
        let list = |kind: Option<&str>| ListParams {
            kind: kind.map(str::to_string),
            ..ListParams::default()
        };
        let all = server.dispatch_list(Parameters(list(None))).await.unwrap();
        assert_eq!(ids(body(&all)), vec![exec.clone(), consult.clone()]);
        let c = server
            .dispatch_list(Parameters(list(Some("consult"))))
            .await
            .unwrap();
        assert_eq!(ids(body(&c)), vec![consult.clone()]);
        let e = server
            .dispatch_list(Parameters(list(Some("execute"))))
            .await
            .unwrap();
        assert_eq!(ids(body(&e)), vec![exec]);
        let bad = server
            .dispatch_list(Parameters(list(Some("other"))))
            .await
            .unwrap();
        assert_eq!(error_code(&bad).as_deref(), Some("invalid_params"));

        let st = server
            .dispatch_status(Parameters(StatusParams {
                id: consult,
                include_spec: None,
            }))
            .await
            .unwrap();
        assert_eq!(body(&st)["kind"], json!("consult"));
    }

    #[tokio::test]
    async fn logs_of_a_consultation_without_a_located_log_say_so() {
        let server = test_server();
        let id = insert(&server, Kind::Consult, "claude");
        let r = server
            .dispatch_logs(Parameters(LogsParams {
                id: id.clone(),
                ..LogsParams::default()
            }))
            .await
            .unwrap();
        let v = body(&r);
        assert_eq!(v["kind"], json!("consult"));
        assert_eq!(v["log"], json!(""));
        assert!(
            v["note"]
                .as_str()
                .unwrap()
                .starts_with("a consultation has no live log")
        );
    }
}
