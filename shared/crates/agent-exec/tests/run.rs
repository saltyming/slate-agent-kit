//! Integration tests for `run`, `run_with_fallback`, `version` and the guard,
//! against the crate's own test-support executables: `stub-backend`, copied
//! onto a temporary `PATH` as `codex` and `claude`; `agent-guard`, copied
//! beside them; and `stub-parent`, a killable parent for the guard tests.
//!
//! This test target has its own `main` (`harness = false`): `PATH` is set to
//! the stub directory alone before any other thread starts — so no real
//! backend CLI can ever be spawned — and the tests then run one after
//! another, which also lets a test move a stub copy aside safely. No test
//! starts this test executable again: the guard is only ever the copied
//! `agent-guard`. A watchdog thread ends the run if it exceeds its time
//! budget, and every process a test starts is killed on drop.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent_exec::errkind::classify;
use agent_exec::{
    Backend, BackendErrorKind, CapturePolicy, FailureTextPolicy, GuardMode, Isolation, Outcome,
    OutputMode, Reentry, RunEvent, RunRecord, RunSpec, Sandbox, UsageSource, failure_text, guard,
    run, run_with_fallback, version,
};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio_util::sync::CancellationToken;

const MARKER: &str = "AGENT_EXEC_IT_DEPTH";
const STUB_BACKEND: &str = env!("CARGO_BIN_EXE_stub-backend");
const STUB_PARENT: &str = env!("CARGO_BIN_EXE_stub-parent");
const AGENT_GUARD: &str = env!("CARGO_BIN_EXE_agent-guard");
/// The whole run's budget; the watchdog exits the process past it.
const RUN_BUDGET: Duration = Duration::from_secs(110);
/// How long any single wait in a test may take.
const WAIT: Duration = Duration::from_secs(20);

type TestFn = fn(Ctx) -> Pin<Box<dyn Future<Output = ()>>>;

/// Shared test context: the stub directory on `PATH` and a scratch directory.
#[derive(Clone)]
struct Ctx {
    bin_dir: PathBuf,
    scratch: PathBuf,
}

impl Ctx {
    fn file(&self, name: &str) -> PathBuf {
        self.scratch.join(name)
    }

    fn bin(&self, name: &str) -> PathBuf {
        self.bin_dir.join(exe_name(name))
    }
}

fn exe_name(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

fn main() {
    // The stub directory's path names `stub-backend`, so a stray copy is easy
    // to find by name from outside.
    let root = std::env::temp_dir().join(format!("agent-exec-it-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let bin_dir = root.join("stub-backend");
    let scratch = root.join("scratch");
    std::fs::create_dir_all(&bin_dir).expect("create bin dir");
    std::fs::create_dir_all(&scratch).expect("create scratch dir");
    for name in ["codex", "claude"] {
        std::fs::copy(STUB_BACKEND, bin_dir.join(exe_name(name))).expect("copy stub");
    }
    std::fs::copy(AGENT_GUARD, bin_dir.join(exe_name("agent-guard"))).expect("copy guard");
    // SAFETY: no other thread exists yet; the watchdog and the runtime are
    // started afterwards.
    unsafe {
        std::env::set_var("PATH", &bin_dir);
        std::env::set_var("STUB_MARKER_NAME", MARKER);
        std::env::remove_var(MARKER);
        std::env::remove_var(guard::GUARD_ENV);
    }
    std::thread::spawn(|| {
        std::thread::sleep(RUN_BUDGET);
        eprintln!("watchdog: the test run exceeded {RUN_BUDGET:?}; exiting");
        std::process::exit(101);
    });
    let ctx = Ctx { bin_dir, scratch };

    let tests: Vec<(&str, TestFn)> = vec![
        ("success_record_with_stdout_and_final_text", |c| {
            Box::pin(success_record_with_stdout_and_final_text(c))
        }),
        ("non_zero_exit_is_a_failed_record", |c| {
            Box::pin(non_zero_exit_is_a_failed_record(c))
        }),
        ("large_prompt_is_delivered_on_stdin", |c| {
            Box::pin(large_prompt_is_delivered_on_stdin(c))
        }),
        ("exit_without_reading_a_large_prompt_is_a_record", |c| {
            Box::pin(exit_without_reading_a_large_prompt_is_a_record(c))
        }),
        ("slow_stdin_reader_with_large_stdout_completes", |c| {
            Box::pin(slow_stdin_reader_with_large_stdout_completes(c))
        }),
        ("dispatch_capture_truncates_by_byte", |c| {
            Box::pin(dispatch_capture_truncates_by_byte(c))
        }),
        ("aside_capture_truncates_by_character", |c| {
            Box::pin(aside_capture_truncates_by_character(c))
        }),
        ("structured_output_gives_usage_and_session", |c| {
            Box::pin(structured_output_gives_usage_and_session(c))
        }),
        ("missing_binary_is_not_found_with_hint", |c| {
            Box::pin(missing_binary_is_not_found_with_hint(c))
        }),
        ("cancellation_kills_the_child_and_its_children", |c| {
            Box::pin(cancellation_kills_the_child_and_its_children(c))
        }),
        ("cancel_while_a_descendant_holds_stdout_kills_it", |c| {
            Box::pin(cancel_while_a_descendant_holds_stdout_kills_it(c))
        }),
        ("an_exit_before_the_cancel_keeps_its_record", |c| {
            Box::pin(an_exit_before_the_cancel_keeps_its_record(c))
        }),
        ("a_descendant_holding_stdout_is_drained_to_its_end", |c| {
            Box::pin(a_descendant_holding_stdout_is_drained_to_its_end(c))
        }),
        ("cancelled_before_start_spawns_nothing", |c| {
            Box::pin(cancelled_before_start_spawns_nothing(c))
        }),
        ("fallback_advances_on_retry_worthy_failures", |c| {
            Box::pin(fallback_advances_on_retry_worthy_failures(c))
        }),
        ("fallback_stops_on_success_and_on_permanent_failure", |c| {
            Box::pin(fallback_stops_on_success_and_on_permanent_failure(c))
        }),
        ("fallback_stops_on_the_last_model", |c| {
            Box::pin(fallback_stops_on_the_last_model(c))
        }),
        ("fallback_stops_on_cancel", |c| {
            Box::pin(fallback_stops_on_cancel(c))
        }),
        ("version_probe_carries_the_marker", |c| {
            Box::pin(version_probe_carries_the_marker(c))
        }),
        ("guard_modes_without_the_executable", |c| {
            Box::pin(guard_modes_without_the_executable(c))
        }),
        ("guard_mode_off_never_looks_up_the_guard", |c| {
            Box::pin(guard_mode_off_never_looks_up_the_guard(c))
        }),
        ("guarded_run_leaves_no_descendant", |c| {
            Box::pin(guarded_run_leaves_no_descendant(c))
        }),
        (
            "guard_mirrors_exit_code_signal_and_clears_its_marker",
            |c| Box::pin(guard_mirrors_exit_code_signal_and_clears_its_marker(c)),
        ),
        ("guard_kills_program_and_its_child_when_parent_dies", |c| {
            Box::pin(guard_kills_program_and_its_child_when_parent_dies(c))
        }),
        ("guard_marked_process_refuses_to_start_a_backend", |c| {
            Box::pin(guard_marked_process_refuses_to_start_a_backend(c))
        }),
        ("killed_server_takes_its_backend_along", |c| {
            Box::pin(killed_server_takes_its_backend_along(c))
        }),
    ];

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let mut failed = Vec::new();
    println!("\nrunning {} tests", tests.len());
    for (name, test) in &tests {
        let c = ctx.clone();
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rt.block_on(test(c))));
        match result {
            Ok(()) => println!("test {name} ... ok"),
            Err(_) => {
                println!("test {name} ... FAILED");
                failed.push(*name);
            }
        }
    }
    // Bounded: a reader blocked on a pipe that a leftover process still holds
    // open (tokio reads child pipes on blocking threads on Windows) must not
    // hold the whole run until the watchdog fires.
    rt.shutdown_timeout(Duration::from_secs(5));
    let _ = std::fs::remove_dir_all(&root);
    if failed.is_empty() {
        println!("\ntest result: ok. {} passed; 0 failed\n", tests.len());
    } else {
        println!(
            "\ntest result: FAILED. {} passed; {} failed: {failed:?}\n",
            tests.len() - failed.len(),
            failed.len()
        );
        std::process::exit(1);
    }
}

// ── helpers ───────────────────────────────────────────────

fn spec(backend: Backend, env: &[(&str, String)]) -> RunSpec {
    RunSpec {
        backend,
        prompt: "hello prompt".into(),
        working_dir: None,
        sandbox: Sandbox::ReadOnly,
        isolation: match backend {
            Backend::Codex => Isolation::IgnoreUserConfig,
            Backend::Claude => Isolation::Permissions,
        },
        output_mode: OutputMode::Default,
        capture: CapturePolicy::dispatch(),
        failure_text: FailureTextPolicy::dispatch(),
        model: None,
        reasoning_effort: None,
        resume_session: None,
        pin_session: None,
        skip_git_repo_check: false,
        env: env
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
        reentry: Reentry {
            name: MARKER.into(),
            ceiling: 1,
        },
        guard: GuardMode::Required,
        backend_version: Some("stub 1.0".into()),
    }
}

fn s(v: impl ToString) -> String {
    v.to_string()
}

/// A name no other test run shares, for finding this test's processes.
fn tag(what: &str) -> String {
    format!("agent-exec-it-{}-{what}", std::process::id())
}

fn record(o: Outcome) -> RunRecord {
    match o {
        Outcome::Record(r) => r,
        other => panic!("expected a record, got {other:?}"),
    }
}

fn drain(rx: &mut UnboundedReceiver<RunEvent>) -> Vec<RunEvent> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        out.push(ev);
    }
    out
}

