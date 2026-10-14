//! `dispatch wait` driven as a process: the built binary run against a
//! temporary store with synthetic rows, checking its exit codes, its one-line
//! output, the orphan transition, and that it never creates a store.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use rusqlite::Connection;

// The store module of the binary, compiled into this test so the rows are
// written through the same schema and insert path the server uses.
#[allow(dead_code)]
#[path = "../src/store.rs"]
mod store;

/// A fresh, empty directory for one test.
fn scratch(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("dispatch-wait-it-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// A store at `dir/dispatch.db` with the server's schema.
fn new_store(dir: &Path) -> (PathBuf, Connection) {
    let path = dir.join("dispatch.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA busy_timeout=5000; PRAGMA journal_mode=WAL;")
        .unwrap();
    store::init(&conn).unwrap();
    (path, conn)
}

/// Insert one row of `kind` and set its status and owner pid.
fn row(conn: &mut Connection, kind: store::Kind, status: &str, owner_pid: Option<i64>) -> String {
    let t = store::NewTask {
        plan_id: None,
        backend: "codex".into(),
        working_dir: "w".into(),
        title: None,
        spec_json: "{}".into(),
        prompt: "synthetic".into(),
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
    let id = match store::insert_queued(conn, &t, 1, "synthetic", None).unwrap() {
        store::InsertOutcome::Created(id) => id,
        store::InsertOutcome::Conflict(o) => panic!("conflict with {o}"),
    };
    conn.execute(
        "UPDATE dispatch_tasks SET status = ?1, owner_pid = ?2 WHERE id = ?3",
        rusqlite::params![status, owner_pid, id],
    )
    .unwrap();
    id
}

fn dispatch() -> Command {
    Command::new(env!("CARGO_BIN_EXE_dispatch"))
}

fn wait(id: &str, db: &Path, extra: &[&str]) -> Output {
    dispatch()
        .arg("wait")
        .arg(id)
        .arg("--db")
        .arg(db)
        .args(extra)
        .output()
        .unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// The pid of a process that has exited: a child that ends at once (a usage
/// error), waited on.
fn dead_pid() -> i64 {
    let mut child = dispatch()
        .arg("wait")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id() as i64;
    child.wait().unwrap();
    pid
}

#[test]
fn a_missing_store_is_an_error_and_is_not_created() {
    let dir = scratch("missing");
    let db = dir.join("nonexistent.db");
    let o = wait("d-1", &db, &[]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stdout(&o).is_empty());
    assert!(!o.stderr.is_empty(), "stderr says why");
    assert!(!db.exists(), "wait never creates the store");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn usage_errors_exit_1() {
    for args in [&["wait"][..], &["wait", "d-1", "--timeout", "x"][..]] {
        let o = dispatch().args(args).output().unwrap();
        assert_eq!(o.status.code(), Some(1), "{args:?}");
        assert!(stdout(&o).is_empty());
        assert!(!o.stderr.is_empty());
    }
}

#[test]
fn an_unknown_id_exits_3() {
    let dir = scratch("unknown");
    let (db, _conn) = new_store(&dir);
    let o = wait("d-404", &db, &["--timeout", "1"]);
    assert_eq!(o.status.code(), Some(3));
    assert!(stdout(&o).is_empty());
    assert!(!o.stderr.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_terminal_row_exits_0_with_one_line() {
    let dir = scratch("terminal");
    let (db, mut conn) = new_store(&dir);
    let id = row(&mut conn, store::Kind::Execute, "succeeded", Some(1));
    conn.execute(
        "UPDATE dispatch_tasks SET final_model = 'm2', result = 'never printed' WHERE id = ?1",
        [&id],
    )
    .unwrap();
    let o = wait(&id, &db, &[]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(stdout(&o), format!("{id} execute succeeded model=m2\n"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_active_row_with_a_live_owner_exits_2_at_the_timeout() {
    let dir = scratch("active");
    let (db, mut conn) = new_store(&dir);
    let id = row(
        &mut conn,
        store::Kind::Consult,
        "running",
        Some(std::process::id() as i64),
    );
    let o = wait(&id, &db, &["--timeout", "1"]);
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(stdout(&o), format!("{id} consult running\n"));
    let after = store::get(&conn, &id).unwrap().unwrap();
    assert_eq!(after.status, "running", "a live owner's row is left alone");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_orphaned_row_is_marked_interrupted_and_exits_0() {
    let dir = scratch("orphan");
    let (db, mut conn) = new_store(&dir);
    let pid = dead_pid();
    let id = row(&mut conn, store::Kind::Consult, "running", Some(pid));
    let other = row(&mut conn, store::Kind::Execute, "running", Some(pid));
    let o = wait(&id, &db, &["--timeout", "10"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(stdout(&o), format!("{id} consult interrupted\n"));
    let after = store::get(&conn, &id).unwrap().unwrap();
    assert_eq!(after.status, store::STATUS_INTERRUPTED);
    assert!(after.error.unwrap().contains(&format!("pid {pid}")));
    assert_eq!(
        store::get(&conn, &other).unwrap().unwrap().status,
        "running",
        "wait reconciles no other row"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn without_db_the_store_is_resolved_as_the_server_does() {
    let dir = scratch("resolve");
    let (_, mut conn) = new_store(&dir);
    let id = row(&mut conn, store::Kind::Execute, "failed", Some(1));
    let o = dispatch()
        .args(["wait", &id])
        .env("DISPATCH_STATE_DIR", &dir)
        .output()
        .unwrap();
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(stdout(&o), format!("{id} execute failed\n"));

    let empty = scratch("resolve-empty");
    let o = dispatch()
        .args(["wait", &id])
        .env("DISPATCH_STATE_DIR", &empty)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(!empty.join("dispatch.db").exists());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&empty);
}

/// Sets a row's `created_at` to a SQLite expression such as
/// `datetime('now', '-10 seconds')`.
fn set_created_at(conn: &Connection, id: &str, expr: &str) {
    conn.execute(
        &format!("UPDATE dispatch_tasks SET created_at = {expr} WHERE id = ?1"),
        [id],
    )
    .unwrap();
}

#[test]
fn a_row_older_than_its_owner_pid_is_reconciled_with_the_real_start_time_reader() {
    // The owner pid is this running test process, but the row was written in
    // 2000: no process running now can be that row's owner. This holds only
    // where the operating system's start time can be read, so it fails on a
    // platform whose reader does not work.
    let dir = scratch("reused");
    let (db, mut conn) = new_store(&dir);
    let id = row(
        &mut conn,
        store::Kind::Execute,
        "running",
        Some(i64::from(std::process::id())),
    );
    set_created_at(&conn, &id, "'2000-01-01 00:00:00'");
    let o = wait(&id, &db, &["--timeout", "10"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(stdout(&o), format!("{id} execute interrupted\n"));
    let after = store::get(&conn, &id).unwrap().unwrap();
    assert_eq!(after.status, store::STATUS_INTERRUPTED);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn another_process_start_time_is_read_and_compared_with_the_rows_created_at() {
    // A long-lived child stands for an owner: `dispatch wait` on an active row
    // that this test process owns runs until it is killed.
    let sleeper_dir = scratch("sleeper");
    let (sleeper_db, mut sleeper_conn) = new_store(&sleeper_dir);
    let sleeper_row = row(
        &mut sleeper_conn,
        store::Kind::Execute,
        "running",
        Some(i64::from(std::process::id())),
    );
    let mut sleeper = dispatch()
        .args(["wait", &sleeper_row, "--db"])
        .arg(&sleeper_db)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let pid = i64::from(sleeper.id());

    let dir = scratch("sleeper-rows");
    let (db, mut conn) = new_store(&dir);
    let older = row(&mut conn, store::Kind::Execute, "running", Some(pid));
    let newer = row(&mut conn, store::Kind::Execute, "running", Some(pid));
    // The child started moments ago, so a row from ten seconds ago predates it.
    // The newer row is dated a few seconds ahead so that SQLite's whole seconds
    // and the one-second rounding of a Linux start time cannot make the
    // test depend on how long ago the child started.
    set_created_at(&conn, &older, "datetime('now', '-10 seconds')");
    set_created_at(&conn, &newer, "datetime('now', '+5 seconds')");

    let o = wait(&older, &db, &["--timeout", "10"]);
    let replaced = (o.status.code(), stdout(&o));
    let o = wait(&newer, &db, &["--timeout", "1"]);
    let kept = (o.status.code(), stdout(&o));
    let _ = sleeper.kill();
    let _ = sleeper.wait();

    assert_eq!(
        replaced,
        (Some(0), format!("{older} execute interrupted\n"))
    );
    assert_eq!(kept, (Some(2), format!("{newer} execute running\n")));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&sleeper_dir);
}
