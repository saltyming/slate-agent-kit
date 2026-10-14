//! SQLite persistence for dispatch tasks (`dispatch.db`).
//!
//! Uses a single rusqlite `Connection`
//! lives behind a std `Mutex` in the server struct; WAL + `busy_timeout` let
//! multiple dispatch server processes (one per harness session) share one
//! project DB. Every function here is synchronous and takes `&Connection` (or
//! `&mut Connection` where a transaction is needed); callers lock the mutex
//! briefly and never hold the guard across an `.await` — that keeps the
//! detached executor future `Send`.

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::{Value, json};

// ── status constants ──────────────────────────────────────

pub const STATUS_QUEUED: &str = "queued";
pub const STATUS_RUNNING: &str = "running";
pub const STATUS_SUCCEEDED: &str = "succeeded";
pub const STATUS_FAILED: &str = "failed";
pub const STATUS_CANCELLED: &str = "cancelled";
pub const STATUS_INTERRUPTED: &str = "interrupted";

/// A task that is still owned by a live executor (queued or running). Used by
/// the per-working_dir concurrency guard and by boot reconciliation.
pub fn is_active(status: &str) -> bool {
    matches!(status, STATUS_QUEUED | STATUS_RUNNING)
}

// ── kinds ─────────────────────────────────────────────────

/// The `kind` of an execution row: a write-capable run in its working_dir.
pub const KIND_EXECUTE: &str = "execute";
/// The `kind` of a consultation row: a read-only question to another model
/// family, run in the server's directory.
pub const KIND_CONSULT: &str = "consult";

/// What a run is: an execution or a consultation. Stored in the `kind`
/// column; a row written before the column existed reads as `Execute`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A write-capable run in its working_dir (`dispatch_submit`,
    /// `dispatch_steer`).
    Execute,
    /// A read-only consultation (`dispatch_consult`).
    Consult,
}

impl Kind {
    /// The stored value: `execute` or `consult`.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Execute => KIND_EXECUTE,
            Kind::Consult => KIND_CONSULT,
        }
    }

    /// Parse a stored or requested value; anything else is `None`.
    pub fn parse(s: &str) -> Option<Kind> {
        match s.trim() {
            KIND_EXECUTE => Some(Kind::Execute),
            KIND_CONSULT => Some(Kind::Consult),
            _ => None,
        }
    }
}

// ── schema ────────────────────────────────────────────────

pub const SCHEMA_SQL: &str = "\
CREATE TABLE IF NOT EXISTS dispatch_tasks (
    id               TEXT PRIMARY KEY,
    plan_id          TEXT,
    backend          TEXT NOT NULL DEFAULT 'codex',
    working_dir      TEXT NOT NULL,
    title            TEXT,
    spec_json        TEXT NOT NULL,
    prompt           TEXT NOT NULL,
    status           TEXT NOT NULL DEFAULT 'queued',
    model            TEXT,
    reasoning_effort TEXT,
    sandbox          TEXT NOT NULL DEFAULT 'workspace-write',
    backend_version  TEXT,
    argv             TEXT,
    owner_pid        INTEGER,
    owner_instance   TEXT,
    child_pid        INTEGER,
    exit_code        INTEGER,
    result           TEXT,
    error            TEXT,
    created_at       TEXT NOT NULL DEFAULT (datetime('now')),
    started_at       TEXT,
    finished_at      TEXT,
    session_id       TEXT,
    rollout_path     TEXT,
    parent_id        TEXT,
    nonce            TEXT,
    rollout_start_line INTEGER,
    model_fallback   TEXT,
    final_model      TEXT,
    fallback_history TEXT,
    allow_concurrent INTEGER NOT NULL DEFAULT 0,
    restart_of       TEXT,
    kind             TEXT NOT NULL DEFAULT 'execute'
);
CREATE INDEX IF NOT EXISTS idx_dispatch_plan_status ON dispatch_tasks(plan_id, status);
CREATE INDEX IF NOT EXISTS idx_dispatch_dir_status  ON dispatch_tasks(working_dir, status);
CREATE TABLE IF NOT EXISTS dispatch_counters (
    scope   TEXT PRIMARY KEY,
    next_id INTEGER NOT NULL DEFAULT 1
);
";

const COLS: &str = "id, plan_id, backend, working_dir, title, spec_json, prompt, status, \
model, reasoning_effort, sandbox, backend_version, argv, owner_pid, owner_instance, child_pid, \
exit_code, result, error, created_at, started_at, finished_at, session_id, rollout_path, parent_id, \
nonce, rollout_start_line, model_fallback, final_model, fallback_history, allow_concurrent, \
restart_of, kind";

pub fn init(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA_SQL)?;
    // Idempotent upgrade for DBs created before these columns existed. ALTER ...
    // ADD COLUMN errors when the column already exists (fresh DB from SCHEMA_SQL
    // above) — that specific failure is expected and ignored.
    for (col, decl) in [
        ("session_id", "TEXT"),
        ("rollout_path", "TEXT"),
        ("parent_id", "TEXT"),
        ("nonce", "TEXT"),
        ("rollout_start_line", "INTEGER"),
        ("model_fallback", "TEXT"),
        ("final_model", "TEXT"),
        ("fallback_history", "TEXT"),
        ("allow_concurrent", "INTEGER NOT NULL DEFAULT 0"),
        ("restart_of", "TEXT"),
        ("kind", "TEXT NOT NULL DEFAULT 'execute'"),
    ] {
        let _ = conn.execute(
            &format!("ALTER TABLE dispatch_tasks ADD COLUMN {col} {decl}"),
            [],
        );
    }
    Ok(())
}

