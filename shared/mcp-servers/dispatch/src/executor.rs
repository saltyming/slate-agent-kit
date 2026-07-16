//! Detached execution + cancellation registry.
//!
//! `submit` inserts a `queued` row, then calls [`spawn`], which fires a detached
//! tokio task and returns immediately — that is what makes dispatch asynchronous
//! despite MCP being request/response. The task:
//!   1. spawns the backend child (`backend::spawn_child`),
//!   2. transitions the row `queued` → `running` once the child exists (so cancel
//!      / status never race a row with no pid yet),
//!   3. awaits capped capture under a `CancellationToken`,
//!   4. writes the terminal status + captured output back to SQLite,
//!   5. deregisters its token.
//!
//! The connection is a std `Mutex`; every DB touch is a short synchronous helper
//! that locks, writes, and drops the guard — never held across an `.await`, so
//! the task future stays `Send`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime};

use tokio_util::sync::CancellationToken;

use crate::backend::{self, Backend};
use crate::errkind::{self, BackendErrorKind};
use crate::opencode;
use crate::render;
use crate::store;

pub type DbHandle = Arc<StdMutex<rusqlite::Connection>>;
pub type Registry = Arc<StdMutex<HashMap<String, CancellationToken>>>;

/// How many times `record_rollout` polls for the rollout file (×150ms) before giving
/// up — covers codex's spawn → first-write gap. A miss is recoverable later at read
/// time via the stored nonce / session id, so this need not be generous.
const ROLLOUT_LOCATE_ATTEMPTS: usize = 40;

/// Owned, `'static` description of one delegated run (the borrowed
/// `backend::SpawnSpec` is rebuilt from this inside the task).
pub struct Job {
    pub id: String,
    pub backend: Backend,
    pub working_dir: PathBuf,
    pub sandbox: String,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub skip_git_repo_check: bool,
    pub prompt: String,
    pub backend_version: Option<String>,
    pub state_dir: PathBuf,
    /// When set, resume this backend session id instead of starting fresh (the
    /// `dispatch_steer` follow-up path).
    pub resume_session: Option<String>,
    /// Per-task identity marker embedded in the prompt (fresh submits only); lets
    /// `record_rollout` positively match the rollout this run produced. None for resume.
    pub nonce: Option<String>,
    /// For backends with dispatch-owned logs (OpenCode), a resumed task appends to
    /// the parent log instead of discovering an external rollout.
    pub rollout_path: Option<PathBuf>,
    /// Ordered fallback chain: on a transient backend error (per `errkind::classify`),
    /// retry the SAME task against the next model here. None/empty for
    /// `dispatch_steer` follow-ups — a resumed session stays on one model.
    pub model_fallback: Option<Vec<String>>,
    /// Mirrors the row's allow_concurrent. An allow_concurrent run is never
    /// auto-restarted: with other writers active in the same tree, "no writes
    /// since spawn" cannot be attributed to THIS run.
    pub allow_concurrent: bool,
    /// Set when this job IS an auto-restart successor — a restarted run never
    /// restarts again (single-shot, no loop).
    pub restart_of: Option<String>,
}

/// Register a cancellation token for `job.id` and fire the detached run task.
/// If the run ends by auto-restarting itself (unassociated-log watchdog), the
/// successor job is spawned here under its own fresh id + token — exactly one
/// hop, since a successor carries `restart_of` and is never restarted again.
pub fn spawn(db: DbHandle, registry: Registry, job: Job) {
    let ct = CancellationToken::new();
    if let Ok(mut reg) = registry.lock() {
        reg.insert(job.id.clone(), ct.clone());
    }
    let db2 = db.clone();
    let reg2 = registry.clone();
    tokio::spawn(async move {
        let successor = run(&db2, &job, &ct).await;
        if let Ok(mut reg) = reg2.lock() {
            reg.remove(&job.id);
        }
        if let Some(next) = successor {
            spawn(db2, reg2, next);
        }
    });
}

