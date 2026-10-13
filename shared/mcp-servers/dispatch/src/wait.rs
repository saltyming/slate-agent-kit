//! `dispatch wait <id> [--timeout <seconds>] [--db <path>]`: the binary's CLI
//! mode that blocks until a run is terminal.
//!
//! A harness runs the command line a submit, consult or steer response returns
//! with its own background means, so the agent is brought back when the run
//! ends instead of polling `dispatch_status`. `main` dispatches here before any
//! runtime, tracing or store setup: this mode opens an existing `dispatch.db`
//! only (never creating it or a state directory), reads the one row every
//! 500 ms outside any transaction, probes no backend, reconciles no other row
//! and cancels nothing. Its one write is the server's own conditional
//! `interrupted` transition (`store::mark_interrupted`) for the waited row
//! when that row is active and its recorded owner process is gone.
//!
//! Output: one line on stdout, `<id> <kind> <status>`, then ` model=<final
//! model>` when a fallback answered and ` restarted_as=<id>` when the run was
//! restarted; never the result. Exit codes: [`EXIT_TERMINAL`],
//! [`EXIT_ACTIVE`], [`EXIT_UNKNOWN`], [`EXIT_ERROR`]; errors go to stderr.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::{Connection, OpenFlags};

use crate::liveness::process_alive;
use crate::store;

/// Exit code: the run is terminal (the line names its status).
pub const EXIT_TERMINAL: i32 = 0;
/// Exit code: a usage or store error (stderr says which).
pub const EXIT_ERROR: i32 = 1;
/// Exit code: the run was still active when `--timeout` elapsed.
pub const EXIT_ACTIVE: i32 = 2;
/// Exit code: no run has this id in the store.
pub const EXIT_UNKNOWN: i32 = 3;

/// How often the row is read.
const POLL: Duration = Duration::from_millis(500);

/// How long a read waits out a writer holding the database lock.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

const USAGE: &str = "usage: dispatch wait <id> [--timeout <seconds>] [--db <path>]";

/// The parsed command line.
#[derive(Debug, PartialEq)]
struct Args {
    id: String,
    timeout: Option<Duration>,
    db: Option<PathBuf>,
}

/// Run `dispatch wait` with the arguments after `wait` and return the
/// process exit code.
pub fn main(args: &[OsString]) -> i32 {
    let args = match parse(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("dispatch wait: {e}\n{USAGE}");
            return EXIT_ERROR;
        }
    };
    let db = match args.db.clone().map_or_else(default_db, Ok) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("dispatch wait: {e}");
            return EXIT_ERROR;
        }
    };
    let conn = match open_existing(&db) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("dispatch wait: {e}");
            return EXIT_ERROR;
        }
    };
    let alive = |pid: i64| i32::try_from(pid).is_ok_and(process_alive);
    match wait_for(&conn, &args.id, args.timeout, alive, std::thread::sleep) {
        Ok(Waited::Done { code, line }) => {
            let mut out = std::io::stdout().lock();
            if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
                return EXIT_ERROR;
            }
            code
        }
        Ok(Waited::Unknown) => {
            eprintln!(
                "dispatch wait: no run with id {:?} in {}",
                args.id,
                db.display()
            );
            EXIT_UNKNOWN
        }
        Err(e) => {
            eprintln!("dispatch wait: store error in {}: {e}", db.display());
            EXIT_ERROR
        }
    }
}