fn started_guarded(events: &[RunEvent]) -> Option<bool> {
    events.iter().find_map(|e| match e {
        RunEvent::Started { guarded, .. } => Some(*guarded),
        _ => None,
    })
}

/// Await `fut`, failing the test if it takes longer than `WAIT`.
async fn within<T>(fut: impl Future<Output = T>) -> T {
    tokio::time::timeout(WAIT, fut)
        .await
        .expect("finished in time")
}

async fn run_one(sp: &RunSpec) -> (Outcome, Vec<RunEvent>) {
    let (tx, mut rx) = unbounded_channel();
    let o = tokio::time::timeout(WAIT, run(sp, &tx, &CancellationToken::new()))
        .await
        .expect("run finishes in time");
    (o, drain(&mut rx))
}

async fn wait_for_file(path: &Path) -> String {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Ok(t) = std::fs::read_to_string(path)
            && !t.trim().is_empty()
        {
            return t;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn read_pid(path: &Path) -> u32 {
    wait_for_file(path).await.trim().parse().expect("pid")
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    // kill(pid, 0): 0 → exists; ESRCH → gone. A zombie still exists until its
    // parent (or init, after re-parenting) reaps it, so callers poll.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code: u32 = 0;
        let ok = GetExitCodeProcess(handle, &mut code as *mut u32);
        CloseHandle(handle);
        ok != 0 && code as i32 == STILL_ACTIVE
    }
}

async fn wait_until_gone(pid: u32) {
    let deadline = Instant::now() + WAIT;
    while process_alive(pid) {
        assert!(
            Instant::now() < deadline,
            "process {pid} still alive after {WAIT:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// How many live processes carry `marker` in their command line. `ps` is
/// called by absolute path: this process's `PATH` holds only the stubs.
#[cfg(unix)]
fn count_marked(marker: &str) -> usize {
    let ps = ["/bin/ps", "/usr/bin/ps"]
        .into_iter()
        .find(|p| Path::new(p).is_file())
        .expect("ps");
    let out = std::process::Command::new(ps)
        .args(["-A", "-o", "args="])
        .output()
        .expect("run ps");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.contains(marker))
        .count()
}

#[cfg(unix)]
async fn wait_until_unmarked(marker: &str) {
    let deadline = Instant::now() + WAIT;
    loop {
        let n = count_marked(marker);
        if n == 0 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{n} process(es) marked {marker} still alive after {WAIT:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A process a test started directly, killed (with its process group, when
/// `group` is set and it still runs) when the test ends however it ends.
struct Spawned {
    child: std::process::Child,
    group: bool,
}

impl Spawned {
    fn new(child: std::process::Child, group: bool) -> Self {
        Spawned { child, group }
    }

    fn kill(&mut self) {
        // Windows has no process groups here; the test kills the child only.
        #[cfg(not(unix))]
        let _ = self.group;
        if let Ok(None) = self.child.try_wait() {
            #[cfg(unix)]
            if self.group {
                unsafe {
                    libc::kill(-(self.child.id() as i32), libc::SIGKILL);
                }
            }
            let _ = self.child.kill();
        }
    }

    /// The exit status, polled until `WAIT` runs out.
    async fn wait(&mut self) -> std::process::ExitStatus {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(st) = self.child.try_wait().expect("try_wait") {
                return st;
            }
            assert!(Instant::now() < deadline, "process did not exit in time");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

impl Drop for Spawned {
    fn drop(&mut self) {
        self.kill();
        let _ = self.child.try_wait();
    }
}

/// Processes a test learned the ids of (from pid files), killed when the test
/// fails, so a failed test leaves nothing behind for the next one. A passing
/// test has already seen them gone, so nothing is killed then.
#[derive(Default)]
struct Reap(Vec<u32>);

impl Reap {
    fn add(&mut self, pid: u32) -> u32 {
        self.0.push(pid);
        pid
    }
}

impl Drop for Reap {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            return;
        }
        for &pid in &self.0 {
            if process_alive(pid) {
                kill_pid(pid);
            }
        }
    }
}

#[cfg(unix)]
fn kill_pid(pid: u32) {
    unsafe {
        libc::kill(pid as i32, libc::SIGKILL);
    }
}

#[cfg(windows)]
fn kill_pid(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !handle.is_null() {
            TerminateProcess(handle, 1);
            CloseHandle(handle);
        }
    }
}

/// Move a file aside for the length of a test and put it back on drop.
struct Aside {
    from: PathBuf,
    to: PathBuf,
}

impl Aside {
    fn new(from: PathBuf) -> Self {
        let to = from.with_extension("moved");
        std::fs::rename(&from, &to).expect("move aside");
        Aside { from, to }
    }
}

impl Drop for Aside {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.from);
        let _ = std::fs::rename(&self.to, &self.from);
    }
}

// ── run ───────────────────────────────────────────────────

async fn success_record_with_stdout_and_final_text(c: Ctx) {
    let args_file = c.file("success-args");
    let stdin_file = c.file("success-stdin");
    let sp = spec(
        Backend::Codex,
        &[
            ("STUB_ECHO_STDIN", s(1)),
            ("STUB_ARGS_FILE", s(args_file.display())),
            ("STUB_STDIN_FILE", s(stdin_file.display())),
        ],
    );
    let (o, events) = run_one(&sp).await;
    let r = record(o);
    assert!(r.success);
    assert_eq!(r.exit_code, Some(0));
    assert_eq!(r.backend, "codex");
    assert_eq!(r.stdout, "hello prompt");
    assert_eq!(r.stdout_total, 12);
    assert!(!r.stdout_truncated);
    assert_eq!(r.final_text.as_deref(), Some("hello prompt"));
    assert_eq!(r.backend_version.as_deref(), Some("stub 1.0"));
    assert_eq!(r.usage, None);
    assert_eq!(
        r.argv,
        vec![
            "codex",
            "-s",
            "read-only",
            "-a",
            "never",
            "exec",
            "--ignore-user-config"
        ]
    );
    // The backend saw exactly that argv (no positional prompt) and the prompt
    // on stdin.
    let seen: Vec<String> = std::fs::read_to_string(&args_file)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(seen, r.argv[1..].to_vec());
    assert_eq!(
        std::fs::read_to_string(&stdin_file).unwrap(),
        "hello prompt"
    );

    assert_eq!(events.len(), 2, "{events:?}");
    match &events[0] {
        RunEvent::Started {
            pid,
            argv,
            backend_version,
            guarded,
        } => {
            assert!(pid.is_some());
            assert_eq!(argv, &r.argv);
            assert_eq!(backend_version.as_deref(), Some("stub 1.0"));
            assert!(*guarded, "a Required run is guarded");
        }
        other => panic!("expected Started, got {other:?}"),
    }
    assert!(matches!(&events[1], RunEvent::Finished(Outcome::Record(fr)) if *fr == r));
}

async fn non_zero_exit_is_a_failed_record(c: Ctx) {
    let _ = c;
    let bad_model = "There's an issue with the selected model (x). It may not exist or you \
                     may not have access to it.";
    let sp = spec(
        Backend::Claude,
        &[
            ("STUB_EXIT", s(3)),
            ("STUB_STDOUT", s(bad_model)),
            ("STUB_STDERR", s("Warning: something incidental")),
        ],
    );
    let (o, _) = run_one(&sp).await;
    let text = failure_text(&sp.failure_text, sp.backend.as_str(), &o).expect("failure text");
    let r = record(o);
    assert!(!r.success);
    assert_eq!(r.exit_code, Some(3));
    assert_eq!(r.stdout, bad_model);
    assert_eq!(r.stderr, "Warning: something incidental");
    assert_eq!(r.final_text, None);
    assert_eq!(classify(&text), BackendErrorKind::ModelUnavailable);
}

async fn large_prompt_is_delivered_on_stdin(c: Ctx) {
    // Far larger than any pipe buffer, echoed back while it is still being
    // written: stdin and stdout must be serviced concurrently.
    let stdin_file = c.file("large-stdin");
    let mut sp = spec(
        Backend::Claude,
        &[
            ("STUB_ECHO_STDIN", s(1)),
            ("STUB_STDIN_FILE", s(stdin_file.display())),
        ],
    );
    sp.prompt = "p".repeat(1024 * 1024);
    let (o, _) = run_one(&sp).await;
    let r = record(o);
    assert!(r.success);
    assert_eq!(
        std::fs::metadata(&stdin_file).unwrap().len(),
        1024 * 1024,
        "the whole prompt reached stdin"
    );
    assert_eq!(r.stdout_total, 1024 * 1024);
    assert_eq!(r.stdout.len(), 200 * 1024);
    assert!(r.stdout_truncated);
}

async fn exit_without_reading_a_large_prompt_is_a_record(c: Ctx) {
    let _ = c;
    // The child exits non-zero without reading a prompt larger than the pipe
    // buffer: the write fails with a broken pipe, which is not a failure of
    // its own; the exit status and the child's stderr decide.
    for mode in [GuardMode::Required, GuardMode::Off] {
        let mut sp = spec(
            Backend::Codex,
            &[
                ("STUB_SKIP_STDIN", s(1)),
                ("STUB_EXIT", s(2)),
                ("STUB_STDERR", s("error: unexpected argument '--bogus'")),
            ],
        );
        sp.guard = mode;
        sp.prompt = "q".repeat(4 * 1024 * 1024);
        let (o, _) = run_one(&sp).await;
        let r = record(o);
        assert!(!r.success, "{mode:?}");
        assert_eq!(r.exit_code, Some(2), "{mode:?}");
        assert_eq!(r.stderr, "error: unexpected argument '--bogus'", "{mode:?}");
    }
}

async fn slow_stdin_reader_with_large_stdout_completes(c: Ctx) {
    // The child reads stdin slowly and writes 4 KiB of stdout per 1 KiB read:
    // more than a pipe buffer of output before the prompt is fully written.
    let stdin_file = c.file("slow-stdin");
    let mut sp = spec(
        Backend::Codex,
        &[
            ("STUB_INTERLEAVE", s(1)),
            ("STUB_STDIN_FILE", s(stdin_file.display())),
        ],
    );
    sp.prompt = "r".repeat(256 * 1024);
    let (o, _) = run_one(&sp).await;
    let r = record(o);
    assert!(r.success);
    assert_eq!(std::fs::metadata(&stdin_file).unwrap().len(), 256 * 1024);
    // At least one 4 KiB block per 1 KiB read of the 256 KiB prompt.
    assert!(r.stdout_total >= 4 * 256 * 1024, "{}", r.stdout_total);
    assert!(r.stdout_truncated);
}

async fn dispatch_capture_truncates_by_byte(c: Ctx) {
    let _ = c;
    let sp = spec(
        Backend::Codex,
        &[
            ("STUB_STDOUT_REPEAT", s(300 * 1024)),
            ("STUB_STDERR", s("HEAD")),
            ("STUB_STDERR_REPEAT", s(20 * 1024)),
        ],
    );
    let (o, _) = run_one(&sp).await;
    let r = record(o);
    assert_eq!(r.stdout.len(), 200 * 1024);
    assert_eq!(r.stdout_total, 300 * 1024);
    assert!(r.stdout_truncated);
    assert_eq!(r.stderr.len(), 16 * 1024);
    assert!(
        r.stderr.starts_with("HEAD"),
        "dispatch keeps the stderr head"
    );
    assert!(r.stderr_truncated);

    // On failure dispatch keeps the same stdout head.
    let mut failing = sp.clone();
    failing.env.push(("STUB_EXIT".into(), "1".into()));
    let (o, _) = run_one(&failing).await;
    let r = record(o);
    assert!(!r.success);
    assert_eq!(r.stdout.len(), 200 * 1024);
    assert!(r.stdout_truncated);
}

async fn aside_capture_truncates_by_character(c: Ctx) {
    let _ = c;
    let mut sp = spec(
        Backend::Claude,
        &[
            ("STUB_FILL", s("é")),
            ("STUB_STDOUT_REPEAT", s(60 * 1024)),
            ("STUB_STDERR", s("")),
            ("STUB_STDERR_REPEAT", s(5000)),
        ],
    );
    sp.capture = CapturePolicy::aside();
    sp.failure_text = FailureTextPolicy::aside();
    let (o, _) = run_one(&sp).await;
    let r = record(o);
    assert!(r.success);
    assert_eq!(r.stdout.chars().count(), 50 * 1024);
    assert_eq!(r.stdout_total, 2 * 60 * 1024);
    assert!(r.stdout_truncated);
    assert_eq!(r.stderr.chars().count(), 2 * 1024);
    assert!(r.stderr_truncated);

    // Below the cap nothing is marked, whatever the byte count.
    let mut small = sp.clone();
    small.env = vec![
        ("STUB_FILL".into(), "é".into()),
        ("STUB_STDOUT_REPEAT".into(), (40 * 1024).to_string()),
    ];
    let (o, _) = run_one(&small).await;
    let r = record(o);
    assert_eq!(r.stdout.chars().count(), 40 * 1024);
    assert!(!r.stdout_truncated);

    // On failure aside keeps only the first 2 KB of stdout.
    sp.env.push(("STUB_EXIT".into(), "1".into()));
    let (o, _) = run_one(&sp).await;
    let r = record(o);
    assert!(!r.success);
    assert_eq!(r.stdout.chars().count(), 2 * 1024);
    assert!(r.stdout_truncated);
    let text = failure_text(&sp.failure_text, "claude", &Outcome::Record(r.clone())).unwrap();
    assert!(text.ends_with(&r.stdout), "aside sends all it kept");
}

async fn structured_output_gives_usage_and_session(c: Ctx) {
    let _ = c;
    let stream = concat!(
        r#"{"type":"thread.started","thread_id":"th-7"}"#,
        "\n",
        r#"{"type":"item.completed","item":{"id":"i1","type":"agent_message","text":"done"}}"#,
        "\n",
        r#"{"type":"turn.completed","usage":{"input_tokens":100,"cached_input_tokens":40,"cache_write_input_tokens":10,"output_tokens":7,"reasoning_output_tokens":3}}"#,
        "\n"
    );
    let mut sp = spec(Backend::Codex, &[("STUB_STDOUT", s(stream))]);
    sp.output_mode = OutputMode::JsonStream;
    let (o, _) = run_one(&sp).await;
    let r = record(o);
    assert_eq!(r.argv.last().map(String::as_str), Some("--json"));
    assert_eq!(r.final_text.as_deref(), Some("done"));
    assert_eq!(r.session_id.as_deref(), Some("th-7"));
    let u = r.usage.expect("codex usage");
    assert_eq!(u.usage_source, UsageSource::CodexJsonStream);
    assert_eq!(u.input_uncached, Some(50));
    assert_eq!(u.cache_write_5m, Some(10));

    let result = r#"{"type":"result","result":"answer","session_id":"cs-1","usage":{"input_tokens":5,"cache_read_input_tokens":6,"output_tokens":8}}"#;
    let mut sp = spec(Backend::Claude, &[("STUB_STDOUT", s(result))]);
    sp.output_mode = OutputMode::Json;
    let (o, _) = run_one(&sp).await;
    let r = record(o);
    assert_eq!(r.final_text.as_deref(), Some("answer"));
    assert_eq!(r.session_id.as_deref(), Some("cs-1"));
    let u = r.usage.expect("claude usage");
    assert_eq!(u.usage_source, UsageSource::ClaudeJsonResult);
    assert_eq!(u.output, Some(8));
    assert_eq!(u.cache_write_5m, None);
}

async fn missing_binary_is_not_found_with_hint(c: Ctx) {
    let moved = Aside::new(c.bin("claude"));
    let (tx, mut rx) = unbounded_channel();
    let sp = spec(Backend::Claude, &[]);
    let o = within(run(&sp, &tx, &CancellationToken::new())).await;
    // A missing binary is never retried.
    let chain = within(run_with_fallback(
        |_, _| spec(Backend::Claude, &[]),
        &[None, Some("fallback".into())],
        &tx,
        &CancellationToken::new(),
    ))
    .await;
    drop(moved);
    match o {
        Outcome::NotFound { binary, hint } => {
            assert_eq!(binary, "claude");
            assert!(hint.contains("@anthropic-ai/claude-code"), "{hint}");
        }
        other => panic!("expected NotFound, got {other:?}"),
    }
    assert!(matches!(chain.outcome, Outcome::NotFound { .. }));
    assert_eq!(chain.model, None);
    assert!(chain.discarded.is_empty());
    let events = drain(&mut rx);
    assert_eq!(events.len(), 2, "{events:?}");
    assert!(
        events
            .iter()
            .all(|e| matches!(e, RunEvent::Finished(Outcome::NotFound { .. }))),
        "{events:?}"
    );
}

async fn cancellation_kills_the_child_and_its_children(c: Ctx) {
    for mode in [GuardMode::Required, GuardMode::Off] {
        let mut reap = Reap::default();
        let marker = tag(&format!("cancel-{mode:?}"));
        let pid_file = c.file(&format!("cancel-pid-{mode:?}"));
        let child_file = c.file(&format!("cancel-child-{mode:?}"));
        let env = vec![
            ("STUB_PID_FILE", s(pid_file.display())),
            ("STUB_SLEEP_MS", s(120_000)),
            ("STUB_CHILD_TAG", marker.clone()),
            ("STUB_CHILD_PID_FILE", s(child_file.display())),
        ];
        let mut sp = spec(Backend::Codex, &env);
        sp.guard = mode;
        sp.model = Some(marker.clone());
        let (tx, mut rx) = unbounded_channel();
        let ct = CancellationToken::new();
        let task = {
            let ct = ct.clone();
            tokio::spawn(async move { run(&sp, &tx, &ct).await })
        };
        let pid = reap.add(read_pid(&pid_file).await);
        let child = reap.add(read_pid(&child_file).await);
        assert!(process_alive(pid));
        ct.cancel();
        let o = tokio::time::timeout(WAIT, task)
            .await
            .expect("run returns promptly after cancel")
            .unwrap();
        assert!(matches!(o, Outcome::Cancelled), "{o:?}");
        wait_until_gone(pid).await;
        wait_until_gone(child).await;
        #[cfg(unix)]
        wait_until_unmarked(&marker).await;
        let events = drain(&mut rx);
        assert!(matches!(events.first(), Some(RunEvent::Started { .. })));
        assert!(matches!(
            events.last(),
            Some(RunEvent::Finished(Outcome::Cancelled))
        ));
    }
}

/// A backend that exits at once after starting a child that inherits its
/// stdout and stderr and sleeps `child_ms`: returns the spec, the backend's
/// pid file and the child's.
fn exiting_backend_with_held_stdout(
    c: &Ctx,
    name: &str,
    child_ms: u64,
) -> (RunSpec, PathBuf, PathBuf, String) {
    let marker = tag(name);
    let pid_file = c.file(&format!("{name}-pid"));
    let child_file = c.file(&format!("{name}-child"));
    let env = vec![
        ("STUB_PID_FILE", s(pid_file.display())),
        ("STUB_STDOUT", s("done")),
        ("STUB_CHILD_TAG", marker.clone()),
        ("STUB_CHILD_PID_FILE", s(child_file.display())),
        ("STUB_CHILD_INHERIT_STDIO", s(1)),
        ("STUB_CHILD_SLEEP_MS", s(child_ms)),
    ];
    let mut sp = spec(Backend::Codex, &env);
    sp.model = Some(marker.clone());
    (sp, pid_file, child_file, marker)
}

async fn cancel_while_a_descendant_holds_stdout_kills_it(c: Ctx) {
    let mut reap = Reap::default();
    let (sp, pid_file, child_file, marker) =
        exiting_backend_with_held_stdout(&c, "held-cancel", 120_000);
    let (tx, mut rx) = unbounded_channel();
    let ct = CancellationToken::new();
    let task = {
        let ct = ct.clone();
        tokio::spawn(async move { run(&sp, &tx, &ct).await })
    };
    let pid = reap.add(read_pid(&pid_file).await);
    let child = reap.add(read_pid(&child_file).await);
    // The backend is gone; its child still holds stdout, so run is draining.
    wait_until_gone(pid).await;
    assert!(process_alive(child));
    ct.cancel();
    let clock = Instant::now();
    let o = tokio::time::timeout(WAIT, task)
        .await
        .expect("run returns promptly after cancel")
        .unwrap();
    assert!(matches!(o, Outcome::Cancelled), "{o:?}");
    assert!(
        clock.elapsed() < Duration::from_secs(5),
        "{:?}",
        clock.elapsed()
    );
    wait_until_gone(child).await;
    #[cfg(unix)]
    wait_until_unmarked(&marker).await;
    #[cfg(not(unix))]
    let _ = marker;
    assert!(matches!(
        drain(&mut rx).last(),
        Some(RunEvent::Finished(Outcome::Cancelled))
    ));
}

async fn an_exit_before_the_cancel_keeps_its_record(c: Ctx) {
    let mut reap = Reap::default();
    // The child releases stdout well within the drain grace after a cancel.
    let (sp, pid_file, child_file, marker) =
        exiting_backend_with_held_stdout(&c, "exit-first", 100);
    let (tx, mut rx) = unbounded_channel();
    let ct = CancellationToken::new();
    let task = {
        let ct = ct.clone();
        tokio::spawn(async move { run(&sp, &tx, &ct).await })
    };
    let pid = reap.add(read_pid(&pid_file).await);
    reap.add(read_pid(&child_file).await);
    let spawned = match within(rx.recv()).await {
        Some(RunEvent::Started { pid: Some(p), .. }) => p,
        other => panic!("expected Started, got {other:?}"),
    };
    wait_until_gone(pid).await;
    // The spawned process (the guard on Unix) stays a zombie until run reaps
    // it, so once it is gone run has observed the exit; the cancellation
    // comes after it, while the child still holds stdout.
    wait_until_gone(spawned).await;
    ct.cancel();
    let o = tokio::time::timeout(WAIT, task)
        .await
        .expect("run finishes in time")
        .unwrap();
    let r = record(o);
    assert!(r.success, "{r:?}");
    assert_eq!(r.stdout, "done");
    assert!(matches!(
        drain(&mut rx).last(),
        Some(RunEvent::Finished(Outcome::Record(_)))
    ));
    #[cfg(unix)]
    wait_until_unmarked(&marker).await;
    #[cfg(not(unix))]
    let _ = marker;
}

async fn a_descendant_holding_stdout_is_drained_to_its_end(c: Ctx) {
    let mut reap = Reap::default();
    let (sp, _pid_file, child_file, marker) =
        exiting_backend_with_held_stdout(&c, "held-natural", 500);
    let (tx, mut rx) = unbounded_channel();
    let task = tokio::spawn(async move { run(&sp, &tx, &CancellationToken::new()).await });
    let child = reap.add(read_pid(&child_file).await);
    let o = tokio::time::timeout(WAIT, task)
        .await
        .expect("run finishes in time")
        .unwrap();
    let r = record(o);
    assert!(r.success, "{r:?}");
    assert_eq!(r.stdout, "done");
    // The drain ended because the child released stdout by exiting.
    wait_until_gone(child).await;
    assert!(matches!(
        drain(&mut rx).last(),
        Some(RunEvent::Finished(Outcome::Record(_)))
    ));
    #[cfg(unix)]
    wait_until_unmarked(&marker).await;
    #[cfg(not(unix))]
    let _ = marker;
}

async fn cancelled_before_start_spawns_nothing(c: Ctx) {
    let pid_file = c.file("precancel-pid");
    let sp = spec(Backend::Codex, &[("STUB_PID_FILE", s(pid_file.display()))]);
    let (tx, mut rx) = unbounded_channel();
    let ct = CancellationToken::new();
    ct.cancel();
    assert!(matches!(
        within(run(&sp, &tx, &ct)).await,
        Outcome::Cancelled
    ));
    let events = drain(&mut rx);
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], RunEvent::Finished(Outcome::Cancelled)));
    assert!(!pid_file.exists(), "nothing was spawned");
}

// ── run_with_fallback ─────────────────────────────────────

/// The attempt builder's calls: (index, model), in order.
type Calls = Arc<Mutex<Vec<(usize, Option<String>)>>>;

/// A builder that records its calls and gives each attempt its own
/// behavior and its own args file, so the args file proves the process of
/// attempt N was spawned from the spec built for attempt N.
fn builder(
    c: &Ctx,
    tag: &str,
    behavior: fn(usize) -> Vec<(&'static str, String)>,
    calls: Calls,
) -> impl FnMut(usize, Option<&str>) -> RunSpec + use<> {
    let c = c.clone();
    let tag = tag.to_string();
    move |idx, model| {
        let args_file = c.file(&format!("{tag}-args-{idx}"));
        // The previous attempt has exited when the next one is built.
        if idx > 0 {
            assert!(
                c.file(&format!("{tag}-args-{}", idx - 1)).exists(),
                "attempt {idx} built before attempt {} ran",
                idx - 1
            );
        }
        assert!(!args_file.exists(), "attempt {idx} built after its spawn");
        calls.lock().unwrap().push((idx, model.map(str::to_string)));
        let mut env = behavior(idx);
        env.push(("STUB_ARGS_FILE", s(args_file.display())));
        let mut sp = spec(Backend::Codex, &env);
        sp.model = model.map(str::to_string);
        sp
    }
}

fn args_of(c: &Ctx, tag: &str, idx: usize) -> Option<String> {
    std::fs::read_to_string(c.file(&format!("{tag}-args-{idx}"))).ok()
}

async fn fallback_advances_on_retry_worthy_failures(c: Ctx) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let build = builder(
        &c,
        "advance",
        |idx| match idx {
            0 => vec![
                ("STUB_EXIT", s(1)),
                ("STUB_STDERR", s("429 Too Many Requests")),
            ],
            1 => vec![
                ("STUB_EXIT", s(1)),
                ("STUB_STDERR", s("The 'm1' model is not supported")),
            ],
            _ => vec![("STUB_STDOUT", s("answer from m2"))],
        },
        calls.clone(),
    );
    let (tx, mut rx) = unbounded_channel();
    let models = [None, Some("m1".to_string()), Some("m2".to_string())];
    let out = within(run_with_fallback(
        build,
        &models,
        &tx,
        &CancellationToken::new(),
    ))
    .await;
    let r = record(out.outcome);
    assert!(r.success);
    assert_eq!(r.stdout, "answer from m2");
    assert_eq!(out.model.as_deref(), Some("m2"));
    assert_eq!(
        *calls.lock().unwrap(),
        vec![(0, None), (1, Some("m1".into())), (2, Some("m2".into()))]
    );
    assert_eq!(out.discarded.len(), 2);
    assert_eq!(out.discarded[0].model, None);
    assert_eq!(out.discarded[0].model_label(), "(backend default)");
    assert_eq!(out.discarded[0].kind, BackendErrorKind::RateLimited);
    assert!(out.discarded[0].detail.contains("429"));
    assert_eq!(out.discarded[1].model.as_deref(), Some("m1"));
    assert_eq!(out.discarded[1].kind, BackendErrorKind::ModelUnavailable);
    // Each attempt's process ran with the spec built for it.
    assert!(!args_of(&c, "advance", 0).unwrap().contains("-m"));
    assert!(args_of(&c, "advance", 1).unwrap().contains("m1"));
    assert!(args_of(&c, "advance", 2).unwrap().contains("m2"));
    // Every attempt sent its own Started and Finished, in order.
    let kinds: Vec<&str> = drain(&mut rx)
        .iter()
        .map(|e| match e {
            RunEvent::Started { .. } => "started",
            RunEvent::Finished(_) => "finished",
            _ => "other",
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "started", "finished", "started", "finished", "started", "finished"
        ]
    );
}

async fn fallback_stops_on_success_and_on_permanent_failure(c: Ctx) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let build = builder(
        &c,
        "first-ok",
        |_| vec![("STUB_STDOUT", s("fine"))],
        calls.clone(),
    );
    let (tx, _rx) = unbounded_channel();
    let models = [Some("a".to_string()), Some("b".to_string())];
    let out = within(run_with_fallback(
        build,
        &models,
        &tx,
        &CancellationToken::new(),
    ))
    .await;
    assert!(record(out.outcome).success);
    assert_eq!(out.model.as_deref(), Some("a"));
    assert_eq!(calls.lock().unwrap().len(), 1);
    assert!(out.discarded.is_empty());

    // Auth failures and a codex answer that merely mentions a rate limit on
    // stdout are not retry-worthy.
    for (tag, behavior) in [
        (
            "auth",
            (|_| vec![("STUB_EXIT", s(1)), ("STUB_STDERR", s("401 Unauthorized"))])
                as fn(usize) -> Vec<(&'static str, String)>,
        ),
        ("stdout-mention", |_| {
            vec![
                ("STUB_EXIT", s(1)),
                ("STUB_STDOUT", s("how to handle a rate limit")),
                ("STUB_STDERR", s("error: sandbox denied")),
            ]
        }),
    ] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let build = builder(&c, tag, behavior, calls.clone());
        let out = within(run_with_fallback(
            build,
            &models,
            &tx,
            &CancellationToken::new(),
        ))
        .await;
        let r = record(out.outcome);
        assert!(!r.success, "{tag}");
        assert_eq!(out.model.as_deref(), Some("a"), "{tag}");
        assert_eq!(calls.lock().unwrap().len(), 1, "{tag}");
        assert!(out.discarded.is_empty(), "{tag}");
        assert!(args_of(&c, tag, 1).is_none(), "{tag}: no second attempt");
    }
}

