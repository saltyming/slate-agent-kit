//! Detached execution, the cancellation registry, and the one place a run's
//! events become store writes.
//!
//! `submit` inserts a `queued` row, then calls [`spawn`], which fires a detached
//! tokio task and returns immediately — that is what makes dispatch asynchronous
//! despite MCP being request/response. The task runs the job's model chain —
//! codex and claude through `agent_exec::run_with_fallback`, opencode through
//! `opencode::run` in `opencode_chain` — and consumes the `RunEvent`s they send
//! (`drive`):
//!   1. before each attempt's spawn, `AttemptBuilder` prepares the attempt's
//!      identity (a fresh nonce for a retry, a pinned claude session id, the
//!      codex rollout snapshot);
//!   2. `Started` transitions the row `queued` → `running` (pid, argv, backend
//!      version); for claude it records the pinned session, for codex it starts
//!      the attempt's association watchdog;
//!   3. `Session` (opencode) records the session and its dispatch-owned log;
//!   4. the chain's final outcome is written back as the terminal status, with
//!      the fallback history when attempts were discarded;
//!   5. the task deregisters its token.
//!
//! The watchdog stops a codex attempt whose rollout never associates by
//! cancelling the attempt's own child token, which the user's `dispatch_cancel`
//! token does not share, so a restart (`interrupted` plus a successor task) and a
//! user cancel (`cancelled`) stay distinguishable.
//!
//! The connection is a std `Mutex`; every DB touch is a short synchronous helper
//! that locks, writes, and drops the guard — never held across an `.await`, so
//! the task future stays `Send`.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime};

use agent_exec::{
    AttemptResult, DiscardedAttempt, FailureTextPolicy, FallbackOutcome, Outcome, RunEvent,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_util::sync::CancellationToken;

use crate::backend::{self, Backend};
use crate::liveness;
use crate::opencode;
use crate::render;
use crate::rollout;
use crate::store;

pub type DbHandle = Arc<StdMutex<rusqlite::Connection>>;
pub type Registry = Arc<StdMutex<HashMap<String, CancellationToken>>>;

/// How many times the association loop polls for the rollout file (×150ms)
/// before giving up — covers codex's spawn → first-write gap. A miss is
/// recoverable later at read time via the stored nonce / session id, so this
/// need not be generous.
const ROLLOUT_LOCATE_ATTEMPTS: usize = 40;

/// How many characters of a discarded attempt's failure text are kept.
const DISCARDED_DETAIL_CHARS: usize = 2000;

/// Owned, `'static` description of one delegated run.
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
    /// the association loop positively match the rollout this run produced. None
    /// for resume.
    pub nonce: Option<String>,
    /// For backends with dispatch-owned logs (OpenCode), a resumed task appends to
    /// the parent log instead of discovering an external rollout.
    pub rollout_path: Option<PathBuf>,
    /// Ordered fallback chain: on a transient backend error (per
    /// `agent_exec::errkind::classify`), retry the SAME task against the next
    /// model here. None/empty for `dispatch_steer` follow-ups — a resumed session
    /// stays on one model.
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

/// Run the job's model chain: `job.model`, then each entry of
/// `job.model_fallback` in order on a transient backend error. Reuses the same
/// task row/id for every attempt — never re-enters `insert_queued` (the row
/// is already `running`, and the one-run-per-`working_dir` guard lives only
/// at insert time; a retry that resubmitted would trip its own prior
/// attempt's `dir_busy`).
///
/// Returns the auto-restart successor job when the unassociated-log watchdog
/// fired (the caller spawns it); `None` on every normal terminal path.
async fn run(db: &DbHandle, job: &Job, ct: &CancellationToken) -> Option<Job> {
    let current = StdMutex::new(Attempt::default());
    let builder = AttemptBuilder {
        db,
        job,
        current: &current,
        snapshot: rollout::session_snapshot,
    };
    let models = attempt_models(job);
    let attempt_ct = ct.child_token();
    let (tx, rx) = unbounded_channel();
    let chain: Pin<Box<dyn Future<Output = FallbackOutcome> + Send + '_>> =
        if job.backend.process().is_some() {
            Box::pin(agent_exec::run_with_fallback(
                |idx, model| builder.build(idx, model),
                &models,
                &tx,
                &attempt_ct,
            ))
        } else {
            Box::pin(opencode_chain(&builder, &models, &tx, &attempt_ct))
        };
    let cx = Consumer {
        db,
        job,
        ct,
        attempt_ct: &attempt_ct,
        current: &current,
        restart_after: restart_window(job),
        locate: locate_rollout,
    };
    cx.drive(chain, rx).await
}

/// The models of the chain in order: the primary (`None` = backend default),
/// then the fallback entries.
fn attempt_models(job: &Job) -> Vec<Option<String>> {
    std::iter::once(job.model.clone())
        .chain(job.model_fallback.iter().flatten().cloned().map(Some))
        .collect()
}

/// The auto-restart window for the job, or `None` when the job is not
/// eligible: only a fresh codex submit (has a nonce, not a resume, not itself
/// a restart, not allow_concurrent) is restarted, and only when the window is
/// enabled.
fn restart_window(job: &Job) -> Option<Duration> {
    if job.backend == Backend::Codex
        && job.restart_of.is_none()
        && job.resume_session.is_none()
        && job.nonce.is_some()
        && !job.allow_concurrent
    {
        restart_after_secs()
    } else {
        None
    }
}

// ── attempt preparation ───────────────────────────────────

/// The identity of the attempt being run, written by [`AttemptBuilder`] before
/// the spawn and read by the consumer when the attempt reports `Started`.
#[derive(Debug, Default)]
struct Attempt {
    /// The nonce the attempt's prompt embeds.
    nonce: Option<String>,
    /// Claude: the attempt's pinned session id.
    pin: Option<String>,
    /// Codex: the rollouts that existed before the spawn.
    snapshot: HashSet<PathBuf>,
    /// When the attempt was prepared, just before its spawn.
    spawn_time: Option<SystemTime>,
}

/// Prepares each attempt of a job before it is spawned.
///
/// Attempt 0 keeps the job's own nonce and prompt exactly as submitted; a
/// fallback retry mints a fresh nonce, swaps it into a copy of the prompt and
/// stores it on the row, so its own association can never match an earlier
/// attempt's still-on-disk rollout (`locate_by_nonce` has no snapshot/floor
/// exclusion and returns on the first match). Claude attempts each pin a fresh
/// session id, so a retry never collides with a prior attempt's half-created
/// session; codex attempts each take their own pre-spawn rollout snapshot.
struct AttemptBuilder<'a> {
    db: &'a DbHandle,
    job: &'a Job,
    current: &'a StdMutex<Attempt>,
    /// Lists the codex rollouts that exist now.
    snapshot: fn() -> HashSet<PathBuf>,
}