// ── row types ─────────────────────────────────────────────

/// Fields supplied at submit time (the id is allocated by `insert_queued`).
#[derive(Debug, Clone)]
pub struct NewTask {
    pub plan_id: Option<String>,
    pub backend: String,
    pub working_dir: String,
    pub title: Option<String>,
    pub spec_json: String,
    pub prompt: String,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub sandbox: String,
    pub parent_id: Option<String>,
    /// Per-task identity marker embedded in the rendered prompt, used to positively
    /// match the backend log this task produced (None for steer/resume tasks, which
    /// already know their session id).
    pub nonce: Option<String>,
    /// For a steer/resume task: the rollout line count at resume time, so dispatch_logs
    /// can show only the new turn rather than the whole inherited parent session.
    pub rollout_start_line: Option<i64>,
    /// Ordered fallback chain (JSON array of model strings), tried in order on a
    /// transient backend error. None/empty for dispatch_steer follow-ups — a
    /// resumed session stays on one model.
    pub model_fallback: Option<String>,
    /// Whether this task bypasses the one-run-per-working_dir guard. Persisted
    /// (mirrors submit's `allow_concurrent`) so a steer chain can inherit it.
    /// Stored as INTEGER 0/1.
    pub allow_concurrent: bool,
    /// For an automatic-restart successor: the task id whose unassociated run was
    /// killed and re-run as this fresh task. Distinct from `parent_id`, which
    /// means steer/resume (an inherited backend session) — a restart is a brand
    /// new session and must never be treated as a resume by log association.
    pub restart_of: Option<String>,
    /// Execution or consultation.
    pub kind: Kind,
}

/// A full row read back from the DB.
#[derive(Debug, Clone)]
pub struct TaskRow {
    pub id: String,
    pub plan_id: Option<String>,
    pub backend: String,
    pub working_dir: String,
    pub title: Option<String>,
    pub spec_json: String,
    pub prompt: String,
    pub status: String,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub sandbox: String,
    pub backend_version: Option<String>,
    pub argv: Option<String>,
    pub owner_pid: Option<i64>,
    pub owner_instance: Option<String>,
    pub child_pid: Option<i64>,
    pub exit_code: Option<i64>,
    pub result: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub session_id: Option<String>,
    pub rollout_path: Option<String>,
    pub parent_id: Option<String>,
    pub nonce: Option<String>,
    pub rollout_start_line: Option<i64>,
    /// Audit copy of the configured fallback chain (JSON array), set at submit time.
    pub model_fallback: Option<String>,
    /// The model that actually produced the terminal outcome — equal to `model`
    /// unless a fallback retry occurred, in which case it's whichever chain entry
    /// finally succeeded (or the last one tried, if the whole chain failed).
    pub final_model: Option<String>,
    /// JSON array of `{model, error_kind, detail}` for every fallback attempt that
    /// was tried and discarded before `final_model`. Absent/null if no retry happened.
    pub fallback_history: Option<String>,
    /// Whether this task bypasses the one-run-per-working_dir guard. A steer
    /// inherits it from the parent unless the steer call overrides it.
    pub allow_concurrent: bool,
    /// Set on an automatic-restart successor: the id of the task it re-runs.
    pub restart_of: Option<String>,
    /// The stored kind, `execute` or `consult` (see [`TaskRow::kind`]).
    pub kind: String,
}

fn row_from(r: &Row) -> rusqlite::Result<TaskRow> {
    Ok(TaskRow {
        id: r.get(0)?,
        plan_id: r.get(1)?,
        backend: r.get(2)?,
        working_dir: r.get(3)?,
        title: r.get(4)?,
        spec_json: r.get(5)?,
        prompt: r.get(6)?,
        status: r.get(7)?,
        model: r.get(8)?,
        reasoning_effort: r.get(9)?,
        sandbox: r.get(10)?,
        backend_version: r.get(11)?,
        argv: r.get(12)?,
        owner_pid: r.get(13)?,
        owner_instance: r.get(14)?,
        child_pid: r.get(15)?,
        exit_code: r.get(16)?,
        result: r.get(17)?,
        error: r.get(18)?,
        created_at: r.get(19)?,
        started_at: r.get(20)?,
        finished_at: r.get(21)?,
        session_id: r.get(22)?,
        rollout_path: r.get(23)?,
        parent_id: r.get(24)?,
        nonce: r.get(25)?,
        rollout_start_line: r.get(26)?,
        model_fallback: r.get(27)?,
        final_model: r.get(28)?,
        fallback_history: r.get(29)?,
        allow_concurrent: r.get(30)?,
        restart_of: r.get(31)?,
        kind: r.get(32)?,
    })
}

impl TaskRow {
    /// The row's kind; an unrecognized stored value reads as `Execute`, the
    /// column's default.
    pub fn kind(&self) -> Kind {
        Kind::parse(&self.kind).unwrap_or(Kind::Execute)
    }