async fn fallback_stops_on_the_last_model(c: Ctx) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let build = builder(
        &c,
        "last",
        |_| {
            vec![
                ("STUB_EXIT", s(1)),
                ("STUB_STDERR", s("rate limit reached")),
            ]
        },
        calls.clone(),
    );
    let (tx, _rx) = unbounded_channel();
    let models = [Some("a".to_string()), Some("b".to_string())];
    let out = within(run_with_fallback(
        build,
        &models,
        &tx,
        &CancellationToken::new(),
    ))
    .await;
    let r = record(out.outcome);
    assert!(!r.success);
    assert_eq!(out.model.as_deref(), Some("b"));
    assert_eq!(calls.lock().unwrap().len(), 2);
    assert_eq!(out.discarded.len(), 1);
    assert_eq!(out.discarded[0].model.as_deref(), Some("a"));
}

async fn fallback_stops_on_cancel(c: Ctx) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let pid_file = c.file("chain-cancel-pid");
    let pid_path = s(pid_file.display());
    let mut build = builder(
        &c,
        "cancel",
        |_| vec![("STUB_SLEEP_MS", s(120_000))],
        calls.clone(),
    );
    let (tx, _rx) = unbounded_channel();
    let ct = CancellationToken::new();
    let task = {
        let ct = ct.clone();
        let models = vec![Some("a".to_string()), Some("b".to_string())];
        tokio::spawn(async move {
            let with_pid = move |idx: usize, model: Option<&str>| {
                let mut sp = build(idx, model);
                sp.env.push(("STUB_PID_FILE".into(), pid_path.clone()));
                sp
            };
            run_with_fallback(with_pid, &models, &tx, &ct).await
        })
    };
    let mut reap = Reap::default();
    let pid = reap.add(read_pid(&pid_file).await);
    ct.cancel();
    let out = tokio::time::timeout(WAIT, task)
        .await
        .expect("chain returns promptly after cancel")
        .unwrap();
    assert!(matches!(out.outcome, Outcome::Cancelled));
    assert_eq!(out.model.as_deref(), Some("a"));
    assert_eq!(calls.lock().unwrap().len(), 1, "no attempt after a cancel");
    assert!(out.discarded.is_empty());
    wait_until_gone(pid).await;

    // A chain whose token fired before it began builds nothing.
    let calls = Arc::new(Mutex::new(Vec::new()));
    let build = builder(&c, "precancel", |_| Vec::new(), calls.clone());
    let (tx, _rx) = unbounded_channel();
    let out = within(run_with_fallback(build, &[None], &tx, &ct)).await;
    assert!(matches!(out.outcome, Outcome::Cancelled));
    assert!(calls.lock().unwrap().is_empty());
}