/// Parse the arguments after `wait`: one id, and `--timeout` / `--db` either
/// as `--flag value` or `--flag=value`.
fn parse(args: &[OsString]) -> Result<Args, String> {
    let mut id: Option<String> = None;
    let mut timeout = None;
    let mut db = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let text = arg.to_str();
        let (flag, inline) = match text {
            Some(t) if t.starts_with("--") => match t.split_once('=') {
                Some((f, v)) => (Some(f), Some(OsString::from(v))),
                None => (Some(t), None),
            },
            _ => (None, None),
        };
        match flag {
            Some("--timeout") => {
                let v = inline
                    .or_else(|| it.next().cloned())
                    .ok_or("--timeout needs a number of seconds")?;
                timeout = Some(parse_seconds(&v)?);
            }
            Some("--db") => {
                let v = inline
                    .or_else(|| it.next().cloned())
                    .ok_or("--db needs a path")?;
                if v.is_empty() {
                    return Err("--db needs a path".into());
                }
                db = Some(PathBuf::from(v));
            }
            Some(other) => return Err(format!("unknown option {other}")),
            None => {
                let value = text
                    .ok_or_else(|| format!("the id {arg:?} is not valid UTF-8"))?
                    .trim();
                if value.is_empty() {
                    return Err("the id is empty".into());
                }
                if id.replace(value.to_string()).is_some() {
                    return Err(format!("unexpected argument {value:?}"));
                }
            }
        }
    }
    let id = id.ok_or("missing the run id")?;
    Ok(Args { id, timeout, db })
}

/// A non-negative, finite number of seconds that fits a `Duration`; anything
/// else (negative, NaN, infinite, too large) is a usage error, never a panic.
fn parse_seconds(v: &OsString) -> Result<Duration, String> {
    let s = v.to_str().unwrap_or("").trim();
    s.parse::<f64>()
        .ok()
        .filter(|n| n.is_finite() && *n >= 0.0)
        .and_then(|n| Duration::try_from_secs_f64(n).ok())
        .ok_or_else(|| {
            format!(
                "--timeout takes a non-negative number of seconds that fits a duration, got {v:?}"
            )
        })
}

/// When a wait of `timeout` started at `now` ends; `None` waits until the run
/// is terminal, which is also what a timeout too large for an `Instant` means.
fn deadline_after(now: Instant, timeout: Option<Duration>) -> Option<Instant> {
    timeout.and_then(|t| now.checked_add(t))
}

/// The store the server would use from this environment and directory: the
/// same project root and state directory resolution as the server.
fn default_db() -> Result<PathBuf, String> {
    let cwd =
        std::env::current_dir().map_err(|e| format!("cannot read the current directory: {e}"))?;
    let root = crate::resolve_project_root(&cwd);
    Ok(crate::resolve_state_dir(root.as_deref()).join("dispatch.db"))
}