    /// JSON view for tool output. `include_result` inlines the (large) captured
    /// stdout/error plus operational/fallback detail — list responses omit it,
    /// status includes it. `include_spec` inlines the (very large) request-side
    /// audit fields — the accepted `spec`, the rendered `prompt` (a second copy of
    /// the spec), and `argv`; omitted by default so repeated polling of a task
    /// does not re-echo its whole spec on every call.
    pub fn to_json(&self, include_result: bool, include_spec: bool) -> Value {
        let mut v = json!({
            "id": self.id,
            "kind": self.kind,
            "plan_id": self.plan_id,
            "backend": self.backend,
            "working_dir": self.working_dir,
            "title": self.title,
            "status": self.status,
            "model": self.model,
            "reasoning_effort": self.reasoning_effort,
            "sandbox": self.sandbox,
            "backend_version": self.backend_version,
            "child_pid": self.child_pid,
            "exit_code": self.exit_code,
            "created_at": self.created_at,
            "started_at": self.started_at,
            "finished_at": self.finished_at,
            "parent_id": self.parent_id,
            "final_model": self.final_model,
            "allow_concurrent": self.allow_concurrent,
        });
        if self.restart_of.is_some() {
            v["restart_of"] = json!(self.restart_of);
        }
        if include_result {
            v["result"] = json!(self.result);
            v["error"] = json!(self.error);
            v["owner_pid"] = json!(self.owner_pid);
            v["owner_instance"] = json!(self.owner_instance);
            v["session_id"] = json!(self.session_id);
            v["rollout_path"] = json!(self.rollout_path);
            v["rollout_start_line"] = json!(self.rollout_start_line);
            v["model_fallback"] = self
                .model_fallback
                .as_deref()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .unwrap_or(Value::Null);
            v["fallback_history"] = self
                .fallback_history
                .as_deref()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .unwrap_or(Value::Null);
            if let Some(hist_json) = &self.fallback_history
                && let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(hist_json)
                && !arr.is_empty()
            {
                let failed: Vec<String> = arr
                    .iter()
                    .map(|a| {
                        format!(
                            "{} ({})",
                            a.get("model").and_then(Value::as_str).unwrap_or("?"),
                            a.get("error_kind").and_then(Value::as_str).unwrap_or("?")
                        )
                    })
                    .collect();
                v["fallback_summary"] = json!(format!(
                    "{} on attempt {} with model {} after {} failed",
                    if self.status == STATUS_SUCCEEDED {
                        "succeeded"
                    } else {
                        "finished"
                    },
                    arr.len() + 1,
                    self.final_model.as_deref().unwrap_or("(unknown)"),
                    failed.join(", "),
                ));
            }
        }
        if include_spec {
            v["argv"] = json!(self.argv);
            v["prompt"] = json!(self.prompt);
            v["spec"] = serde_json::from_str::<Value>(&self.spec_json)
                .unwrap_or_else(|_| json!(self.spec_json));
        }
        v
    }
}

// ── lifecycle ─────────────────────────────────────────────

/// Result of a guarded insert.
pub enum InsertOutcome {
    /// Inserted; carries the new id (e.g. `d-7`).
    Created(String),
    /// Rejected: an active (queued/running) task already targets this working_dir.
    /// Carries that task's id.
    Conflict(String),
}

/// Allocate the next sequential id and insert the task as `queued`, atomically.
///
/// When `enforce_unique_dir` is `Some(dir)`, the per-working_dir concurrency guard
/// is applied *inside the same transaction*: the check and the insert are atomic so
/// two processes cannot both pass the check and both insert. The transaction is
/// `IMMEDIATE` (write lock taken at BEGIN), so a concurrent submit for the same dir
/// blocks on `busy_timeout` until this one commits, then sees the row and is
/// rejected — a `DEFERRED` transaction would take no lock until its first write,
/// letting both submits read an empty result and both insert.
pub fn insert_queued(
    conn: &mut Connection,
    t: &NewTask,
    owner_pid: i64,
    owner_instance: &str,
    enforce_unique_dir: Option<&str>,
) -> rusqlite::Result<InsertOutcome> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if let Some(dir) = enforce_unique_dir
        && let Some(existing) = active_for_dir(&tx, dir)?
    {
        // tx drops here → rollback; no counter id is burned.
        return Ok(InsertOutcome::Conflict(existing));
    }
    tx.execute(
        "INSERT OR IGNORE INTO dispatch_counters (scope, next_id) VALUES ('global', 1)",
        [],
    )?;
    let n: i64 = tx.query_row(
        "SELECT next_id FROM dispatch_counters WHERE scope='global'",
        [],
        |r| r.get(0),
    )?;
    let id = format!("d-{}", n);
    tx.execute(
        "INSERT INTO dispatch_tasks \
         (id, plan_id, backend, working_dir, title, spec_json, prompt, status, \
          model, reasoning_effort, sandbox, owner_pid, owner_instance, parent_id, \
          nonce, rollout_start_line, model_fallback, allow_concurrent, restart_of, kind) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, \
         ?20)",
        params![
            id,
            t.plan_id,
            t.backend,
            t.working_dir,
            t.title,
            t.spec_json,
            t.prompt,
            STATUS_QUEUED,
            t.model,
            t.reasoning_effort,
            t.sandbox,
            owner_pid,
            owner_instance,
            t.parent_id,
            t.nonce,
            t.rollout_start_line,
            t.model_fallback,
            t.allow_concurrent,
            t.restart_of,
            t.kind.as_str(),
        ],
    )?;
    tx.execute(
        "UPDATE dispatch_counters SET next_id = ?1 WHERE scope='global'",
        params![n + 1],
    )?;
    tx.commit()?;
    Ok(InsertOutcome::Created(id))
}

/// Transition queued → running once the child process exists.
pub fn mark_running(
    conn: &Connection,
    id: &str,
    child_pid: Option<i64>,
    argv: &str,
    backend_version: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE dispatch_tasks SET status = ?1, child_pid = ?2, argv = ?3, \
         backend_version = ?4, started_at = datetime('now') WHERE id = ?5",
        params![STATUS_RUNNING, child_pid, argv, backend_version, id],
    )?;
    Ok(())
}