// ── version ───────────────────────────────────────────────

async fn version_probe_carries_the_marker(c: Ctx) {
    let _ = c;
    let reentry = Reentry {
        name: MARKER.into(),
        ceiling: 1,
    };
    let v = within(version(Backend::Codex, &reentry)).await;
    assert_eq!(
        v.as_deref(),
        Some("stub 1.0 1"),
        "the probe carries depth 1"
    );
}

// ── guard ─────────────────────────────────────────────────

async fn guard_modes_without_the_executable(c: Ctx) {
    if cfg!(windows) {
        // Windows guards with a Job Object and looks nothing up.
        return;
    }
    let moved = Aside::new(c.bin("agent-guard"));
    assert_eq!(
        guard::locate(),
        None,
        "no agent-guard may be found beside the test executable or on PATH"
    );

    // Required: an error before anything is spawned.
    let pid_file = c.file("required-pid");
    let sp = spec(Backend::Codex, &[("STUB_PID_FILE", s(pid_file.display()))]);
    let (o, events) = run_one(&sp).await;
    match &o {
        Outcome::Spawn(m) => {
            assert!(m.contains("agent-guard"), "{m}");
            assert!(m.contains("cargo build"), "{m}");
        }
        other => panic!("expected Spawn, got {other:?}"),
    }
    assert_eq!(events.len(), 1, "{events:?}");
    assert!(!pid_file.exists(), "nothing was spawned");

    // Preferred: runs unguarded and says so.
    let mut sp = spec(Backend::Codex, &[("STUB_ECHO_STDIN", s(1))]);
    sp.guard = GuardMode::Preferred;
    let (o, events) = run_one(&sp).await;
    assert_eq!(record(o).stdout, "hello prompt");
    assert_eq!(started_guarded(&events), Some(false));

    // Off: runs unguarded.
    sp.guard = GuardMode::Off;
    let (o, events) = run_one(&sp).await;
    assert!(record(o).success);
    assert_eq!(started_guarded(&events), Some(false));
    drop(moved);
}