/// Open an existing store read-write without the create flag, so a missing
/// file is an error rather than a new empty database.
fn open_existing(path: &Path) -> Result<Connection, String> {
    if !path.is_file() {
        return Err(format!("no dispatch store at {}", path.display()));
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    conn.busy_timeout(BUSY_TIMEOUT)
        .map_err(|e| format!("cannot configure {}: {e}", path.display()))?;
    Ok(conn)
}

/// How a wait ended.
#[derive(Debug, PartialEq, Eq)]
enum Waited {
    /// The row was read: terminal ([`EXIT_TERMINAL`]) or still active at the
    /// timeout ([`EXIT_ACTIVE`]), with the line to print.
    Done { code: i32, line: String },
    /// No row has the id.
    Unknown,
}

/// Poll row `id` until it is terminal or `timeout` elapses (`None` waits
/// without limit). An active row whose owner pid is set and not `alive` is
/// marked `interrupted` through the server's conditional update, then read
/// again; a row with no owner pid is waited on.
fn wait_for(
    conn: &Connection,
    id: &str,
    timeout: Option<Duration>,
    alive: impl Fn(i64) -> bool,
    sleep: impl Fn(Duration),
) -> rusqlite::Result<Waited> {
    let deadline = deadline_after(Instant::now(), timeout);
    loop {
        let Some(mut row) = store::get(conn, id)? else {
            return Ok(Waited::Unknown);
        };
        if store::is_active(&row.status)
            && let Some(pid) = row.owner_pid
            && !alive(pid)
        {
            store::mark_interrupted(
                conn,
                id,
                &format!(
                    "owning dispatch server (pid {pid}) is no longer running \
                     (reconciled by dispatch wait)"
                ),
            )?;
            match store::get(conn, id)? {
                Some(r) => row = r,
                None => return Ok(Waited::Unknown),
            }
        }
        let active = store::is_active(&row.status);
        let now = Instant::now();
        let timed_out = deadline.is_some_and(|d| now >= d);
        if !active || timed_out {
            return Ok(Waited::Done {
                code: if active { EXIT_ACTIVE } else { EXIT_TERMINAL },
                line: status_line(conn, &row)?,
            });
        }
        let pause = deadline.map_or(POLL, |d| POLL.min(d.saturating_duration_since(now)));
        sleep(pause);
    }
}

/// `<id> <kind> <status>`, then ` model=<final model>` when set and
/// ` restarted_as=<id>` when a restart successor exists.
fn status_line(conn: &Connection, row: &store::TaskRow) -> rusqlite::Result<String> {
    let mut line = format!("{} {} {}", row.id, row.kind, row.status);
    if let Some(m) = row.final_model.as_deref().filter(|m| !m.is_empty()) {
        line.push_str(&format!(" model={m}"));
    }
    if let Some(succ) = store::restart_successor(conn, &row.id)? {
        line.push_str(&format!(" restarted_as={succ}"));
    }
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_id_timeout_and_db_in_either_form() {
        assert_eq!(
            parse(&os(&["d-3"])).unwrap(),
            Args {
                id: "d-3".into(),
                timeout: None,
                db: None
            }
        );
        assert_eq!(
            parse(&os(&["--timeout", "25", "d-3", "--db", "x.db"])).unwrap(),
            Args {
                id: "d-3".into(),
                timeout: Some(Duration::from_secs(25)),
                db: Some(PathBuf::from("x.db"))
            }
        );
        assert_eq!(
            parse(&os(&["d-3", "--timeout=0.5", "--db=x.db"])).unwrap(),
            Args {
                id: "d-3".into(),
                timeout: Some(Duration::from_millis(500)),
                db: Some(PathBuf::from("x.db"))
            }
        );
    }

    #[test]
    fn usage_errors_are_reported() {
        for bad in [
            &[][..],
            &["--timeout", "5"][..],
            &["d-1", "d-2"][..],
            &["d-1", "--timeout"][..],
            &["d-1", "--timeout", "-1"][..],
            &["d-1", "--timeout", "soon"][..],
            &["d-1", "--timeout", "nan"][..],
            &["d-1", "--timeout", "inf"][..],
            &["d-1", "--timeout", "1e100"][..],
            &["d-1", "--db"][..],
            &["d-1", "--db="][..],
            &["d-1", "--follow"][..],
            &[""][..],
        ] {
            assert!(parse(&os(bad)).is_err(), "{bad:?} must be a usage error");
        }
    }

    fn store_with(status: &str, owner_pid: Option<i64>) -> (Connection, String) {
        let mut c = Connection::open_in_memory().unwrap();
        store::init(&c).unwrap();
        let t = store::NewTask {
            plan_id: None,
            backend: "codex".into(),
            working_dir: "w".into(),
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
            kind: store::Kind::Consult,
        };
        let id = match store::insert_queued(&mut c, &t, 1, "i", None).unwrap() {
            store::InsertOutcome::Created(id) => id,
            store::InsertOutcome::Conflict(o) => panic!("conflict with {o}"),
        };
        c.execute(
            "UPDATE dispatch_tasks SET status = ?1, owner_pid = ?2 WHERE id = ?3",
            rusqlite::params![status, owner_pid, id],
        )
        .unwrap();
        (c, id)
    }

    fn no_sleep(_: Duration) {}

    #[test]
    fn a_terminal_row_returns_at_once_with_its_line() {
        let (c, id) = store_with("succeeded", Some(1));
        c.execute(
            "UPDATE dispatch_tasks SET final_model = 'm2' WHERE id = ?1",
            [&id],
        )
        .unwrap();
        let got = wait_for(&c, &id, None, |_| true, |_| panic!("must not sleep")).unwrap();
        assert_eq!(
            got,
            Waited::Done {
                code: EXIT_TERMINAL,
                line: format!("{id} consult succeeded model=m2")
            }
        );
    }

    #[test]
    fn an_active_row_with_a_live_owner_times_out_active() {
        let (c, id) = store_with("running", Some(1));
        let slept = Cell::new(Duration::ZERO);
        let got = wait_for(
            &c,
            &id,
            Some(Duration::from_millis(30)),
            |_| true,
            |d| {
                slept.set(slept.get() + d);
                std::thread::sleep(d);
            },
        )
        .unwrap();
        assert_eq!(
            got,
            Waited::Done {
                code: EXIT_ACTIVE,
                line: format!("{id} consult running")
            }
        );
        assert!(
            slept.get() <= Duration::from_millis(30),
            "never sleeps past the timeout"
        );
        assert_eq!(store::get(&c, &id).unwrap().unwrap().status, "running");
    }

    #[test]
    fn an_orphaned_row_is_marked_interrupted_and_reported() {
        let (c, id) = store_with("running", Some(424242));
        let got = wait_for(&c, &id, None, |pid| pid != 424242, no_sleep).unwrap();
        assert_eq!(
            got,
            Waited::Done {
                code: EXIT_TERMINAL,
                line: format!("{id} consult interrupted")
            }
        );
        let row = store::get(&c, &id).unwrap().unwrap();
        assert_eq!(row.status, store::STATUS_INTERRUPTED);
        assert!(row.error.unwrap_or_default().contains("pid 424242"));
    }

    #[test]
    fn a_row_without_an_owner_pid_is_waited_on() {
        let (c, id) = store_with("queued", None);
        let got = wait_for(&c, &id, Some(Duration::ZERO), |_| false, no_sleep).unwrap();
        assert_eq!(
            got,
            Waited::Done {
                code: EXIT_ACTIVE,
                line: format!("{id} consult queued")
            }
        );
    }

    #[test]
    fn an_unknown_id_is_reported() {
        let (c, _) = store_with("running", Some(1));
        assert_eq!(
            wait_for(&c, "d-99", None, |_| true, no_sleep).unwrap(),
            Waited::Unknown
        );
    }

    #[test]
    fn the_line_names_a_restart_successor() {
        let (c, id) = store_with("interrupted", Some(1));
        c.execute(
            "INSERT INTO dispatch_tasks (id, backend, working_dir, spec_json, prompt, restart_of) \
             VALUES ('d-50', 'codex', 'w', '{}', 'p', ?1)",
            [&id],
        )
        .unwrap();
        let got = wait_for(&c, &id, None, |_| true, no_sleep).unwrap();
        assert_eq!(
            got,
            Waited::Done {
                code: EXIT_TERMINAL,
                line: format!("{id} consult interrupted restarted_as=d-50")
            }
        );
    }

    #[test]
    fn a_missing_store_is_not_created() {
        let path =
            std::env::temp_dir().join(format!("dispatch-wait-missing-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        assert!(open_existing(&path).is_err());
        assert!(!path.exists(), "opening must not create the store");
    }

    #[test]
    fn timeouts_that_do_not_fit_are_refused_or_mean_no_deadline() {
        for bad in ["1e100", "-1", "nan", "NaN", "inf", "-0.5"] {
            assert!(
                parse_seconds(&OsString::from(bad)).is_err(),
                "{bad} must be a usage error"
            );
        }
        // 1e19 s fits a Duration but not an Instant: no deadline, no panic.
        let huge = parse_seconds(&OsString::from("1e19")).expect("1e19 fits a Duration");
        assert_eq!(huge, Duration::from_secs(10_000_000_000_000_000_000));
        assert_eq!(deadline_after(Instant::now(), Some(huge)), None);
        assert!(deadline_after(Instant::now(), Some(Duration::from_secs(1))).is_some());
        assert_eq!(deadline_after(Instant::now(), None), None);

        let (c, id) = store_with("succeeded", Some(1));
        let got = wait_for(&c, &id, Some(huge), |_| true, no_sleep).unwrap();
        assert!(matches!(
            got,
            Waited::Done {
                code: EXIT_TERMINAL,
                ..
            }
        ));
    }
}