/// Request cancellation of a running task. Returns true if a live token was
/// found and fired (the task writes `cancelled` and deregisters itself).
pub fn request_cancel(registry: &Registry, id: &str) -> bool {
    if let Ok(reg) = registry.lock()
        && let Some(tok) = reg.get(id)
    {
        tok.cancel();
        return true;
    }
    false
}

/// One fallback attempt's classified failure, kept for `fallback_history`.
struct AttemptRecord {
    model: String,
    kind: BackendErrorKind,
    detail: String,
}

/// What one backend invocation produced: a normal outcome to record, or an
/// auto-restart — the row is already terminal (`interrupted`) and the successor
/// job (if any) must be spawned under its own id.
enum AttemptResult {
    Outcome(backend::RunOutcome),
    Restarted(Option<Box<Job>>),
}

/// Retry loop: try `job.model`, then each entry in `job.model_fallback` in
/// order, on a transient backend error (`errkind::classify`). Reuses the same
/// task row/id for every attempt — never re-enters `insert_queued` (the row
/// is already `running`, and the one-run-per-`working_dir` guard lives only
/// at insert time; a retry that resubmitted would trip its own prior
/// attempt's `dir_busy`).
///
/// Returns the auto-restart successor job when the unassociated-log watchdog
/// fired (the caller spawns it); `None` on every normal terminal path.
async fn run(db: &DbHandle, job: &Job, ct: &CancellationToken) -> Option<Job> {
    if ct.is_cancelled() {
        db_finish(
            db,
            &job.id,
            store::STATUS_CANCELLED,
            None,
            None,
            Some("cancelled before the backend started"),
        );
        return None;
    }

    let attempts: Vec<Option<String>> = std::iter::once(job.model.clone())
        .chain(job.model_fallback.iter().flatten().cloned().map(Some))
        .collect();
    let last_idx = attempts.len() - 1;
    let mut history: Vec<AttemptRecord> = Vec::new();

    for (idx, model) in attempts.iter().enumerate() {
        if ct.is_cancelled() {
            db_finish(
                db,
                &job.id,
                store::STATUS_CANCELLED,
                None,
                None,
                Some("cancelled before the backend started"),
            );
            return None;
        }

        let outcome = match run_attempt(db, job, model.as_deref(), idx, ct).await {
            // The watchdog already wrote this row's terminal state; nothing to
            // finish here — hand the successor up for spawning.
            AttemptResult::Restarted(successor) => return successor.map(|b| *b),
            AttemptResult::Outcome(o) => o,
        };

        if matches!(outcome, backend::RunOutcome::Cancelled) {
            finish_with_history(db, &job.id, outcome, model.as_deref(), &history);
            return None;
        }

        if let Some(text) = failure_text(job.backend, &outcome) {
            let kind = errkind::classify(&text);
            tracing::info!(
                "dispatch: {} attempt {}/{} model={:?} failed kind={} detail={:.200}",
                job.id,
                idx + 1,
                attempts.len(),
                model,
                kind.as_str(),
                text
            );
            if kind.is_retry_worthy() && idx != last_idx {
                history.push(AttemptRecord {
                    model: model.clone().unwrap_or_else(|| "(backend default)".into()),
                    kind,
                    detail: text,
                });
                continue;
            }
        }
        finish_with_history(db, &job.id, outcome, model.as_deref(), &history);
        return None;
    }
    None
}