async fn guard_mode_off_never_looks_up_the_guard(c: Ctx) {
    if cfg!(windows) {
        return;
    }
    // A file named agent-guard that cannot run: whoever looks it up and uses
    // it fails to spawn.
    let moved = Aside::new(c.bin("agent-guard"));
    std::fs::write(c.bin("agent-guard"), "not an executable").unwrap();
    assert_eq!(guard::locate(), Some(c.bin("agent-guard")));

    let mut sp = spec(Backend::Codex, &[("STUB_ECHO_STDIN", s(1))]);
    sp.guard = GuardMode::Off;
    let (o, events) = run_one(&sp).await;
    assert_eq!(record(o).stdout, "hello prompt");
    assert_eq!(started_guarded(&events), Some(false));

    sp.guard = GuardMode::Preferred;
    let (o, _) = run_one(&sp).await;
    assert!(
        matches!(&o, Outcome::Spawn(m) if m.contains("agent-guard")),
        "Preferred looked it up and used it: {o:?}"
    );
    drop(moved);
}

async fn guarded_run_leaves_no_descendant(c: Ctx) {
    let _ = c;
    let marker = tag("guarded-run");
    let mut sp = spec(Backend::Codex, &[("STUB_ECHO_STDIN", s(1))]);
    sp.model = Some(marker.clone());
    for _ in 0..3 {
        let (o, events) = run_one(&sp).await;
        assert!(record(o).success);
        assert_eq!(started_guarded(&events), Some(true));
    }
    #[cfg(unix)]
    wait_until_unmarked(&marker).await;
}

