//! Test support: a stand-in for the codex and claude CLIs in this crate's
//! integration tests. It is never installed.
//!
//! The tests copy this binary onto a temporary `PATH` as `codex` and `claude`
//! and steer it through environment variables, so a "backend" can echo its
//! stdin, print a given amount of output, exit with a given code or signal,
//! sleep, or start a child of its own. It never links `agent_exec::run` and
//! never starts a guard or a copy of itself that could do anything but sleep.
//!
//! Modes:
//! - `--version`: print `stub 1.0 <value>`, the value being that of the
//!   variable named by `STUB_MARKER_NAME` (empty when unset).
//! - `__sleep <ms> [<tag>...]`: sleep, then exit 0. Selected by argv, not by
//!   the environment, so the child a backend starts in this mode can never
//!   start another; the tags only make the process findable.
//! - anything else: the backend below.
//!
//! Backend variables: `STUB_ARGS_FILE` (write the argv after the binary, one
//! per line), `STUB_PID_FILE` (write the process id), `STUB_SKIP_STDIN=1`
//! (never read stdin), `STUB_INTERLEAVE=1` (read stdin in 1 KiB reads with a
//! pause, writing 4 KiB of `STUB_FILL` to stdout after each), `STUB_STDIN_FILE`
//! (write the received stdin), `STUB_ECHO_STDIN=1` (copy stdin to stdout),
//! `STUB_STDOUT` / `STUB_STDERR` (literal text), `STUB_STDOUT_REPEAT` /
//! `STUB_STDERR_REPEAT` (print `STUB_FILL`, default `a`, that many times),
//! `STUB_CHILD_TAG` (start `__sleep 120000 <tag>` as a child and write its pid
//! to `STUB_CHILD_PID_FILE`), `STUB_SLEEP_MS`, `STUB_SIGNAL` (Unix: end by
//! raising that signal) and `STUB_EXIT` (default 0).

use std::io::{Read, Write};
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") => {
            let marker = var("STUB_MARKER_NAME")
                .and_then(|n| std::env::var(n).ok())
                .unwrap_or_default();
            println!("stub 1.0 {marker}");
        }
        Some("__sleep") => {
            let ms = args.get(1).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
            std::thread::sleep(Duration::from_millis(ms));
        }
        _ => backend(&args),
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn repeat(name: &str) -> usize {
    var(name).and_then(|n| n.parse().ok()).unwrap_or(0)
}

fn backend(args: &[String]) {
    if let Some(p) = var("STUB_ARGS_FILE") {
        let _ = std::fs::write(p, args.join("\n"));
    }
    if let Some(p) = var("STUB_PID_FILE") {
        let _ = std::fs::write(p, std::process::id().to_string());
    }
    if let Some(tag) = var("STUB_CHILD_TAG") {
        start_sleeping_child(&tag);
    }
    let fill = var("STUB_FILL").unwrap_or_else(|| "a".into());

    let mut input = Vec::new();
    if var("STUB_SKIP_STDIN").is_none() {
        if var("STUB_INTERLEAVE").is_some() {
            interleave(&mut input, &fill);
        } else {
            let _ = std::io::stdin().read_to_end(&mut input);
        }
    }
    if let Some(p) = var("STUB_STDIN_FILE") {
        let _ = std::fs::write(p, &input);
    }
    {
        let mut out = std::io::stdout().lock();
        if var("STUB_ECHO_STDIN").as_deref() == Some("1") {
            let _ = out.write_all(&input);
        }
        if let Some(t) = var("STUB_STDOUT") {
            let _ = out.write_all(t.as_bytes());
        }
        let _ = out.write_all(fill.repeat(repeat("STUB_STDOUT_REPEAT")).as_bytes());
        let _ = out.flush();
    }
    {
        let mut err = std::io::stderr().lock();
        if let Some(t) = var("STUB_STDERR") {
            let _ = err.write_all(t.as_bytes());
        }
        let _ = err.write_all(fill.repeat(repeat("STUB_STDERR_REPEAT")).as_bytes());
        let _ = err.flush();
    }
    if let Some(ms) = var("STUB_SLEEP_MS").and_then(|v| v.parse::<u64>().ok()) {
        std::thread::sleep(Duration::from_millis(ms));
    }
    #[cfg(unix)]
    if let Some(sig) = var("STUB_SIGNAL").and_then(|v| v.parse::<i32>().ok()) {
        unsafe {
            libc::signal(sig, libc::SIG_DFL);
            libc::raise(sig);
        }
    }
    let code = var("STUB_EXIT")
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(0);
    std::process::exit(code);
}

/// Read stdin slowly while writing more than a pipe buffer of stdout, so the
/// caller must service both at once.
fn interleave(input: &mut Vec<u8>, fill: &str) {
    let mut stdin = std::io::stdin().lock();
    let mut out = std::io::stdout().lock();
    let block = fill.repeat(4096 / fill.len().max(1));
    let mut chunk = [0u8; 1024];
    loop {
        match stdin.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => input.extend_from_slice(&chunk[..n]),
        }
        if out.write_all(block.as_bytes()).is_err() {
            break;
        }
        std::thread::sleep(Duration::from_micros(200));
    }
    let _ = out.flush();
}

/// Start `<this binary> __sleep 120000 <tag>` in this process's group and
/// write its pid to `STUB_CHILD_PID_FILE`. The child gets no stdio of ours and
/// none of the variables that steer this binary.
fn start_sleeping_child(tag: &str) {
    let Ok(me) = std::env::current_exe() else {
        return;
    };
    let mut cmd = std::process::Command::new(me);
    cmd.args(["__sleep", "120000", tag])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("STUB_") {
            cmd.env_remove(k);
        }
    }
    if let Ok(child) = cmd.spawn()
        && let Some(p) = var("STUB_CHILD_PID_FILE")
    {
        let _ = std::fs::write(p, child.id().to_string());
    }
}