impl AttemptBuilder<'_> {
    /// The nonce and prompt of attempt `idx`; for a retry, the new nonce is
    /// stored on the row with the previous association cleared.
    fn identity(&self, idx: usize) -> (Option<String>, String) {
        let job = self.job;
        let Some(base) = job.nonce.as_deref().filter(|_| idx > 0) else {
            return (job.nonce.clone(), job.prompt.clone());
        };
        let new_nonce = format!("{base}-retry{idx}");
        let swapped = job.prompt.replace(
            &render::nonce_marker(base),
            &render::nonce_marker(&new_nonce),
        );
        // The row must carry the identity this attempt actually embeds:
        // dispatch_logs / dispatch_steer re-validate the cached rollout and
        // re-locate by `row.nonce`'s FULL marker, so a row left on `<base>`
        // would reject this attempt's rollout (and, if attempt 0's is still
        // on disk, resolve to that failed attempt instead). Clearing the
        // prior association at the same time fails closed until this
        // attempt's own rollout is found.
        if let Ok(conn) = self.db.lock()
            && let Err(e) = store::set_attempt_nonce(&conn, &job.id, &new_nonce)
        {
            tracing::warn!("dispatch: set_attempt_nonce({}) failed: {e}", job.id);
        }
        (Some(new_nonce), swapped)
    }

    /// The `RunSpec` of codex or claude attempt `idx` on `model`.
    fn build(&self, idx: usize, model: Option<&str>) -> agent_exec::RunSpec {
        let job = self.job;
        let (nonce, prompt) = self.identity(idx);
        // Claude: pin a fresh session id per attempt, so the session log path
        // under ~/.claude/projects/ is deterministic (no discovery/polling). A
        // steered run passes the parent session via --resume … --fork-session
        // and still pins its own new id.
        let pin = (job.backend == Backend::Claude).then(|| uuid::Uuid::new_v4().to_string());
        // Codex: snapshot the rollouts that already exist BEFORE this attempt's
        // spawn, so the association can require a file that did not exist yet —
        // never a pre-existing same-cwd session, including a prior fallback
        // attempt's own (already-failed) rollout.
        let snapshot = if job.backend == Backend::Codex {
            (self.snapshot)()
        } else {
            HashSet::new()
        };
        let spec = backend::run_spec(
            job.backend.process().unwrap_or(agent_exec::Backend::Codex),
            &backend::AttemptSpec {
                working_dir: &job.working_dir,
                sandbox: &job.sandbox,
                model,
                reasoning_effort: job.reasoning_effort.as_deref(),
                skip_git_repo_check: job.skip_git_repo_check,
                resume_session: job.resume_session.as_deref(),
                pin_session: pin.as_deref(),
                backend_version: job.backend_version.as_deref(),
            },
            prompt,
        );
        *lock(self.current) = Attempt {
            nonce,
            pin,
            snapshot,
            spawn_time: Some(SystemTime::now()),
        };
        spec
    }

    /// The prompt of opencode attempt `idx`.
    fn opencode_prompt(&self, idx: usize) -> String {
        let (nonce, prompt) = self.identity(idx);
        *lock(self.current) = Attempt {
            nonce,
            spawn_time: Some(SystemTime::now()),
            ..Attempt::default()
        };
        prompt
    }
}

/// The opencode model chain. opencode is not a process backend of
/// `agent-exec`, so each attempt runs through `opencode::run` inside
/// `agent_exec::fallback_chain`, which applies the crate's order, stopping
/// rules and classification of the failure text.
async fn opencode_chain(
    builder: &AttemptBuilder<'_>,
    models: &[Option<String>],
    events: &UnboundedSender<RunEvent>,
    ct: &CancellationToken,
) -> FallbackOutcome {
    let job = builder.job;
    let policy = FailureTextPolicy::dispatch();
    let policy = &policy;
    agent_exec::fallback_chain(
        |idx, model: Option<String>| {
            let prompt = builder.opencode_prompt(idx);
            async move {
                let spec = opencode::RunSpec {
                    id: &job.id,
                    working_dir: &job.working_dir,
                    sandbox: &job.sandbox,
                    model: model.as_deref(),
                    reasoning_effort: job.reasoning_effort.as_deref(),
                    prompt: &prompt,
                    state_dir: &job.state_dir,
                    backend_version: job.backend_version.as_deref(),
                    resume_session: job.resume_session.as_deref(),
                    rollout_path: job.rollout_path.as_deref(),
                };
                let outcome = opencode::run(spec, events, ct).await;
                AttemptResult {
                    failure_text: agent_exec::failure_text(policy, opencode::BINARY, &outcome),
                    outcome,
                    label: opencode::BINARY.to_string(),
                }
            }
        },
        models,
        events,
        ct,
    )
    .await
}

// ── the event consumer ────────────────────────────────────

/// Finds the rollout of a codex attempt: `(job, attempt nonce, pre-spawn
/// snapshot)` → `(path, session id)`.
type Locate = fn(&Job, Option<&str>, &HashSet<PathBuf>) -> Option<(PathBuf, String)>;

/// Writes the store from a run's events and runs the codex watchdog.
struct Consumer<'a> {
    db: &'a DbHandle,
    job: &'a Job,
    /// The task's token, fired by `dispatch_cancel`.
    ct: &'a CancellationToken,
    /// A child of `ct` that the attempts run under; the watchdog cancels it
    /// alone to restart.
    attempt_ct: &'a CancellationToken,
    current: &'a StdMutex<Attempt>,
    /// The auto-restart window, `None` when the job is not eligible.
    restart_after: Option<Duration>,
    locate: Locate,
}