async fn guard_mirrors_exit_code_signal_and_clears_its_marker(c: Ctx) {
    if cfg!(windows) {
        return;
    }
    let guard_cmd = |env: &[(&str, &str)], args: &[&str]| {
        let mut cmd = std::process::Command::new(c.bin("agent-guard"));
        cmd.arg(std::process::id().to_string())
            .arg("--")
            .arg(c.bin("codex"))
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        Spawned::new(cmd.spawn().expect("spawn agent-guard"), true)
    };

    let mut g = guard_cmd(&[("STUB_EXIT", "7")], &[]);
    let st = g.wait().await;
    assert_eq!(st.code(), Some(7));

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        let mut g = guard_cmd(&[("STUB_SIGNAL", "15")], &[]);
        let st = g.wait().await;
        assert_eq!(st.signal(), Some(15), "{st:?}");
    }

    // The guard marker is set on the guard and removed from its program: the
    // stub prints the value of the variable STUB_MARKER_NAME names.
    let mut g = guard_cmd(
        &[
            (guard::GUARD_ENV, "1"),
            ("STUB_MARKER_NAME", guard::GUARD_ENV),
        ],
        &["--version"],
    );
    let st = g.wait().await;
    assert!(st.success());
    let mut out = String::new();
    use std::io::Read;
    g.child
        .stdout
        .take()
        .expect("stdout")
        .read_to_string(&mut out)
        .unwrap();
    assert_eq!(
        out.trim_end(),
        "stub 1.0",
        "the program must not see the marker"
    );
}