/// One backend invocation for `model` (the `idx`-th attempt) — the
/// pre-refactor body of `run`, extracted so the retry loop can call it once
/// per fallback-chain entry. Each attempt mints its OWN nonce and takes its
/// own pre-spawn rollout snapshot (see the doc comment on the nonce-swap
/// below) rather than reusing attempt 0's — reusing one nonce across attempts
/// was an earlier design that turned out to be an actual bug: `locate_by_nonce`
/// has no snapshot/floor exclusion and returns on the first match, so a
/// retry's `record_rollout` could transiently match a still-on-disk PRIOR
/// attempt's (already-failed) rollout before its own newer one appears.
async fn run_attempt(
    db: &DbHandle,
    job: &Job,
    model: Option<&str>,
    idx: usize,
    ct: &CancellationToken,
) -> AttemptResult {
    // Attempt 0 keeps the job's own nonce/prompt exactly as submitted (no
    // swap needed); a fallback retry (idx > 0) mints a fresh nonce and swaps
    // it into a copy of the prompt, so its own record_rollout call can never
    // match an earlier attempt's still-on-disk rollout.
    let (attempt_nonce, attempt_prompt): (Option<String>, std::borrow::Cow<'_, str>) =
        if idx == 0 || job.nonce.is_none() {
            (
                job.nonce.clone(),
                std::borrow::Cow::Borrowed(job.prompt.as_str()),
            )
        } else {
            let base = job.nonce.as_deref().unwrap_or("");
            let new_nonce = format!("{base}-retry{idx}");
            let swapped = job.prompt.replace(
                &render::nonce_marker(base),
                &render::nonce_marker(&new_nonce),
            );
            (Some(new_nonce), std::borrow::Cow::Owned(swapped))
        };

    if job.backend == Backend::Opencode {
        let ospec = opencode::RunSpec {
            id: &job.id,
            working_dir: &job.working_dir,
            sandbox: &job.sandbox,
            model,
            reasoning_effort: job.reasoning_effort.as_deref(),
            prompt: &attempt_prompt,
            state_dir: &job.state_dir,
            backend_version: job.backend_version.as_deref(),
            resume_session: job.resume_session.as_deref(),
            rollout_path: job.rollout_path.as_deref(),
        };
        return AttemptResult::Outcome(opencode::run(db, ospec, ct).await);
    }

    if job.backend == Backend::Claude {
        // Pin a fresh session id per attempt: the session log path under
        // ~/.claude/projects/ becomes deterministic (no discovery/polling),
        // and a fallback retry can never collide with a prior attempt's
        // half-created session. A steered run passes the parent session via
        // --resume … --fork-session and still pins its own new id.
        let pin = uuid::Uuid::new_v4().to_string();
        let spec = backend::SpawnSpec {
            working_dir: &job.working_dir,
            sandbox: &job.sandbox,
            model,
            reasoning_effort: job.reasoning_effort.as_deref(),
            skip_git_repo_check: job.skip_git_repo_check,
            resume_session: job.resume_session.as_deref(),
            pin_session: Some(&pin),
        };
        let spawned = match backend::spawn_child(job.backend, &spec, &attempt_prompt) {
            Ok(s) => s,
            Err(e) => return AttemptResult::Outcome(backend::RunOutcome::WaitFailed(e)),
        };
        let child_pid = spawned.child_pid.map(|p| p as i64);
        let argv_json = serde_json::to_string(&spawned.argv).unwrap_or_default();
        db_mark_running(
            db,
            &job.id,
            child_pid,
            &argv_json,
            job.backend_version.as_deref(),
        );
        let path = crate::rollout::claude_session_path(&job.working_dir, &pin);
        if let Ok(conn) = db.lock()
            && let Err(e) = store::set_session(&conn, &job.id, &pin, &path.to_string_lossy())
        {
            tracing::warn!("dispatch: set_session({}) failed: {e}", job.id);
        }
        return AttemptResult::Outcome(backend::capture(spawned, ct).await);
    }

    let spec = backend::SpawnSpec {
        working_dir: &job.working_dir,
        sandbox: &job.sandbox,
        model,
        reasoning_effort: job.reasoning_effort.as_deref(),
        skip_git_repo_check: job.skip_git_repo_check,
        resume_session: job.resume_session.as_deref(),
        pin_session: None,
    };

    // Snapshot the rollouts that already exist BEFORE spawning THIS attempt,
    // so the association loop can require a file that did not exist yet —
    // never a pre-existing same-cwd session, including a prior fallback
    // attempt's own (already-failed) rollout.
    let snapshot = crate::rollout::session_snapshot();
    let spawn_time = SystemTime::now();

    let mut spawned = match backend::spawn_child(job.backend, &spec, &attempt_prompt) {
        Ok(s) => s,
        Err(e) => return AttemptResult::Outcome(backend::RunOutcome::WaitFailed(e)),
    };

    let child_pid = spawned.child_pid.map(|p| p as i64);
    let argv_json = serde_json::to_string(&spawned.argv).unwrap_or_default();
    db_mark_running(
        db,
        &job.id,
        child_pid,
        &argv_json,
        job.backend_version.as_deref(),
    );

    // Associate the rollout codex just created (path + session id) so
    // dispatch_logs can tail it live and dispatch_steer can resume the
    // session — and, for an eligible fresh submit that stays unassociated
    // past the configured window with no writes under working_dir, kill the
    // run and hand back a fresh successor (the auto-restart watchdog).
    match associate_or_restart(
        db,
        job,
        &mut spawned,
        attempt_nonce.as_deref(),
        &snapshot,
        spawn_time,
        ct,
    )
    .await
    {
        WatchdogVerdict::Proceed => AttemptResult::Outcome(backend::capture(spawned, ct).await),
        WatchdogVerdict::Restarted(successor) => AttemptResult::Restarted(successor),
    }
}

/// Extract the text `errkind::classify` should judge from a non-success
/// outcome. `None` for a success or a cancellation (neither is a failure to
/// classify — cancellation is handled separately, before this is called).
///
/// The `claude` CLI reports its discriminating failure text (e.g. the
/// bad-model message, captured live) on **stdout**, not stderr — stderr
/// carries only incidental warnings — so its stdout tail joins the
/// classification text. Other backends stay stderr-only to avoid false
/// pattern hits in verbose agent stdout.
fn failure_text(backend: Backend, outcome: &backend::RunOutcome) -> Option<String> {
    match outcome {
        backend::RunOutcome::Done {
            success: false,
            stdout,
            stderr,
            exit_code,
            ..
        } => {
            let mut text = format!("exit_code={:?} stderr={}", exit_code, stderr);
            if backend == Backend::Claude {
                let tail: String = stdout
                    .chars()
                    .rev()
                    .take(2000)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                text.push_str(" stdout=");
                text.push_str(&tail);
            }
            Some(text)
        }
        backend::RunOutcome::WaitFailed(e) => Some(e.clone()),
        backend::RunOutcome::Done { success: true, .. } | backend::RunOutcome::Cancelled => None,
    }
}

/// Write the terminal outcome via the existing `finish_outcome` (unchanged),
/// then — only when at least one fallback attempt was tried and discarded —
/// record which model actually produced the result plus the discarded
/// attempts' history via the narrow `store::set_fallback_result` addition.
fn finish_with_history(
    db: &DbHandle,
    id: &str,
    outcome: backend::RunOutcome,
    final_model: Option<&str>,
    history: &[AttemptRecord],
) {
    finish_outcome(db, id, outcome);
    if !history.is_empty() {
        let history_json = serde_json::to_string(
            &history
                .iter()
                .map(|a| {
                    serde_json::json!({
                        "model": a.model,
                        "error_kind": a.kind.as_str(),
                        "detail": a.detail.chars().take(2000).collect::<String>(),
                    })
                })
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".into());
        db_set_fallback_result(db, id, final_model, &history_json);
    }
}

fn db_set_fallback_result(db: &DbHandle, id: &str, final_model: Option<&str>, history_json: &str) {
    match db.lock() {
        Ok(conn) => {
            if let Err(e) = store::set_fallback_result(&conn, id, final_model, history_json) {
                tracing::warn!("dispatch: set_fallback_result({id}) failed: {e}");
            }
        }
        Err(e) => tracing::warn!("dispatch: db lock poisoned in set_fallback_result: {e}"),
    }
}

fn finish_outcome(db: &DbHandle, id: &str, outcome: backend::RunOutcome) {
    match outcome {
        backend::RunOutcome::Done {
            exit_code,
            success,
            stdout,
            stdout_total,
            stdout_truncated,
            stderr,
            stderr_truncated,
        } => {
            let status = if success {
                store::STATUS_SUCCEEDED
            } else {
                store::STATUS_FAILED
            };
            let result = build_result(&stdout, stdout_total, stdout_truncated);
            let error = if success {
                None
            } else {
                Some(build_error(&stderr, stderr_truncated, exit_code))
            };
            db_finish(
                db,
                id,
                status,
                exit_code.map(|c| c as i64),
                Some(&result),
                error.as_deref(),
            );
        }
        backend::RunOutcome::Cancelled => {
            db_finish(
                db,
                id,
                store::STATUS_CANCELLED,
                None,
                None,
                Some("cancelled by request; the backend process group was killed"),
            );
        }
        backend::RunOutcome::WaitFailed(e) => {
            db_finish(db, id, store::STATUS_FAILED, None, None, Some(&e));
        }
    }
}

/// How the association/watchdog loop ended: proceed to capture the running
/// child, or the child was killed and (maybe) re-submitted as a fresh task.
enum WatchdogVerdict {
    Proceed,
    Restarted(Option<Box<Job>>),
}

/// Interval between association attempts.
const ASSOC_POLL_MS: u64 = 150;

/// Default auto-restart window for a fresh codex submit whose rollout never
/// associates. Overridable via `DISPATCH_RESTART_UNASSOCIATED_SECS` (0 = off).
const RESTART_DEFAULT_SECS: u64 = 30;

/// Cap on entries visited by `dir_changed_since` — past this the scan is
/// inconclusive (None) and the restart is skipped rather than risked.
const DIR_SCAN_CAP: usize = 20_000;

/// Locate the rollout this run produced and persist its path + session id (so
/// dispatch_logs can tail it and dispatch_steer can resume). codex writes the
/// rollout at startup, so polling covers the spawn → first-write gap. The match
/// is **positive**, never "newest same-cwd": a resume run is found by its
/// already-known session id; a fresh run by its prompt nonce (falling back to
/// the pre-spawn snapshot diff only when there is no nonce). The DB lock is
/// held only for the brief `set_session`, never across a sleep.
///
/// Watchdog: when an ELIGIBLE run (fresh codex submit — has a nonce, not a
/// resume, not itself a restart, not allow_concurrent) is still unassociated
/// past the restart window AND nothing under working_dir changed since spawn,
/// the child is killed and re-submitted once as a fresh task (`restart_of`
/// linkage). Detection is a best-effort mtime scan — the accepted residual
/// risk (documented in the dispatch rule) is killing a healthy run whose
/// rollout the locator failed to find and that had not yet written anything.
async fn associate_or_restart(
    db: &DbHandle,
    job: &Job,
    spawned: &mut backend::Spawned,
    nonce: Option<&str>,
    snapshot: &HashSet<PathBuf>,
    spawn_time: SystemTime,
    ct: &CancellationToken,
) -> WatchdogVerdict {
    let restart_after = if job.restart_of.is_none()
        && job.resume_session.is_none()
        && job.nonce.is_some()
        && !job.allow_concurrent
    {
        restart_after_secs()
    } else {
        None
    };
    let assoc_budget = Duration::from_millis(ASSOC_POLL_MS * ROLLOUT_LOCATE_ATTEMPTS as u64);
    let start = tokio::time::Instant::now();

    loop {
        // A cancelled or already-exited child is capture's business — it kills /
        // reaps and reports; the watchdog must never eat a real failure outcome.
        if ct.is_cancelled() {
            return WatchdogVerdict::Proceed;
        }
        if let Ok(Some(_)) = spawned.child.try_wait() {
            return WatchdogVerdict::Proceed;
        }

        let found = match job.resume_session.as_deref() {
            // Resume: the session id is known — locate its rollout deterministically
            // by id (codex appends the new turn to the same file).
            Some(sid) => crate::rollout::locate_by_session_id(sid).map(|p| (p, sid.to_string())),
            // Fresh: prefer the nonce (positive identity, robust to a concurrent
            // same-cwd codex); fall back to the snapshot diff only when there's no nonce.
            None => match nonce {
                Some(n) => crate::rollout::locate_by_nonce(&job.working_dir, n),
                None => crate::rollout::locate_new_by_cwd(&job.working_dir, snapshot, None),
            },
        };
        if let Some((path, sid)) = found {
            if let Ok(conn) = db.lock()
                && let Err(e) = store::set_session(&conn, &job.id, &sid, &path.to_string_lossy())
            {
                tracing::warn!("dispatch: set_session({}) failed: {e}", job.id);
            }
            return WatchdogVerdict::Proceed;
        }

        let elapsed = start.elapsed();
        match restart_after {
            Some(window) if elapsed >= window => {
                return try_restart(db, job, spawned, spawn_time, window).await;
            }
            Some(_) => {}
            None if elapsed >= assoc_budget => {
                tracing::warn!("dispatch: could not locate codex rollout for {}", job.id);
                return WatchdogVerdict::Proceed;
            }
            None => {}
        }
        tokio::time::sleep(Duration::from_millis(ASSOC_POLL_MS)).await;
    }
}

/// The restart window fired: verify nothing under working_dir changed, then
/// kill + reap the child, terminal-mark the row (conditionally — a racing
/// cancel wins), and insert + return the fresh successor task.
async fn try_restart(
    db: &DbHandle,
    job: &Job,
    spawned: &mut backend::Spawned,
    spawn_time: SystemTime,
    window: Duration,
) -> WatchdogVerdict {
    match dir_changed_since(&job.working_dir, spawn_time) {
        Some(false) => {}
        verdict => {
            tracing::info!(
                "dispatch: {} unassociated past {}s but working_dir scan says {} — not restarting",
                job.id,
                window.as_secs(),
                match verdict {
                    Some(true) => "changed",
                    Some(false) => unreachable!(),
                    None => "inconclusive",
                },
            );
            return WatchdogVerdict::Proceed;
        }
    }

    // Kill while the pid is unambiguously ours, then reap — same order as
    // capture's cancel path. After this the child cannot produce an outcome.
    if let Some(pid) = spawned.child_pid {
        backend::kill_process_group(pid);
    }
    let _ = spawned.child.wait().await;

    let reason = format!(
        "auto-restarted: no backend log associated within {}s and no writes detected under the \
         working directory — the process was killed and the task re-submitted fresh (the \
         successor carries restart_of={}). Tune or disable with \
         DISPATCH_RESTART_UNASSOCIATED_SECS (0 = off).",
        window.as_secs(),
        job.id,
    );
    let marked = match db.lock() {
        Ok(conn) => store::mark_interrupted(&conn, &job.id, &reason).unwrap_or(0),
        Err(_) => 0,
    };
    if marked != 1 {
        // A racing terminal write (cancel) got there first — its verdict stands.
        tracing::warn!(
            "dispatch: {} was already terminal when the auto-restart fired; no successor",
            job.id
        );
        return WatchdogVerdict::Restarted(None);
    }

    let Some(old) = (match db.lock() {
        Ok(conn) => store::get(&conn, &job.id).ok().flatten(),
        Err(_) => None,
    }) else {
        return WatchdogVerdict::Restarted(None);
    };

    // Fresh identity: new nonce swapped into the prompt (same mechanism as a
    // fallback retry), so the successor's association can never match this
    // attempt's rollout should it surface later.
    let base = job.nonce.as_deref().unwrap_or_default();
    let new_nonce = format!("{base}-restart");
    let new_prompt = job.prompt.replace(
        &render::nonce_marker(base),
        &render::nonce_marker(&new_nonce),
    );

    let new_task = store::NewTask {
        plan_id: old.plan_id.clone(),
        backend: old.backend.clone(),
        working_dir: old.working_dir.clone(),
        title: old
            .title
            .clone()
            .or_else(|| Some(format!("restart of {}", job.id))),
        spec_json: old.spec_json.clone(),
        prompt: new_prompt.clone(),
        model: old.model.clone(),
        reasoning_effort: old.reasoning_effort.clone(),
        sandbox: old.sandbox.clone(),
        parent_id: None,
        nonce: Some(new_nonce.clone()),
        rollout_start_line: None,
        model_fallback: old.model_fallback.clone(),
        allow_concurrent: old.allow_concurrent,
        restart_of: Some(job.id.clone()),
    };
    let inserted = match db.lock() {
        Ok(mut conn) => store::insert_queued(
            &mut conn,
            &new_task,
            old.owner_pid.unwrap_or_else(|| std::process::id() as i64),
            old.owner_instance.as_deref().unwrap_or(""),
            Some(old.working_dir.as_str()),
        ),
        Err(e) => {
            tracing::warn!("dispatch: db lock poisoned inserting restart successor: {e}");
            return WatchdogVerdict::Restarted(None);
        }
    };
    let new_id = match inserted {
        Ok(store::InsertOutcome::Created(id)) => id,
        Ok(store::InsertOutcome::Conflict(existing)) => {
            tracing::warn!(
                "dispatch: restart of {} aborted — {existing} became active for the dir first",
                job.id
            );
            return WatchdogVerdict::Restarted(None);
        }
        Err(e) => {
            tracing::warn!("dispatch: restart insert for {} failed: {e}", job.id);
            return WatchdogVerdict::Restarted(None);
        }
    };
    tracing::info!("dispatch: {} auto-restarted as {new_id}", job.id);

    WatchdogVerdict::Restarted(Some(Box::new(Job {
        id: new_id,
        backend: job.backend,
        working_dir: job.working_dir.clone(),
        sandbox: job.sandbox.clone(),
        model: job.model.clone(),
        reasoning_effort: job.reasoning_effort.clone(),
        skip_git_repo_check: job.skip_git_repo_check,
        prompt: new_prompt,
        backend_version: job.backend_version.clone(),
        state_dir: job.state_dir.clone(),
        resume_session: None,
        nonce: Some(new_nonce),
        rollout_path: None,
        model_fallback: job.model_fallback.clone(),
        allow_concurrent: job.allow_concurrent,
        restart_of: Some(job.id.clone()),
    })))
}

/// Auto-restart window from `DISPATCH_RESTART_UNASSOCIATED_SECS`: unset →
/// default (30s); `0` → disabled; unparsable → default, with a warning.
fn restart_after_secs() -> Option<Duration> {
    parse_restart_secs(
        std::env::var("DISPATCH_RESTART_UNASSOCIATED_SECS")
            .ok()
            .as_deref(),
    )
}

/// Pure core of `restart_after_secs`, split out for testing.
fn parse_restart_secs(raw: Option<&str>) -> Option<Duration> {
    let raw = match raw {
        None => return Some(Duration::from_secs(RESTART_DEFAULT_SECS)),
        Some(s) if s.trim().is_empty() => return Some(Duration::from_secs(RESTART_DEFAULT_SECS)),
        Some(s) => s.trim(),
    };
    match raw.parse::<u64>() {
        Ok(0) => None,
        Ok(n) => Some(Duration::from_secs(n)),
        Err(_) => {
            tracing::warn!(
                "dispatch: DISPATCH_RESTART_UNASSOCIATED_SECS={raw:?} is not a number; \
                 using the {RESTART_DEFAULT_SECS}s default"
            );
            Some(Duration::from_secs(RESTART_DEFAULT_SECS))
        }
    }
}

/// Best-effort "did anything under `dir` change since `since`?" — a bounded
/// mtime walk. Skips `.git` (VCS bookkeeping; the accepted blind spot is a
/// commit-only change), never follows symlinks, caps at `DIR_SCAN_CAP`
/// entries. `Some(true)` = a change was seen, `Some(false)` = the scan
/// completed clean, `None` = inconclusive (unreadable entry / cap hit) — the
/// caller treats anything but `Some(false)` as "do not restart".
fn dir_changed_since(dir: &Path, since: SystemTime) -> Option<bool> {
    // A 2s epsilon absorbs coarse filesystem mtime granularity; it errs toward
    // "changed", which safely skips the restart.
    let floor = since.checked_sub(Duration::from_secs(2))?;
    let mut seen = 0usize;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = std::fs::read_dir(&d).ok()?;
        for e in entries {
            let e = e.ok()?;
            seen += 1;
            if seen > DIR_SCAN_CAP {
                return None;
            }
            let ft = e.file_type().ok()?;
            if ft.is_dir() {
                if e.file_name() == ".git" {
                    continue;
                }
                stack.push(e.path());
            }
            // The entry's own mtime: covers file writes, and a directory's
            // mtime flags entry creation/removal inside it.
            if let Ok(md) = e.metadata()
                && let Ok(m) = md.modified()
                && m >= floor
            {
                return Some(true);
            }
        }
    }
    Some(false)
}

fn build_result(stdout: &str, total: usize, truncated: bool) -> String {
    if truncated {
        format!(
            "{stdout}\n\n[stdout truncated; captured first {} of {} bytes]",
            stdout.len(),
            total
        )
    } else {
        stdout.to_string()
    }
}

fn build_error(stderr: &str, truncated: bool, exit_code: Option<i32>) -> String {
    let mut s = String::new();
    if let Some(c) = exit_code {
        s.push_str(&format!("exit code {c}\n"));
    }
    if stderr.is_empty() {
        s.push_str("(no stderr captured)");
    } else {
        s.push_str(stderr);
        if truncated {
            s.push_str("\n[stderr truncated]");
        }
    }
    s
}

// ── DB helpers: lock briefly, write, drop the guard before any await ──

fn db_mark_running(db: &DbHandle, id: &str, pid: Option<i64>, argv: &str, ver: Option<&str>) {
    match db.lock() {
        Ok(conn) => {
            if let Err(e) = store::mark_running(&conn, id, pid, argv, ver) {
                tracing::warn!("dispatch: mark_running({id}) failed: {e}");
            }
        }
        Err(e) => tracing::warn!("dispatch: db lock poisoned in mark_running: {e}"),
    }
}

fn db_finish(
    db: &DbHandle,
    id: &str,
    status: &str,
    exit_code: Option<i64>,
    result: Option<&str>,
    error: Option<&str>,
) {
    match db.lock() {
        Ok(conn) => {
            if let Err(e) = store::finish(&conn, id, status, exit_code, result, error) {
                tracing::warn!("dispatch: finish({id}) failed: {e}");
            }
        }
        Err(e) => tracing::warn!("dispatch: db lock poisoned in finish: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_restart_secs_defaults_and_disables() {
        let default = Some(Duration::from_secs(RESTART_DEFAULT_SECS));
        assert_eq!(parse_restart_secs(None), default);
        assert_eq!(parse_restart_secs(Some("")), default);
        assert_eq!(parse_restart_secs(Some("  ")), default);
        assert_eq!(
            parse_restart_secs(Some("0")),
            None,
            "0 disables the watchdog"
        );
        assert_eq!(
            parse_restart_secs(Some("45")),
            Some(Duration::from_secs(45))
        );
        assert_eq!(
            parse_restart_secs(Some(" 45 ")),
            Some(Duration::from_secs(45))
        );
        assert_eq!(
            parse_restart_secs(Some("abc")),
            default,
            "unparsable falls back to the default rather than silently disabling"
        );
    }

    #[test]
    fn dir_changed_since_detects_fresh_writes_and_skips_git() {
        let root = std::env::temp_dir().join(format!("dispatch-dirscan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub").join("a.txt"), "x").unwrap();

        let past = SystemTime::now() - Duration::from_secs(60);
        let future = SystemTime::now() + Duration::from_secs(60);
        assert_eq!(
            dir_changed_since(&root, past),
            Some(true),
            "files created now are newer than a spawn 60s ago"
        );
        assert_eq!(
            dir_changed_since(&root, future),
            Some(false),
            "nothing can be newer than a spawn in the future"
        );

        // .git internals are a deliberate blind spot — VCS bookkeeping only.
        let git_only =
            std::env::temp_dir().join(format!("dispatch-dirscan-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&git_only);
        std::fs::create_dir_all(git_only.join(".git")).unwrap();
        std::fs::write(git_only.join(".git").join("HEAD"), "ref").unwrap();
        assert_eq!(
            dir_changed_since(&git_only, past),
            Some(false),
            ".git internals must not count as working-dir writes"
        );

        assert_eq!(
            dir_changed_since(&root.join("missing"), past),
            None,
            "an unreadable root is inconclusive, never a clean verdict"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&git_only);
    }
}