/// Which attempts reported `Started`.
#[derive(Debug, Default)]
struct Seen {
    /// The attempt in flight has spawned.
    started: bool,
    /// The last finished attempt had spawned.
    final_started: bool,
}

/// What handling one event asks of the drive loop.
enum Next {
    Nothing,
    /// Start the association watchdog of the codex attempt that just spawned.
    Watch(WatchStart),
    /// The attempt finished; its watchdog has nothing left to watch.
    StopWatch,
}

/// What the watchdog of one codex attempt needs.
struct WatchStart {
    /// The spawned process (the guard on Linux and macOS).
    pid: Option<u32>,
    nonce: Option<String>,
    snapshot: HashSet<PathBuf>,
    spawn_time: SystemTime,
}

/// How a watchdog ended: the attempt proceeds, or it must be restarted after
/// the given window.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Proceed,
    Restart(Duration),
}

/// One step of the drive loop.
enum Step {
    Event(RunEvent),
    Watchdog(Verdict),
    Done(FallbackOutcome),
}

type WatchFuture<'a> = Pin<Box<dyn Future<Output = Verdict> + Send + 'a>>;

impl Consumer<'_> {
    /// Consume `rx` while `chain` runs, then write the terminal state. Returns
    /// the auto-restart successor when the watchdog restarted the run.
    async fn drive(
        &self,
        chain: impl Future<Output = FallbackOutcome>,
        mut rx: UnboundedReceiver<RunEvent>,
    ) -> Option<Job> {
        let mut chain = std::pin::pin!(chain);
        let mut seen = Seen::default();
        let mut watchdog: Option<WatchFuture<'_>> = None;
        let mut restart: Option<Duration> = None;
        let fallback = loop {
            let step = tokio::select! {
                biased;
                Some(ev) = rx.recv() => Step::Event(ev),
                v = async {
                    match watchdog.as_mut() {
                        Some(w) => w.await,
                        None => std::future::pending().await,
                    }
                } => Step::Watchdog(v),
                out = &mut chain => Step::Done(out),
            };
            match step {
                Step::Event(ev) => match self.on_event(ev, &mut seen) {
                    Next::Nothing => {}
                    Next::Watch(w) => watchdog = Some(Box::pin(self.watch(w))),
                    Next::StopWatch => watchdog = None,
                },
                Step::Watchdog(v) => {
                    watchdog = None;
                    if let Verdict::Restart(window) = v {
                        restart = Some(window);
                        self.attempt_ct.cancel();
                    }
                }
                Step::Done(out) => break out,
            }
        };
        // Events sent in the poll that completed the chain are still queued;
        // they precede the terminal write (a `Started` must land before
        // `finish`).
        while let Ok(ev) = rx.try_recv() {
            let _ = self.on_event(ev, &mut seen);
        }
        if let (Some(window), Outcome::Cancelled) = (restart, &fallback.outcome) {
            // The watchdog already decided; the row's terminal state is the
            // restart's, and no fallback history is recorded.
            return self.restart(window);
        }
        finish(
            self.db,
            self.job.backend,
            &self.job.id,
            fallback,
            seen.final_started,
        );
        None
    }

    /// Apply one event to the store.
    fn on_event(&self, ev: RunEvent, seen: &mut Seen) -> Next {
        let job = self.job;
        match ev {
            RunEvent::Started {
                pid,
                argv,
                backend_version,
                ..
            } => {
                seen.started = true;
                let argv_json = serde_json::to_string(&argv).unwrap_or_default();
                db_mark_running(
                    self.db,
                    &job.id,
                    pid.map(|p| p as i64),
                    &argv_json,
                    backend_version.as_deref(),
                );
                let mut a = lock(self.current);
                match job.backend {
                    Backend::Claude => {
                        if let Some(pin) = a.pin.as_deref() {
                            let path =
                                harness_log::claude::claude_session_path(&job.working_dir, pin);
                            db_set_session(self.db, &job.id, pin, &path.to_string_lossy());
                        }
                        Next::Nothing
                    }
                    Backend::Codex => Next::Watch(WatchStart {
                        pid,
                        nonce: a.nonce.clone(),
                        snapshot: std::mem::take(&mut a.snapshot),
                        spawn_time: a.spawn_time.unwrap_or_else(SystemTime::now),
                    }),
                    Backend::Opencode => Next::Nothing,
                }
            }
            RunEvent::Session { session_id, .. } => {
                if job.backend == Backend::Opencode {
                    let path = opencode::log_path(
                        job.rollout_path.as_deref(),
                        &job.state_dir,
                        &session_id,
                    );
                    db_set_session(self.db, &job.id, &session_id, &path.to_string_lossy());
                }
                Next::Nothing
            }
            // The runner has already appended the line to its own log.
            RunEvent::Progress(_) => Next::Nothing,
            RunEvent::Finished(_) => {
                seen.final_started = seen.started;
                seen.started = false;
                Next::StopWatch
            }
        }
    }

    /// Associate the rollout a codex attempt just created (path + session id) so
    /// dispatch_logs can tail it live and dispatch_steer can resume the session.
    /// codex writes the rollout at startup, so polling covers the spawn →
    /// first-write gap. The match is **positive**, never "newest same-cwd": a
    /// resume run is found by its already-known session id; a fresh run by its
    /// prompt nonce (falling back to the pre-spawn snapshot diff only when there
    /// is no nonce). The DB lock is held only for the brief `set_session`, never
    /// across a sleep.
    ///
    /// Watchdog: when an ELIGIBLE run (`restart_after` set) is still unassociated
    /// past the restart window AND nothing under working_dir changed since
    /// spawn, the verdict is `Restart`: the attempt is killed and re-submitted
    /// once as a fresh task (`restart_of` linkage). Detection is a best-effort
    /// mtime scan — the accepted residual risk (documented in the dispatch rule)
    /// is killing a healthy run whose rollout the locator failed to find and
    /// that had not yet written anything. A cancelled or exited attempt is the
    /// run's business: the watchdog then proceeds and never eats a real outcome.
    async fn watch(&self, w: WatchStart) -> Verdict {
        let job = self.job;
        let assoc_budget = Duration::from_millis(ASSOC_POLL_MS * ROLLOUT_LOCATE_ATTEMPTS as u64);
        let start = tokio::time::Instant::now();
        loop {
            if self.ct.is_cancelled() || self.attempt_ct.is_cancelled() {
                return Verdict::Proceed;
            }
            if let Some(pid) = w.pid
                && !liveness::process_alive(pid as i32)
            {
                return Verdict::Proceed;
            }
            if let Some((path, sid)) = (self.locate)(job, w.nonce.as_deref(), &w.snapshot) {
                db_set_session(self.db, &job.id, &sid, &path.to_string_lossy());
                return Verdict::Proceed;
            }

            let elapsed = start.elapsed();
            match self.restart_after {
                Some(window) if elapsed >= window => {
                    return match dir_changed_since(&job.working_dir, w.spawn_time) {
                        Some(false) => Verdict::Restart(window),
                        verdict => {
                            tracing::info!(
                                "dispatch: {} unassociated past {}s but working_dir scan says {} — not restarting",
                                job.id,
                                window.as_secs(),
                                if verdict.is_some() {
                                    "changed"
                                } else {
                                    "inconclusive"
                                },
                            );
                            Verdict::Proceed
                        }
                    };
                }
                Some(_) => {}
                None if elapsed >= assoc_budget => {
                    tracing::warn!("dispatch: could not locate codex rollout for {}", job.id);
                    return Verdict::Proceed;
                }
                None => {}
            }
            tokio::time::sleep(Duration::from_millis(ASSOC_POLL_MS)).await;
        }
    }

    /// The restart window fired and the attempt was killed: terminal-mark the
    /// row (conditionally — a racing terminal write wins), then insert and
    /// return the fresh successor task.
    fn restart(&self, window: Duration) -> Option<Job> {
        let db = self.db;
        let job = self.job;
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
            tracing::warn!(
                "dispatch: {} was already terminal when the auto-restart fired; no successor",
                job.id
            );
            return None;
        }

        let old = match db.lock() {
            Ok(conn) => store::get(&conn, &job.id).ok().flatten(),
            Err(_) => None,
        }?;

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
                return None;
            }
        };
        let new_id = match inserted {
            Ok(store::InsertOutcome::Created(id)) => id,
            Ok(store::InsertOutcome::Conflict(existing)) => {
                tracing::warn!(
                    "dispatch: restart of {} aborted — {existing} became active for the dir first",
                    job.id
                );
                return None;
            }
            Err(e) => {
                tracing::warn!("dispatch: restart insert for {} failed: {e}", job.id);
                return None;
            }
        };
        tracing::info!("dispatch: {} auto-restarted as {new_id}", job.id);

        Some(Job {
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
        })
    }
}