/// Record the codex session id + rollout file path once the executor has located
/// them (used by `dispatch_logs` to read the live log and `dispatch_steer` to resume).
pub fn set_session(
    conn: &Connection,
    id: &str,
    session_id: &str,
    rollout_path: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE dispatch_tasks SET session_id = ?1, rollout_path = ?2 WHERE id = ?3",
        params![session_id, rollout_path, id],
    )?;
    Ok(())
}

/// Switch a task to a fallback attempt's identity: the nonce its prompt now
/// embeds, with the previous attempt's association cleared (a rollout found for
/// the old nonce is no longer this task's live log).
pub fn set_attempt_nonce(conn: &Connection, id: &str, nonce: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE dispatch_tasks SET nonce = ?1, session_id = NULL, rollout_path = NULL WHERE id = ?2",
        params![nonce, id],
    )?;
    Ok(())
}

/// A run's terminal outcome, written by [`finish`] as one unit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunOutcome<'a> {
    /// `succeeded`, `failed` or `cancelled`.
    pub status: &'a str,
    /// The backend's exit code, when it exited.
    pub exit_code: Option<i64>,
    /// The captured result (execution: stdout; consultation: the reply text).
    pub result: Option<&'a str>,
    /// The error text of a run that did not succeed.
    pub error: Option<&'a str>,
    /// The model that produced the outcome; set only when a fallback attempt
    /// was tried and discarded first, else `None` (stored NULL).
    pub final_model: Option<&'a str>,
    /// JSON array of the discarded attempts (`{model, error_kind, detail}`);
    /// `None` (stored NULL) when no fallback attempt was discarded.
    pub fallback_history: Option<&'a str>,
}

/// Write a terminal outcome: status, exit code, result, error, final model and
/// fallback history in ONE statement, so a reader that sees the terminal
/// status also sees the rest of the outcome (`dispatch wait` reports
/// `final_model` the moment it reads a terminal status).
pub fn finish(conn: &Connection, id: &str, o: &RunOutcome<'_>) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE dispatch_tasks SET status = ?1, exit_code = ?2, result = ?3, error = ?4, \
         final_model = ?5, fallback_history = ?6, finished_at = datetime('now') WHERE id = ?7",
        params![
            o.status,
            o.exit_code,
            o.result,
            o.error,
            o.final_model,
            o.fallback_history,
            id
        ],
    )?;
    Ok(())
}

/// Mark a stranded task (owner process died, or an auto-restarted run) as
/// interrupted — but only while the row is still active. The conditional guard
/// makes the transition safe against a racing terminal write (e.g. the owning
/// executor finishing successfully between a liveness check and this update):
/// a terminal row is never clobbered. Returns the number of rows changed
/// (1 = transitioned, 0 = the row was already terminal or missing).
pub fn mark_interrupted(conn: &Connection, id: &str, reason: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE dispatch_tasks SET status = ?1, error = ?2, finished_at = datetime('now') \
         WHERE id = ?3 AND status IN ('queued', 'running')",
        params![STATUS_INTERRUPTED, reason, id],
    )
}

// ── reads ─────────────────────────────────────────────────

pub fn get(conn: &Connection, id: &str) -> rusqlite::Result<Option<TaskRow>> {
    conn.query_row(
        &format!("SELECT {COLS} FROM dispatch_tasks WHERE id = ?1"),
        params![id],
        row_from,
    )
    .optional()
}

/// List tasks, optionally filtered by plan_id, status and/or kind. `rowid`
/// ordering preserves insertion order (the TEXT id sorts lexically, which is
/// wrong).
pub fn list(
    conn: &Connection,
    plan_id: Option<&str>,
    status: Option<&str>,
    kind: Option<Kind>,
) -> rusqlite::Result<Vec<TaskRow>> {
    let sql = format!(
        "SELECT {COLS} FROM dispatch_tasks \
         WHERE (?1 IS NULL OR plan_id = ?1) AND (?2 IS NULL OR status = ?2) \
         AND (?3 IS NULL OR kind = ?3) \
         ORDER BY rowid"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![plan_id, status, kind.map(Kind::as_str)], row_from)?;
    rows.collect()
}

/// Returns the id of an active (queued/running) execution already targeting
/// this canonical working_dir, if any — the per-dir concurrency guard.
/// Consultations take no working-dir slot, so they are never counted.
pub fn active_for_dir(conn: &Connection, working_dir: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT id FROM dispatch_tasks WHERE working_dir = ?1 AND kind = 'execute' \
         AND status IN ('queued', 'running') ORDER BY rowid LIMIT 1",
        params![working_dir],
        |r| r.get(0),
    )
    .optional()
}

/// The id of the fresh task that automatically re-ran `id` after its original
/// run was killed unassociated — surfaced on the interrupted row's status so
/// the chain is discoverable from either side.
pub fn restart_successor(conn: &Connection, id: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT id FROM dispatch_tasks WHERE restart_of = ?1 ORDER BY rowid LIMIT 1",
        params![id],
        |r| r.get(0),
    )
    .optional()
}

/// Active task ids belonging to a plan — for cancel-by-plan.
pub fn active_ids_for_plan(conn: &Connection, plan_id: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM dispatch_tasks WHERE plan_id = ?1 AND status IN ('queued', 'running') \
         ORDER BY rowid",
    )?;
    let rows = stmt.query_map(params![plan_id], |r| r.get::<_, String>(0))?;
    rows.collect()
}