async fn guard_kills_program_and_its_child_when_parent_dies(c: Ctx) {
    if cfg!(windows) {
        return;
    }
    let marker = tag("parent-dies");
    let guard_file = c.file("pd-guard-pid");
    let pid_file = c.file("pd-backend-pid");
    let child_file = c.file("pd-child-pid");
    let mut parent = Spawned::new(
        std::process::Command::new(STUB_PARENT)
            .arg("guard")
            .arg(c.bin("agent-guard"))
            .arg(c.bin("codex"))
            .arg(&marker)
            .env("STUB_GUARD_PID_FILE", &guard_file)
            .env("STUB_PID_FILE", &pid_file)
            .env("STUB_CHILD_TAG", &marker)
            .env("STUB_CHILD_PID_FILE", &child_file)
            .env("STUB_SLEEP_MS", "120000")
            .stdin(std::process::Stdio::null())
            .spawn()
            .expect("spawn stub-parent"),
        false,
    );
    let mut reap = Reap::default();
    let guard_pid = reap.add(read_pid(&guard_file).await);
    let backend_pid = reap.add(read_pid(&pid_file).await);
    let child_pid = reap.add(read_pid(&child_file).await);
    assert!(process_alive(backend_pid) && process_alive(child_pid));
    #[cfg(unix)]
    assert_eq!(
        count_marked(&marker),
        4,
        "stub-parent, the guard, the program and its child"
    );

    parent.kill();
    let _ = parent.wait().await;
    wait_until_gone(guard_pid).await;
    wait_until_gone(backend_pid).await;
    wait_until_gone(child_pid).await;
    #[cfg(unix)]
    wait_until_unmarked(&marker).await;
}