/// Locate a codex attempt's rollout: a resume by its known session id (codex
/// appends the new turn to the same file); a fresh run by its nonce (positive
/// identity, robust to a concurrent same-cwd codex), else by the snapshot diff.
fn locate_rollout(
    job: &Job,
    nonce: Option<&str>,
    snapshot: &HashSet<PathBuf>,
) -> Option<(PathBuf, String)> {
    match job.resume_session.as_deref() {
        Some(sid) => harness_log::codex::locate_by_session_id(sid).map(|p| (p, sid.to_string())),
        None => match nonce {
            Some(n) => rollout::locate_by_nonce(&job.working_dir, n),
            None => rollout::locate_new_by_cwd(&job.working_dir, snapshot, None),
        },
    }
}

// ── terminal state ────────────────────────────────────────

/// A row's terminal columns.
#[derive(Debug, PartialEq, Eq)]
struct Terminal {
    status: &'static str,
    exit_code: Option<i64>,
    result: Option<String>,
    error: Option<String>,
}

/// The terminal columns for the chain's final `outcome` of a `backend` run.
/// `started` says whether that attempt had spawned: a cancellation before it
/// did is recorded as such.
fn terminal(backend: Backend, outcome: &Outcome, started: bool) -> Terminal {
    let failed = |error: String| Terminal {
        status: store::STATUS_FAILED,
        exit_code: None,
        result: None,
        error: Some(error),
    };
    match outcome {
        Outcome::Record(r) => Terminal {
            status: if r.success {
                store::STATUS_SUCCEEDED
            } else {
                store::STATUS_FAILED
            },
            exit_code: r.exit_code.map(|c| c as i64),
            result: Some(build_result(&r.stdout, r.stdout_total, r.stdout_truncated)),
            error: (!r.success).then(|| build_error(&r.stderr, r.stderr_truncated, r.exit_code)),
        },
        // opencode's not-found text is its install hint alone.
        Outcome::NotFound { hint, .. } if backend == Backend::Opencode => failed(hint.clone()),
        Outcome::NotFound { binary, hint } => {
            failed(format!("backend `{binary}` not found on PATH — {hint}"))
        }
        Outcome::Spawn(msg) | Outcome::WaitFailed(msg) => failed(msg.clone()),
        Outcome::Cancelled => Terminal {
            status: store::STATUS_CANCELLED,
            exit_code: None,
            result: None,
            error: Some(if started {
                "cancelled by request; the backend process group was killed".to_string()
            } else {
                "cancelled before the backend started".to_string()
            }),
        },
    }
}

/// Write the chain's terminal outcome, then — only when at least one fallback
/// attempt was tried and discarded — record which model actually produced the
/// result plus the discarded attempts' history. A cancellation before the
/// last attempt spawned records no history.
fn finish(db: &DbHandle, backend: Backend, id: &str, f: FallbackOutcome, started: bool) {
    let t = terminal(backend, &f.outcome, started);
    db_finish(
        db,
        id,
        t.status,
        t.exit_code,
        t.result.as_deref(),
        t.error.as_deref(),
    );
    let cancelled_before_start = matches!(f.outcome, Outcome::Cancelled) && !started;
    if !f.discarded.is_empty() && !cancelled_before_start {
        db_set_fallback_result(db, id, f.model.as_deref(), &history_json(&f.discarded));
    }
}