/// (id, owner_pid, created_at) for every active task — boot reconciliation
/// reads this and the caller decides which owners are dead (liveness lives in
/// `liveness`, which has the platform-specific process checks). `created_at`
/// is the SQLite text the row was inserted with, the time its owner was already
/// running at.
pub fn active_owners(conn: &Connection) -> rusqlite::Result<Vec<(String, Option<i64>, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, owner_pid, created_at FROM dispatch_tasks WHERE status IN ('queued', 'running')",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    fn sample_task() -> NewTask {
        NewTask {
            plan_id: None,
            backend: "codex".to_string(),
            working_dir: "/tmp/dispatch-guard".to_string(),
            title: None,
            spec_json: "{}".to_string(),
            prompt: "noop".to_string(),
            model: None,
            reasoning_effort: None,
            sandbox: "workspace-write".to_string(),
            parent_id: None,
            nonce: None,
            rollout_start_line: None,
            model_fallback: None,
            allow_concurrent: false,
            restart_of: None,
            kind: Kind::Execute,
        }
    }

    fn open(path: &std::path::Path) -> Connection {
        let c = Connection::open(path).unwrap();
        c.execute_batch("PRAGMA busy_timeout=5000; PRAGMA journal_mode=WAL;")
            .unwrap();
        c
    }

    /// The one-active-run-per-working_dir guard must hold across *separate*
    /// connections racing to insert: IMMEDIATE serializes them so exactly one
    /// wins and the other sees the conflict. (A DEFERRED transaction would let
    /// both read an empty active-dir result and both insert.)
    #[test]
    fn one_active_run_per_dir_is_atomic_across_connections() {
        let path =
            std::env::temp_dir().join(format!("dispatch-guard-test-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        init(&open(&path)).unwrap();

        let (p1, p2) = (path.clone(), path.clone());
        let h1 = thread::spawn(move || {
            insert_queued(
                &mut open(&p1),
                &sample_task(),
                1,
                "i1",
                Some("/tmp/dispatch-guard"),
            )
            .unwrap()
        });
        let h2 = thread::spawn(move || {
            insert_queued(
                &mut open(&p2),
                &sample_task(),
                2,
                "i2",
                Some("/tmp/dispatch-guard"),
            )
            .unwrap()
        });
        let r1 = h1.join().unwrap();
        let r2 = h2.join().unwrap();

        let created = [&r1, &r2]
            .iter()
            .filter(|r| matches!(r, InsertOutcome::Created(_)))
            .count();
        let conflict = [&r1, &r2]
            .iter()
            .filter(|r| matches!(r, InsertOutcome::Conflict(_)))
            .count();
        assert_eq!(created, 1, "exactly one racing insert should be created");
        assert_eq!(conflict, 1, "the other should see the active-dir conflict");

        let _ = std::fs::remove_file(&path);
    }

    /// allow_concurrent (enforce_unique_dir = None) bypasses the guard entirely.
    #[test]
    fn allow_concurrent_skips_the_guard() {
        let path = std::env::temp_dir().join(format!(
            "dispatch-concurrent-test-{}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let mut c = open(&path);
        init(&c).unwrap();
        let a = insert_queued(&mut c, &sample_task(), 1, "i1", None).unwrap();
        let b = insert_queued(&mut c, &sample_task(), 1, "i1", None).unwrap();
        assert!(matches!(a, InsertOutcome::Created(_)));
        assert!(matches!(b, InsertOutcome::Created(_)));
        let _ = std::fs::remove_file(&path);
    }

    /// The persisted `allow_concurrent` flag round-trips through insert + read
    /// (this is the first bool column, so the 0/1 mapping is asserted explicitly).
    #[test]
    fn allow_concurrent_persists_round_trip() {
        let path =
            std::env::temp_dir().join(format!("dispatch-ac-roundtrip-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut c = open(&path);
        init(&c).unwrap();

        let mut t_true = sample_task();
        t_true.allow_concurrent = true;
        let id_true = match insert_queued(&mut c, &t_true, 1, "i1", None).unwrap() {
            InsertOutcome::Created(id) => id,
            InsertOutcome::Conflict(_) => panic!("expected Created"),
        };
        let mut t_false = sample_task();
        t_false.allow_concurrent = false;
        let id_false = match insert_queued(&mut c, &t_false, 1, "i1", None).unwrap() {
            InsertOutcome::Created(id) => id,
            InsertOutcome::Conflict(_) => panic!("expected Created"),
        };

        assert!(get(&c, &id_true).unwrap().unwrap().allow_concurrent);
        assert!(!get(&c, &id_false).unwrap().unwrap().allow_concurrent);
        let _ = std::fs::remove_file(&path);
    }

    /// A `dispatch.db` created before the column existed still opens, migrates,
    /// and reads back with `allow_concurrent` backfilled to false.
    #[test]
    fn migration_adds_allow_concurrent_to_old_db() {
        let path =
            std::env::temp_dir().join(format!("dispatch-ac-migrate-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        // The current schema WITHOUT the allow_concurrent column (a pre-change DB).
        let old_schema = "\
CREATE TABLE dispatch_tasks (
    id TEXT PRIMARY KEY, plan_id TEXT, backend TEXT NOT NULL DEFAULT 'codex',
    working_dir TEXT NOT NULL, title TEXT, spec_json TEXT NOT NULL, prompt TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued', model TEXT, reasoning_effort TEXT,
    sandbox TEXT NOT NULL DEFAULT 'workspace-write', backend_version TEXT, argv TEXT,
    owner_pid INTEGER, owner_instance TEXT, child_pid INTEGER, exit_code INTEGER,
    result TEXT, error TEXT, created_at TEXT NOT NULL DEFAULT (datetime('now')),
    started_at TEXT, finished_at TEXT, session_id TEXT, rollout_path TEXT, parent_id TEXT,
    nonce TEXT, rollout_start_line INTEGER, model_fallback TEXT, final_model TEXT,
    fallback_history TEXT
);
CREATE TABLE dispatch_counters (scope TEXT PRIMARY KEY, next_id INTEGER NOT NULL DEFAULT 1);
";
        {
            let c = open(&path);
            c.execute_batch(old_schema).unwrap();
            c.execute(
                "INSERT INTO dispatch_tasks (id, backend, working_dir, spec_json, prompt) \
                 VALUES ('d-1', 'codex', '/tmp/x', '{}', 'noop')",
                [],
            )
            .unwrap();
        }
        // Reopen (as new code would) and migrate.
        let c = open(&path);
        init(&c).unwrap();
        let row = get(&c, "d-1")
            .unwrap()
            .expect("old row still readable after migration");
        assert!(
            !row.allow_concurrent,
            "an old row must backfill allow_concurrent to false"
        );
        assert_eq!(row.kind, KIND_EXECUTE, "an old row must read as execute");
        assert_eq!(row.kind(), Kind::Execute);
        assert_eq!(row.to_json(false, false)["kind"], json!("execute"));
        let _ = std::fs::remove_file(&path);
    }

    /// `mark_interrupted` transitions only active rows — a terminal row is never
    /// clobbered by a racing liveness check (the live-reconcile race codex review
    /// flagged: owner looks dead at read time, but the executor already finished).
    #[test]
    fn mark_interrupted_is_conditional_on_active() {
        let path =
            std::env::temp_dir().join(format!("dispatch-interrupt-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut c = open(&path);
        init(&c).unwrap();

        let id = match insert_queued(&mut c, &sample_task(), 1, "i1", None).unwrap() {
            InsertOutcome::Created(id) => id,
            InsertOutcome::Conflict(_) => panic!("expected Created"),
        };
        assert_eq!(mark_interrupted(&c, &id, "r1").unwrap(), 1);
        assert_eq!(
            mark_interrupted(&c, &id, "r2").unwrap(),
            0,
            "an already-terminal row must not transition again"
        );
        let row = get(&c, &id).unwrap().unwrap();
        assert_eq!(row.status, STATUS_INTERRUPTED);
        assert_eq!(row.error.as_deref(), Some("r1"), "r2 must not overwrite r1");

        let id2 = match insert_queued(&mut c, &sample_task(), 1, "i1", None).unwrap() {
            InsertOutcome::Created(id) => id,
            InsertOutcome::Conflict(_) => panic!("expected Created"),
        };
        finish(
            &c,
            &id2,
            &RunOutcome {
                status: STATUS_SUCCEEDED,
                exit_code: Some(0),
                result: Some("ok"),
                ..RunOutcome::default()
            },
        )
        .unwrap();
        assert_eq!(
            mark_interrupted(&c, &id2, "late").unwrap(),
            0,
            "a succeeded row must never be clobbered to interrupted"
        );
        assert_eq!(get(&c, &id2).unwrap().unwrap().status, STATUS_SUCCEEDED);
        let _ = std::fs::remove_file(&path);
    }

    /// `active_owners` lists only active rows, each with its owner pid and the
    /// `created_at` text SQLite wrote, which reconciliation compares with the
    /// owner's start time.
    #[test]
    fn active_owners_returns_the_owner_pid_and_created_at_of_active_rows() {
        let mut c = Connection::open_in_memory().unwrap();
        init(&c).unwrap();
        let mut insert =
            |owner: i64| match insert_queued(&mut c, &sample_task(), owner, "i", None).unwrap() {
                InsertOutcome::Created(id) => id,
                InsertOutcome::Conflict(_) => panic!("expected Created"),
            };
        let queued = insert(41);
        let done = insert(42);
        c.execute(
            "UPDATE dispatch_tasks SET status = ?1 WHERE id = ?2",
            params![STATUS_SUCCEEDED, done],
        )
        .unwrap();
        c.execute(
            "UPDATE dispatch_tasks SET owner_pid = NULL, created_at = '2026-10-14 06:23:41' \
             WHERE id = ?1",
            params![queued],
        )
        .unwrap();
        assert_eq!(
            active_owners(&c).unwrap(),
            vec![(queued.clone(), None, "2026-10-14 06:23:41".to_string())]
        );
        let again = {
            c.execute(
                "UPDATE dispatch_tasks SET owner_pid = 43 WHERE id = ?1",
                params![queued],
            )
            .unwrap();
            active_owners(&c).unwrap()
        };
        assert_eq!(again[0].1, Some(43));
        let stored = get(&c, &queued).unwrap().unwrap().created_at;
        assert_eq!(again[0].2, stored);
    }

    /// `restart_of` round-trips through insert + read, the successor is
    /// discoverable from the original id, and `to_json` surfaces the field only
    /// when set (so plain tasks don't grow a null key).
    #[test]
    fn restart_of_round_trips_and_successor_lookup() {
        let path = std::env::temp_dir().join(format!("dispatch-restart-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut c = open(&path);
        init(&c).unwrap();

        let id1 = match insert_queued(&mut c, &sample_task(), 1, "i1", None).unwrap() {
            InsertOutcome::Created(id) => id,
            InsertOutcome::Conflict(_) => panic!("expected Created"),
        };
        let mut succ = sample_task();
        succ.restart_of = Some(id1.clone());
        let id2 = match insert_queued(&mut c, &succ, 1, "i1", None).unwrap() {
            InsertOutcome::Created(id) => id,
            InsertOutcome::Conflict(_) => panic!("expected Created"),
        };

        let row1 = get(&c, &id1).unwrap().unwrap();
        let row2 = get(&c, &id2).unwrap().unwrap();
        assert_eq!(row2.restart_of.as_deref(), Some(id1.as_str()));
        assert_eq!(
            restart_successor(&c, &id1).unwrap().as_deref(),
            Some(id2.as_str())
        );
        assert_eq!(restart_successor(&c, &id2).unwrap(), None);
        assert_eq!(row2.to_json(false, false)["restart_of"], json!(id1));
        assert!(
            row1.to_json(false, false).get("restart_of").is_none(),
            "plain tasks must not grow a restart_of key"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// Insert a task with the given spec_json + prompt, read it back, and stamp
    /// terminal result/error/argv so the `to_json` views can be exercised.
    fn terminal_row(tag: &str, spec_json: &str, prompt: &str) -> TaskRow {
        let path =
            std::env::temp_dir().join(format!("dispatch-tojson-{tag}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut c = open(&path);
        init(&c).unwrap();
        let mut t = sample_task();
        t.spec_json = spec_json.to_string();
        t.prompt = prompt.to_string();
        let id = match insert_queued(&mut c, &t, 1, "i1", None).unwrap() {
            InsertOutcome::Created(id) => id,
            InsertOutcome::Conflict(_) => panic!("expected Created"),
        };
        let mut row = get(&c, &id).unwrap().unwrap();
        row.result = Some("build ok".to_string());
        row.error = Some("a warning".to_string());
        row.argv = Some(r#"["codex","exec"]"#.to_string());
        let _ = std::fs::remove_file(&path);
        row
    }

    /// Compact status (the default) must drop the giant spec/prompt/argv but keep
    /// the terminal result/error — the fused-boolean hazard: flipping status to
    /// `to_json(false)` would silently drop result/error too.
    #[test]
    fn status_compact_omits_spec_prompt_argv_but_keeps_result() {
        let row = terminal_row(
            "compact",
            r#"{"objective":"do the thing"}"#,
            "RENDERED PROMPT",
        );
        let v = row.to_json(true, false);
        assert!(v.get("result").is_some(), "compact status must keep result");
        assert!(v.get("error").is_some(), "compact status must keep error");
        assert!(v.get("spec").is_none(), "compact status must omit spec");
        assert!(v.get("prompt").is_none(), "compact status must omit prompt");
        assert!(v.get("argv").is_none(), "compact status must omit argv");
    }

    /// `include_spec=true` re-adds the accepted spec (parsed), the rendered prompt,
    /// and argv, without dropping the terminal result.
    #[test]
    fn status_verbose_includes_spec() {
        let row = terminal_row(
            "verbose",
            r#"{"objective":"do the thing"}"#,
            "RENDERED PROMPT",
        );
        let v = row.to_json(true, true);
        assert!(
            v["spec"].is_object(),
            "include_spec re-parses spec_json to an object"
        );
        assert_eq!(v["spec"]["objective"], json!("do the thing"));
        assert!(
            v.get("prompt").is_some(),
            "include_spec returns the rendered prompt"
        );
        assert!(v.get("argv").is_some(), "include_spec returns argv");
        assert!(v.get("result").is_some(), "result stays alongside the spec");
    }

    /// The list form (`to_json(false, false)`) stays minimal: no result, no spec.
    #[test]
    fn list_form_omits_result_and_spec() {
        let row = terminal_row("list", r#"{"objective":"x"}"#, "RENDERED PROMPT");
        let v = row.to_json(false, false);
        assert!(v.get("result").is_none(), "list omits the captured result");
        assert!(v.get("error").is_none(), "list omits error");
        assert!(v.get("spec").is_none(), "list omits spec");
        assert!(v.get("prompt").is_none(), "list omits prompt");
        assert!(v.get("id").is_some());
        assert!(v.get("status").is_some());
        assert!(v.get("allow_concurrent").is_some());
    }

    /// A fallback retry swaps the nonce its prompt embeds; the row must follow
    /// (so main.rs validates / re-locates by the marker actually on disk) and the
    /// prior attempt's association must be dropped rather than trusted.
    #[test]
    fn set_attempt_nonce_replaces_identity_and_clears_association() {
        let path = std::env::temp_dir().join(format!(
            "dispatch-attempt-nonce-test-{}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let mut conn = open(&path);
        init(&conn).unwrap();
        let mut task = sample_task();
        task.nonce = Some("d-1:N".to_string());
        let id = match insert_queued(&mut conn, &task, 1, "i1", None).unwrap() {
            InsertOutcome::Created(id) => id,
            InsertOutcome::Conflict(other) => panic!("expected Created, got conflict with {other}"),
        };
        set_session(&conn, &id, "sid-attempt0", "/rollouts/attempt0.jsonl").unwrap();

        set_attempt_nonce(&conn, &id, "d-1:N-retry1").unwrap();

        let row = get(&conn, &id).unwrap().unwrap();
        assert_eq!(row.nonce.as_deref(), Some("d-1:N-retry1"));
        assert_eq!(row.session_id, None);
        assert_eq!(row.rollout_path, None);
        let _ = std::fs::remove_file(&path);
    }

    fn created(o: InsertOutcome) -> String {
        match o {
            InsertOutcome::Created(id) => id,
            InsertOutcome::Conflict(other) => panic!("expected Created, got conflict with {other}"),
        }
    }

    /// A database created before the `kind` column (the schema of the previous
    /// release) opens, migrates and reads every old row as `execute`; new rows
    /// of either kind are written beside them.
    #[test]
    fn kind_defaults_to_execute_on_a_database_without_the_column() {
        let path =
            std::env::temp_dir().join(format!("dispatch-kind-migrate-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let c = open(&path);
            // SCHEMA_SQL without the kind column, as the previous release wrote it.
            let old = SCHEMA_SQL.replace(
                ",\n    kind             TEXT NOT NULL DEFAULT 'execute'",
                "",
            );
            assert!(
                !old.contains("kind "),
                "the old schema must lack the column"
            );
            c.execute_batch(&old).unwrap();
            c.execute(
                "INSERT INTO dispatch_tasks (id, backend, working_dir, spec_json, prompt, status) \
                 VALUES ('d-legacy', 'codex', '/tmp/x', '{}', 'noop', 'running')",
                [],
            )
            .unwrap();
        }
        let mut c = open(&path);
        init(&c).unwrap();
        init(&c).unwrap(); // idempotent
        let row = get(&c, "d-legacy").unwrap().unwrap();
        assert_eq!(row.kind(), Kind::Execute);
        let mut consult = sample_task();
        consult.kind = Kind::Consult;
        let id = created(insert_queued(&mut c, &consult, 1, "i1", None).unwrap());
        assert_eq!(get(&c, &id).unwrap().unwrap().kind(), Kind::Consult);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn list_filters_by_kind() {
        let mut c = Connection::open_in_memory().unwrap();
        init(&c).unwrap();
        let exec = created(insert_queued(&mut c, &sample_task(), 1, "i1", None).unwrap());
        let mut t = sample_task();
        t.kind = Kind::Consult;
        let consult = created(insert_queued(&mut c, &t, 1, "i1", None).unwrap());
        let ids = |k: Option<Kind>| -> Vec<String> {
            list(&c, None, None, k)
                .unwrap()
                .into_iter()
                .map(|r| r.id)
                .collect()
        };
        assert_eq!(ids(None), vec![exec.clone(), consult.clone()]);
        assert_eq!(ids(Some(Kind::Execute)), vec![exec]);
        assert_eq!(ids(Some(Kind::Consult)), vec![consult]);
    }

    /// Consultations take no working-dir slot in either direction: an active
    /// consultation in a directory does not block an execution there, and an
    /// active execution does not block a consultation.
    #[test]
    fn active_for_dir_ignores_consult_rows() {
        let mut c = Connection::open_in_memory().unwrap();
        init(&c).unwrap();
        let dir = "/tmp/dispatch-guard";
        let mut consult = sample_task();
        consult.kind = Kind::Consult;
        created(insert_queued(&mut c, &consult, 1, "i1", None).unwrap());
        assert_eq!(active_for_dir(&c, dir).unwrap(), None);
        let exec = created(insert_queued(&mut c, &sample_task(), 1, "i1", Some(dir)).unwrap());
        assert_eq!(active_for_dir(&c, dir).unwrap(), Some(exec));
        // A second consultation is inserted without the guard and stays apart.
        created(insert_queued(&mut c, &consult, 1, "i1", None).unwrap());
        assert!(matches!(
            insert_queued(&mut c, &sample_task(), 1, "i1", Some(dir)).unwrap(),
            InsertOutcome::Conflict(_)
        ));
    }

    /// `finish` writes the whole outcome in one statement: a row read right
    /// after carries the terminal status together with the final model and
    /// the fallback history; without a fallback both stay NULL.
    #[test]
    fn finish_writes_the_whole_outcome_at_once() {
        let mut c = Connection::open_in_memory().unwrap();
        init(&c).unwrap();
        let id = created(insert_queued(&mut c, &sample_task(), 1, "i1", None).unwrap());
        finish(
            &c,
            &id,
            &RunOutcome {
                status: STATUS_SUCCEEDED,
                exit_code: Some(0),
                result: Some("reply"),
                error: None,
                final_model: Some("m2"),
                fallback_history: Some(
                    r#"[{"model":"m1","error_kind":"rate_limited","detail":""}]"#,
                ),
            },
        )
        .unwrap();
        let row = get(&c, &id).unwrap().unwrap();
        assert_eq!(row.status, STATUS_SUCCEEDED);
        assert_eq!(row.exit_code, Some(0));
        assert_eq!(row.result.as_deref(), Some("reply"));
        assert_eq!(row.final_model.as_deref(), Some("m2"));
        assert!(row.fallback_history.is_some());
        assert!(row.finished_at.is_some());

        let id2 = created(insert_queued(&mut c, &sample_task(), 1, "i1", None).unwrap());
        finish(
            &c,
            &id2,
            &RunOutcome {
                status: STATUS_FAILED,
                exit_code: Some(1),
                error: Some("boom"),
                ..RunOutcome::default()
            },
        )
        .unwrap();
        let row = get(&c, &id2).unwrap().unwrap();
        assert_eq!(row.status, STATUS_FAILED);
        assert_eq!(row.error.as_deref(), Some("boom"));
        assert_eq!(row.final_model, None);
        assert_eq!(row.fallback_history, None);
    }

    #[test]
    fn kind_round_trips_its_stored_values() {
        for k in [Kind::Execute, Kind::Consult] {
            assert_eq!(Kind::parse(k.as_str()), Some(k));
        }
        assert_eq!(Kind::parse("other"), None);
    }
}
