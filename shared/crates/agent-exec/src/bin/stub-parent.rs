//! Test support: a killable parent for this crate's guard tests, and a
//! process that runs one attempt in an environment the test controls. It is
//! never installed.
//!
//! Modes (the first argument):
//! - `guard <agent-guard> <program> <args...>`: start `<agent-guard> <own pid>
//!   -- <program> <args...>` directly, write the guard's pid to
//!   `STUB_GUARD_PID_FILE`, then sleep until killed. Killing this process is
//!   the "parent died" case of the guard.
//! - `run`: run one codex attempt through `agent_exec::run` with the guard mode
//!   in `STUB_GUARD_MODE` (`required`, `preferred`, `off`) and the model in
//!   `STUB_MODEL`, and print one line: `outcome=<kind> guarded=<bool|none>`
//!   followed by the outcome's message, if any.
//!
//! A copy of this process never starts another: it refuses to run when
//! `STUB_PARENT_DEPTH` is set in its environment and sets it for everything
//! it starts, and the attempt's backend is `stub-backend`, which starts no
//! parent.

use std::time::Duration;

use agent_exec::{
    Backend, CapturePolicy, FailureTextPolicy, GuardMode, Isolation, Outcome, OutputMode, Reentry,
    RunEvent, RunSpec, Sandbox,
};

const DEPTH: &str = "STUB_PARENT_DEPTH";

fn main() {
    if std::env::var_os(DEPTH).is_some() {
        eprintln!("stub-parent: refusing to run inside another stub-parent");
        std::process::exit(3);
    }
    // SAFETY: no other thread exists yet.
    unsafe { std::env::set_var(DEPTH, "1") };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("guard") if args.len() >= 3 => guard(&args[1], &args[2..]),
        Some("run") => run(),
        _ => {
            eprintln!("usage: stub-parent guard <agent-guard> <program> <args...> | run");
            2
        }
    };
    std::process::exit(code);
}

fn guard(guard_path: &str, argv: &[String]) -> i32 {
    let child = std::process::Command::new(guard_path)
        .arg(std::process::id().to_string())
        .arg("--")
        .args(argv)
        .stdin(std::process::Stdio::null())
        .spawn();
    let child = match child {
        Ok(c) => c,
        Err(e) => {
            eprintln!("stub-parent: spawn {guard_path} failed: {e}");
            return 1;
        }
    };
    if let Ok(p) = std::env::var("STUB_GUARD_PID_FILE") {
        let _ = std::fs::write(p, child.id().to_string());
    }
    // Stay alive until the test kills this process; a bounded sleep so a
    // test that fails to kill it does not leave it behind for long.
    std::thread::sleep(Duration::from_secs(120));
    0
}

fn run() -> i32 {
    let guard = match std::env::var("STUB_GUARD_MODE").as_deref() {
        Ok("required") => GuardMode::Required,
        Ok("preferred") => GuardMode::Preferred,
        _ => GuardMode::Off,
    };
    let spec = RunSpec {
        backend: Backend::Codex,
        prompt: "from stub-parent".into(),
        working_dir: None,
        sandbox: Sandbox::ReadOnly,
        isolation: Isolation::IgnoreUserConfig,
        output_mode: OutputMode::Default,
        capture: CapturePolicy::dispatch(),
        failure_text: FailureTextPolicy::dispatch(),
        model: std::env::var("STUB_MODEL").ok(),
        reasoning_effort: None,
        resume_session: None,
        pin_session: None,
        skip_git_repo_check: false,
        env: Vec::new(),
        reentry: Reentry {
            name: "STUB_PARENT_REENTRY_DEPTH".into(),
            ceiling: 1,
        },
        guard,
        backend_version: None,
    };
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return 2;
    };
    let (outcome, guarded) = rt.block_on(async {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let ct = tokio_util::sync::CancellationToken::new();
        let outcome = agent_exec::run(&spec, &tx, &ct).await;
        let mut guarded = None;
        while let Ok(ev) = rx.try_recv() {
            if let RunEvent::Started { guarded: g, .. } = ev {
                guarded = Some(g);
            }
        }
        (outcome, guarded)
    });
    let guarded = guarded.map_or("none".to_string(), |g| g.to_string());
    let (kind, message) = match &outcome {
        Outcome::Record(r) => ("record", format!("exit_code={:?}", r.exit_code)),
        Outcome::NotFound { hint, .. } => ("not_found", hint.clone()),
        Outcome::Spawn(m) => ("spawn", m.clone()),
        Outcome::WaitFailed(m) => ("wait_failed", m.clone()),
        Outcome::Cancelled => ("cancelled", String::new()),
    };
    println!("outcome={kind} guarded={guarded} {message}");
    0
}