async fn guard_marked_process_refuses_to_start_a_backend(c: Ctx) {
    let pid_file = c.file("tripwire-pid");
    let mut parent = Spawned::new(
        std::process::Command::new(STUB_PARENT)
            .arg("run")
            .env(guard::GUARD_ENV, "1")
            .env("STUB_GUARD_MODE", "off")
            .env("STUB_PID_FILE", &pid_file)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("spawn stub-parent"),
        false,
    );
    let st = parent.wait().await;
    assert!(st.success());
    let mut out = String::new();
    use std::io::Read;
    parent
        .child
        .stdout
        .take()
        .expect("stdout")
        .read_to_string(&mut out)
        .unwrap();
    assert!(
        out.starts_with("outcome=spawn guarded=none") && out.contains(guard::GUARD_ENV),
        "{out}"
    );
    assert!(!pid_file.exists(), "nothing was spawned");
}

async fn killed_server_takes_its_backend_along(c: Ctx) {
    // stub-parent runs one Required attempt whose backend sleeps with a child
    // of its own. Kill stub-parent outright — no cleanup code of its own can
    // run — and the backend and its child must die with it: behind
    // agent-guard on Linux and macOS, in the kill-on-close Job Object on
    // Windows, which the kernel closes with stub-parent's handles.
    let marker = tag("server-killed");
    let pid_file = c.file("sk-backend-pid");
    let child_file = c.file("sk-child-pid");
    let mut parent = Spawned::new(
        std::process::Command::new(STUB_PARENT)
            .arg("run")
            .env("STUB_GUARD_MODE", "required")
            .env("STUB_MODEL", &marker)
            .env("STUB_PID_FILE", &pid_file)
            .env("STUB_CHILD_TAG", &marker)
            .env("STUB_CHILD_PID_FILE", &child_file)
            .env("STUB_SLEEP_MS", "120000")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("spawn stub-parent"),
        false,
    );
    let mut reap = Reap::default();
    let backend_pid = reap.add(read_pid(&pid_file).await);
    let child_pid = reap.add(read_pid(&child_file).await);
    assert!(process_alive(backend_pid) && process_alive(child_pid));
    parent.kill();
    let _ = parent.wait().await;
    wait_until_gone(backend_pid).await;
    wait_until_gone(child_pid).await;
    #[cfg(unix)]
    wait_until_unmarked(&marker).await;
}