/// The stored `fallback_history`: `[{model, error_kind, detail}]`, the model
/// `(backend default)` when unset and the detail at most 2,000 characters.
fn history_json(discarded: &[DiscardedAttempt]) -> String {
    serde_json::to_string(
        &discarded
            .iter()
            .map(|a| {
                serde_json::json!({
                    "model": a.model_label(),
                    "error_kind": a.kind.as_str(),
                    "detail": a.detail.chars().take(DISCARDED_DETAIL_CHARS).collect::<String>(),
                })
            })
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| "[]".into())
}

/// Interval between association attempts.
const ASSOC_POLL_MS: u64 = 150;

/// Default auto-restart window for a fresh codex submit whose rollout never
/// associates. Overridable via `DISPATCH_RESTART_UNASSOCIATED_SECS` (0 = off).
const RESTART_DEFAULT_SECS: u64 = 30;

/// Cap on entries visited by `dir_changed_since` — past this the scan is
/// inconclusive (None) and the restart is skipped rather than risked.
const DIR_SCAN_CAP: usize = 20_000;

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

fn build_result(stdout: &str, total: u64, truncated: bool) -> String {
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

/// Lock the attempt state; a poisoned lock still holds a usable value.
fn lock(m: &StdMutex<Attempt>) -> std::sync::MutexGuard<'_, Attempt> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

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

fn db_set_session(db: &DbHandle, id: &str, session_id: &str, path: &str) {
    if let Ok(conn) = db.lock()
        && let Err(e) = store::set_session(&conn, id, session_id, path)
    {
        tracing::warn!("dispatch: set_session({id}) failed: {e}");
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
    use agent_exec::{BackendErrorKind, RunRecord};
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    // ── fixtures ──────────────────────────────────────────

    /// A scratch directory unique to this test process and `tag`, empty.
    fn scratch(tag: &str) -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("dispatch-exec-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// An in-memory store with one queued row for a job of `backend` in `dir`.
    fn db_with_row(backend: Backend, dir: &Path, nonce: Option<&str>) -> (DbHandle, String) {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        store::init(&conn).unwrap();
        let new = store::NewTask {
            plan_id: None,
            backend: backend.as_str().to_string(),
            working_dir: dir.to_string_lossy().into_owned(),
            title: None,
            spec_json: "{}".to_string(),
            prompt: prompt_with(nonce),
            model: None,
            reasoning_effort: None,
            sandbox: "workspace-write".to_string(),
            parent_id: None,
            nonce: nonce.map(str::to_string),
            rollout_start_line: None,
            model_fallback: None,
            allow_concurrent: false,
            restart_of: None,
        };
        let id = match store::insert_queued(
            &mut conn,
            &new,
            std::process::id() as i64,
            "test-instance",
            None,
        )
        .unwrap()
        {
            store::InsertOutcome::Created(id) => id,
            store::InsertOutcome::Conflict(other) => panic!("conflict with {other}"),
        };
        (Arc::new(StdMutex::new(conn)), id)
    }

    fn prompt_with(nonce: Option<&str>) -> String {
        match nonce {
            Some(n) => format!("do the thing\n\n{}", render::nonce_marker(n)),
            None => "do the thing".to_string(),
        }
    }

    fn job(id: &str, backend: Backend, dir: &Path, nonce: Option<&str>) -> Job {
        Job {
            id: id.to_string(),
            backend,
            working_dir: dir.to_path_buf(),
            sandbox: "workspace-write".to_string(),
            model: Some("m1".to_string()),
            reasoning_effort: None,
            skip_git_repo_check: false,
            prompt: prompt_with(nonce),
            backend_version: Some("v-test".to_string()),
            state_dir: dir.join("state"),
            resume_session: None,
            nonce: nonce.map(str::to_string),
            rollout_path: None,
            model_fallback: Some(vec!["m2".to_string()]),
            allow_concurrent: false,
            restart_of: None,
        }
    }

    fn row(db: &DbHandle, id: &str) -> store::TaskRow {
        store::get(&db.lock().unwrap(), id).unwrap().unwrap()
    }

    static SNAPSHOTS: AtomicUsize = AtomicUsize::new(0);

    fn counting_snapshot() -> HashSet<PathBuf> {
        SNAPSHOTS.fetch_add(1, Ordering::SeqCst);
        HashSet::from([PathBuf::from("pre-existing.jsonl")])
    }

    fn never_found(_: &Job, _: Option<&str>, _: &HashSet<PathBuf>) -> Option<(PathBuf, String)> {
        None
    }

    fn found(_: &Job, _: Option<&str>, _: &HashSet<PathBuf>) -> Option<(PathBuf, String)> {
        Some((PathBuf::from("rollout-x-sid-9.jsonl"), "sid-9".to_string()))
    }

    fn started(pid: u32) -> RunEvent {
        RunEvent::Started {
            pid: Some(pid),
            argv: vec!["codex".into(), "exec".into()],
            backend_version: Some("v-test".into()),
            guarded: true,
        }
    }

    fn record(success: bool, stdout: &str, stderr: &str) -> Outcome {
        Outcome::Record(RunRecord {
            backend: "codex".into(),
            exit_code: Some(if success { 0 } else { 1 }),
            success,
            stdout: stdout.into(),
            stdout_total: stdout.len() as u64,
            stderr: stderr.into(),
            ..RunRecord::default()
        })
    }

    /// Drive `cx` over a scripted chain, bounded so a hang fails the test.
    async fn drive_bounded(
        cx: &Consumer<'_>,
        chain: impl Future<Output = FallbackOutcome>,
        rx: UnboundedReceiver<RunEvent>,
    ) -> Option<Job> {
        tokio::time::timeout(Duration::from_secs(10), cx.drive(chain, rx))
            .await
            .expect("drive did not finish within 10s")
    }

    // ── attempt builder ───────────────────────────────────

    #[test]
    fn builder_gives_each_attempt_a_fresh_nonce_and_clears_the_association() {
        let dir = scratch("builder-codex");
        let (db, id) = db_with_row(Backend::Codex, &dir, Some("d-1:N1"));
        let job = job(&id, Backend::Codex, &dir, Some("d-1:N1"));
        store::set_session(&db.lock().unwrap(), &id, "old-sid", "/old/rollout.jsonl").unwrap();
        let current = StdMutex::new(Attempt::default());
        let b = AttemptBuilder {
            db: &db,
            job: &job,
            current: &current,
            snapshot: counting_snapshot,
        };
        let before = SNAPSHOTS.load(Ordering::SeqCst);

        let first = b.build(0, Some("m1"));
        assert_eq!(
            first.prompt, job.prompt,
            "attempt 0 runs the prompt as submitted"
        );
        assert_eq!(first.model.as_deref(), Some("m1"));
        assert_eq!(lock(&current).nonce.as_deref(), Some("d-1:N1"));
        assert!(
            lock(&current)
                .snapshot
                .contains(Path::new("pre-existing.jsonl"))
        );
        assert!(lock(&current).spawn_time.is_some());
        assert_eq!(lock(&current).pin, None, "codex pins no session");
        let r = row(&db, &id);
        assert_eq!(r.nonce.as_deref(), Some("d-1:N1"));
        assert_eq!(
            r.session_id.as_deref(),
            Some("old-sid"),
            "attempt 0 keeps the row"
        );

        let second = b.build(1, Some("m2"));
        assert!(
            second
                .prompt
                .contains(&render::nonce_marker("d-1:N1-retry1"))
        );
        assert!(!second.prompt.contains(&render::nonce_marker("d-1:N1")));
        assert_eq!(second.model.as_deref(), Some("m2"));
        assert_eq!(lock(&current).nonce.as_deref(), Some("d-1:N1-retry1"));
        let r = row(&db, &id);
        assert_eq!(r.nonce.as_deref(), Some("d-1:N1-retry1"));
        assert_eq!(r.session_id, None, "the prior association is cleared");
        assert_eq!(r.rollout_path, None);

        let third = b.build(2, None);
        assert!(
            third
                .prompt
                .contains(&render::nonce_marker("d-1:N1-retry2"))
        );
        assert_eq!(row(&db, &id).nonce.as_deref(), Some("d-1:N1-retry2"));
        // One snapshot per attempt, taken while building it (before its spawn).
        assert_eq!(SNAPSHOTS.load(Ordering::SeqCst) - before, 3);

        // The spec carries dispatch's codex policy.
        let argv = agent_exec::command(&first).1;
        assert!(argv.contains(&"mcp_servers.dispatch.enabled=false".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn builder_pins_a_fresh_claude_session_per_attempt() {
        let dir = scratch("builder-claude");
        let (db, id) = db_with_row(Backend::Claude, &dir, Some("d-2:N2"));
        let job = job(&id, Backend::Claude, &dir, Some("d-2:N2"));
        let current = StdMutex::new(Attempt::default());
        let b = AttemptBuilder {
            db: &db,
            job: &job,
            current: &current,
            snapshot: || panic!("claude attempts take no rollout snapshot"),
        };
        let first = b.build(0, None);
        let pin1 = lock(&current).pin.clone().expect("pinned");
        let second = b.build(1, None);
        let pin2 = lock(&current).pin.clone().expect("pinned");
        assert_ne!(pin1, pin2, "each attempt pins its own session id");
        assert_eq!(first.pin_session.as_deref(), Some(pin1.as_str()));
        assert_eq!(second.pin_session.as_deref(), Some(pin2.as_str()));
        let argv = agent_exec::command(&second).1;
        let at = argv.iter().position(|a| a == "--session-id").unwrap();
        assert_eq!(argv[at + 1], pin2);
        assert_eq!(lock(&current).nonce.as_deref(), Some("d-2:N2-retry1"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── outcome → row ─────────────────────────────────────

    #[test]
    fn each_outcome_maps_to_its_row() {
        let ok = terminal(Backend::Codex, &record(true, "all done", ""), true);
        assert_eq!(
            ok,
            Terminal {
                status: store::STATUS_SUCCEEDED,
                exit_code: Some(0),
                result: Some("all done".into()),
                error: None,
            }
        );

        let bad = terminal(Backend::Claude, &record(false, "partial", "boom"), true);
        assert_eq!(
            bad,
            Terminal {
                status: store::STATUS_FAILED,
                exit_code: Some(1),
                result: Some("partial".into()),
                error: Some("exit code 1\nboom".into()),
            }
        );

        let truncated = Outcome::Record(RunRecord {
            success: true,
            exit_code: Some(0),
            stdout: "head".into(),
            stdout_total: 999,
            stdout_truncated: true,
            ..RunRecord::default()
        });
        assert_eq!(
            terminal(Backend::Codex, &truncated, true).result.as_deref(),
            Some("head\n\n[stdout truncated; captured first 4 of 999 bytes]")
        );

        let missing = Outcome::NotFound {
            binary: "codex".into(),
            hint: "install codex".into(),
        };
        let t = terminal(Backend::Codex, &missing, false);
        assert_eq!(t.status, store::STATUS_FAILED);
        assert_eq!(t.exit_code, None);
        assert_eq!(t.result, None);
        assert_eq!(
            t.error.as_deref(),
            Some("backend `codex` not found on PATH — install codex")
        );
        let oc_missing = Outcome::NotFound {
            binary: "opencode".into(),
            hint: opencode::install_hint(),
        };
        assert_eq!(
            terminal(Backend::Opencode, &oc_missing, false).error,
            Some(opencode::install_hint())
        );

        for o in [
            Outcome::Spawn("spawn agent-guard for codex failed: x".into()),
            Outcome::WaitFailed("wait failed: y".into()),
        ] {
            let msg = match &o {
                Outcome::Spawn(m) | Outcome::WaitFailed(m) => m.clone(),
                _ => unreachable!(),
            };
            let t = terminal(Backend::Codex, &o, true);
            assert_eq!(t.status, store::STATUS_FAILED);
            assert_eq!(t.exit_code, None);
            assert_eq!(t.result, None);
            assert_eq!(t.error, Some(msg));
        }

        let killed = terminal(Backend::Codex, &Outcome::Cancelled, true);
        assert_eq!(killed.status, store::STATUS_CANCELLED);
        assert_eq!(
            killed.error.as_deref(),
            Some("cancelled by request; the backend process group was killed")
        );
        let early = terminal(Backend::Codex, &Outcome::Cancelled, false);
        assert_eq!(early.status, store::STATUS_CANCELLED);
        assert_eq!(
            early.error.as_deref(),
            Some("cancelled before the backend started")
        );
    }

    // ── the consumer ──────────────────────────────────────

    #[tokio::test]
    async fn events_write_running_then_the_final_outcome_with_history() {
        let dir = scratch("drive-history");
        let (db, id) = db_with_row(Backend::Codex, &dir, None);
        let job = job(&id, Backend::Codex, &dir, None);
        let ct = CancellationToken::new();
        let attempt_ct = ct.child_token();
        let current = StdMutex::new(Attempt::default());
        let cx = Consumer {
            db: &db,
            job: &job,
            ct: &ct,
            attempt_ct: &attempt_ct,
            current: &current,
            restart_after: None,
            locate: found,
        };
        let (tx, rx) = unbounded_channel();
        let chain = async {
            let _ = tx.send(started(4242));
            let _ = tx.send(RunEvent::Finished(record(false, "", "429 rate limit")));
            let _ = tx.send(started(4243));
            let done = record(true, "final answer", "");
            let _ = tx.send(RunEvent::Finished(done.clone()));
            FallbackOutcome {
                outcome: done,
                model: Some("m2".into()),
                discarded: vec![DiscardedAttempt {
                    model: None,
                    kind: BackendErrorKind::RateLimited,
                    detail: "x".repeat(2500),
                }],
            }
        };
        assert!(drive_bounded(&cx, chain, rx).await.is_none());

        let r = row(&db, &id);
        assert_eq!(r.status, store::STATUS_SUCCEEDED);
        assert_eq!(r.result.as_deref(), Some("final answer"));
        assert_eq!(r.error, None);
        assert_eq!(r.exit_code, Some(0));
        assert_eq!(r.child_pid, Some(4243), "the last attempt's pid");
        assert_eq!(r.argv.as_deref(), Some(r#"["codex","exec"]"#));
        assert_eq!(r.backend_version.as_deref(), Some("v-test"));
        assert!(r.started_at.is_some());
        assert_eq!(r.final_model.as_deref(), Some("m2"));
        let history: serde_json::Value =
            serde_json::from_str(r.fallback_history.as_deref().unwrap()).unwrap();
        assert_eq!(history[0]["model"], "(backend default)");
        assert_eq!(
            history[0]["error_kind"],
            BackendErrorKind::RateLimited.as_str()
        );
        assert_eq!(history[0]["detail"].as_str().unwrap().chars().count(), 2000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn claude_started_records_the_pinned_session() {
        let dir = scratch("drive-claude");
        let (db, id) = db_with_row(Backend::Claude, &dir, None);
        let job = job(&id, Backend::Claude, &dir, None);
        let ct = CancellationToken::new();
        let attempt_ct = ct.child_token();
        let current = StdMutex::new(Attempt {
            pin: Some("pin-uuid-1".into()),
            ..Attempt::default()
        });
        let cx = Consumer {
            db: &db,
            job: &job,
            ct: &ct,
            attempt_ct: &attempt_ct,
            current: &current,
            restart_after: None,
            locate: never_found,
        };
        let (tx, rx) = unbounded_channel();
        let chain = async {
            let _ = tx.send(started(7));
            let failed = record(false, "", "bad");
            let _ = tx.send(RunEvent::Finished(failed.clone()));
            FallbackOutcome {
                outcome: failed,
                model: Some("m1".into()),
                discarded: Vec::new(),
            }
        };
        assert!(drive_bounded(&cx, chain, rx).await.is_none());
        let r = row(&db, &id);
        assert_eq!(r.status, store::STATUS_FAILED);
        assert_eq!(r.error.as_deref(), Some("exit code 1\nbad"));
        assert_eq!(r.session_id.as_deref(), Some("pin-uuid-1"));
        assert!(
            r.rollout_path
                .as_deref()
                .is_some_and(|p| p.ends_with("pin-uuid-1.jsonl"))
        );
        assert_eq!(r.fallback_history, None, "no retry, no history");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn opencode_session_records_the_dispatch_owned_log() {
        let dir = scratch("drive-opencode");
        let (db, id) = db_with_row(Backend::Opencode, &dir, None);
        let job = job(&id, Backend::Opencode, &dir, None);
        let ct = CancellationToken::new();
        let attempt_ct = ct.child_token();
        let current = StdMutex::new(Attempt::default());
        let cx = Consumer {
            db: &db,
            job: &job,
            ct: &ct,
            attempt_ct: &attempt_ct,
            current: &current,
            restart_after: None,
            locate: never_found,
        };
        let (tx, rx) = unbounded_channel();
        let chain = async {
            let _ = tx.send(started(9));
            let _ = tx.send(RunEvent::Session {
                session_id: "ses_1".into(),
                port: Some(1234),
            });
            let _ = tx.send(RunEvent::Progress("{}".into()));
            let done = record(true, "ok", "");
            let _ = tx.send(RunEvent::Finished(done.clone()));
            FallbackOutcome {
                outcome: done,
                model: None,
                discarded: Vec::new(),
            }
        };
        assert!(drive_bounded(&cx, chain, rx).await.is_none());
        let r = row(&db, &id);
        assert_eq!(r.status, store::STATUS_SUCCEEDED);
        assert_eq!(r.session_id.as_deref(), Some("ses_1"));
        let expected = opencode::log_path(None, &job.state_dir, "ses_1");
        assert_eq!(
            r.rollout_path.as_deref(),
            Some(expected.to_string_lossy().as_ref())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_cancel_before_the_spawn_records_no_history() {
        let dir = scratch("drive-early-cancel");
        let (db, id) = db_with_row(Backend::Codex, &dir, None);
        let job = job(&id, Backend::Codex, &dir, None);
        let ct = CancellationToken::new();
        let attempt_ct = ct.child_token();
        let current = StdMutex::new(Attempt::default());
        let cx = Consumer {
            db: &db,
            job: &job,
            ct: &ct,
            attempt_ct: &attempt_ct,
            current: &current,
            restart_after: None,
            locate: never_found,
        };
        let (tx, rx) = unbounded_channel();
        let chain = async {
            let _ = tx.send(started(1));
            let _ = tx.send(RunEvent::Finished(record(false, "", "rate limit")));
            // The next attempt is cancelled before it is built.
            let _ = tx.send(RunEvent::Finished(Outcome::Cancelled));
            FallbackOutcome {
                outcome: Outcome::Cancelled,
                model: Some("m2".into()),
                discarded: vec![DiscardedAttempt {
                    model: Some("m1".into()),
                    kind: BackendErrorKind::RateLimited,
                    detail: "rate limit".into(),
                }],
            }
        };
        assert!(drive_bounded(&cx, chain, rx).await.is_none());
        let r = row(&db, &id);
        assert_eq!(r.status, store::STATUS_CANCELLED);
        assert_eq!(
            r.error.as_deref(),
            Some("cancelled before the backend started")
        );
        assert_eq!(r.fallback_history, None);
        assert_eq!(r.final_model, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A scripted codex attempt: it spawns (as this live test process), then
    /// runs until its attempt token is cancelled.
    async fn attempt_until_cancelled(
        tx: &UnboundedSender<RunEvent>,
        current: &StdMutex<Attempt>,
        attempt_ct: &CancellationToken,
        nonce: &str,
    ) -> FallbackOutcome {
        *lock(current) = Attempt {
            nonce: Some(nonce.to_string()),
            spawn_time: Some(SystemTime::now()),
            ..Attempt::default()
        };
        let _ = tx.send(started(std::process::id()));
        attempt_ct.cancelled().await;
        let _ = tx.send(RunEvent::Finished(Outcome::Cancelled));
        FallbackOutcome {
            outcome: Outcome::Cancelled,
            model: Some("m1".into()),
            discarded: Vec::new(),
        }
    }

    #[tokio::test]
    async fn the_watchdog_restarts_an_unassociated_quiet_run() {
        let dir = scratch("drive-restart");
        let (db, id) = db_with_row(Backend::Codex, &dir, Some("d-5:N5"));
        let job = job(&id, Backend::Codex, &dir, Some("d-5:N5"));
        assert_eq!(
            restart_window(&job).is_some(),
            restart_after_secs().is_some()
        );
        let ct = CancellationToken::new();
        let attempt_ct = ct.child_token();
        let current = StdMutex::new(Attempt::default());
        let cx = Consumer {
            db: &db,
            job: &job,
            ct: &ct,
            attempt_ct: &attempt_ct,
            current: &current,
            restart_after: Some(Duration::ZERO),
            locate: never_found,
        };
        let (tx, rx) = unbounded_channel();
        let chain = attempt_until_cancelled(&tx, &current, &attempt_ct, "d-5:N5");
        let successor = drive_bounded(&cx, chain, rx)
            .await
            .expect("the watchdog restarts the run");

        assert!(!ct.is_cancelled(), "a restart never fires the user's token");
        let r = row(&db, &id);
        assert_eq!(r.status, store::STATUS_INTERRUPTED);
        assert!(
            r.error
                .as_deref()
                .is_some_and(|e| e.starts_with("auto-restarted:"))
        );
        assert_eq!(successor.restart_of.as_deref(), Some(id.as_str()));
        assert_eq!(successor.nonce.as_deref(), Some("d-5:N5-restart"));
        assert!(
            successor
                .prompt
                .contains(&render::nonce_marker("d-5:N5-restart"))
        );
        let conn = db.lock().unwrap();
        assert_eq!(
            store::restart_successor(&conn, &id).unwrap().as_deref(),
            Some(successor.id.as_str())
        );
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_user_cancel_ends_the_row_cancelled_even_when_restart_is_eligible() {
        let dir = scratch("drive-user-cancel");
        let (db, id) = db_with_row(Backend::Codex, &dir, Some("d-6:N6"));
        let job = job(&id, Backend::Codex, &dir, Some("d-6:N6"));
        let ct = CancellationToken::new();
        let attempt_ct = ct.child_token();
        let current = StdMutex::new(Attempt::default());
        let cx = Consumer {
            db: &db,
            job: &job,
            ct: &ct,
            attempt_ct: &attempt_ct,
            current: &current,
            restart_after: Some(Duration::ZERO),
            locate: never_found,
        };
        // dispatch_cancel fires the task token; the attempt token is its child.
        ct.cancel();
        let (tx, rx) = unbounded_channel();
        let chain = attempt_until_cancelled(&tx, &current, &attempt_ct, "d-6:N6");
        assert!(drive_bounded(&cx, chain, rx).await.is_none());

        let r = row(&db, &id);
        assert_eq!(r.status, store::STATUS_CANCELLED);
        assert_eq!(
            r.error.as_deref(),
            Some("cancelled by request; the backend process group was killed")
        );
        let conn = db.lock().unwrap();
        assert_eq!(store::restart_successor(&conn, &id).unwrap(), None);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_associated_run_is_not_restarted() {
        let dir = scratch("drive-associated");
        let (db, id) = db_with_row(Backend::Codex, &dir, Some("d-8:N8"));
        let job = job(&id, Backend::Codex, &dir, Some("d-8:N8"));
        let ct = CancellationToken::new();
        let attempt_ct = ct.child_token();
        let current = StdMutex::new(Attempt::default());
        let cx = Consumer {
            db: &db,
            job: &job,
            ct: &ct,
            attempt_ct: &attempt_ct,
            current: &current,
            restart_after: Some(Duration::ZERO),
            locate: found,
        };
        let (tx, rx) = unbounded_channel();
        let chain = async {
            *lock(&current) = Attempt {
                nonce: Some("d-8:N8".into()),
                spawn_time: Some(SystemTime::now()),
                ..Attempt::default()
            };
            let _ = tx.send(started(std::process::id()));
            // Let the watchdog run before the attempt completes.
            tokio::time::sleep(Duration::from_millis(300)).await;
            let done = record(true, "done", "");
            let _ = tx.send(RunEvent::Finished(done.clone()));
            FallbackOutcome {
                outcome: done,
                model: Some("m1".into()),
                discarded: Vec::new(),
            }
        };
        assert!(drive_bounded(&cx, chain, rx).await.is_none());
        assert!(!attempt_ct.is_cancelled());
        let r = row(&db, &id);
        assert_eq!(r.status, store::STATUS_SUCCEEDED);
        assert_eq!(r.session_id.as_deref(), Some("sid-9"));
        assert_eq!(r.rollout_path.as_deref(), Some("rollout-x-sid-9.jsonl"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
