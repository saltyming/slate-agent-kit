//! `agent-guard`: the parent-death guard executable. It sits between a server
//! and a backend process so the backend's whole process tree dies with the
//! server, even under a hard `SIGKILL` of the server itself.
//!
//! Usage: `agent-guard <parent pid> -- <program> <args...>`. The guard leads a
//! new process group, starts the program in it with the guard's own stdio,
//! then blocks until either the program exits (the guard exits with the same
//! status: the code, or the same signal) or the parent dies (the guard kills
//! the whole group). `agent_exec::run` starts guarded backends through this
//! executable (`agent_exec::guard::wrap`); nothing else in the workspace does.
//!
//! Boundary: the guard is synchronous and single-threaded, and it never calls
//! `agent_exec::run` or `agent_exec::guard::wrap`, so it can never start a
//! guard or a backend of its own accord. The crate marks the guard process
//! with `AGENT_EXEC_GUARD`; the guard removes the variable from the program's
//! environment, so the program is an ordinary process again.
//!
//! Why a guard, not a bare `PR_SET_PDEATHSIG` on the backend directly: the
//! primitive only arms the ONE process that calls it — it does not cover a
//! descendant the backend spawns mid-run (e.g. a shell or test runner). Turning
//! "parent died" into "kill the whole group" needs code we control reacting to
//! that signal, and that code can't live inside the backend's own binary since
//! we don't control its source. So this guard sits in between:
//! - it is the pgid leader in place of the backend
//! - the real backend inherits that pgid without calling `process_group(0)`
//!   itself
//! - on parent death, the guard kills the whole group
//! - on backend exit, the guard reaps it and mirrors the exit status onto
//!   itself, so the server sees this as "the child's" exit status either way
//!
//! The parent watch is per platform: `linux.rs` (`PR_SET_PDEATHSIG` through a
//! `signalfd`) and `macos.rs` (`kqueue` `EVFILT_PROC`). On Windows the guard is
//! a Job Object attached inside the server process and this executable is
//! never used; there it only reports that and exits non-zero.

use std::ffi::{OsStr, OsString};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

fn main() {
    let args = std::env::args_os().skip(1);
    let code = match parse_args(args) {
        Ok((parent_pid, argv)) => match platform::run(parent_pid, &argv) {
            Ok(never) => match never {},
            Err(e) => {
                eprintln!("agent-guard: {e}");
                1
            }
        },
        Err(e) => {
            eprintln!("agent-guard: {e}");
            eprintln!("usage: agent-guard <parent pid> -- <program> <args...>");
            2
        }
    };
    std::process::exit(code);
}

/// Split `<parent pid> -- <program> <args...>` into the pid and the program's
/// argv (program first).
fn parse_args(
    args: impl IntoIterator<Item = OsString>,
) -> Result<(u32, Vec<OsString>), &'static str> {
    let mut args = args.into_iter();
    let pid_arg = args.next().ok_or("missing <parent pid> argument")?;
    let parent_pid: u32 = pid_arg
        .to_str()
        .and_then(|s| s.parse().ok())
        .filter(|pid| *pid > 0)
        .ok_or("<parent pid> must be a positive integer")?;
    if args.next().as_deref() != Some(OsStr::new("--")) {
        return Err("expected `--` before the program");
    }
    let argv: Vec<OsString> = args.collect();
    if argv.is_empty() {
        return Err("missing program after `--`");
    }
    Ok((parent_pid, argv))
}

/// An uninhabited type: `run` returns only on error, since on success the
/// guard exits with the program's status.
enum Never {}

/// How the wait ended.
#[cfg(any(target_os = "linux", target_os = "macos"))]
enum Outcome {
    ChildExited(std::process::ExitStatus),
    ParentDied,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod platform {
    use std::ffi::OsString;
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Command, ExitStatus, Stdio};

    #[cfg(target_os = "linux")]
    use super::linux::{arm_parent_watch, wait_for_either};
    #[cfg(target_os = "macos")]
    use super::macos::{arm_parent_watch, wait_for_either};
    use super::{Never, Outcome};

    pub(super) fn run(parent_pid: u32, argv: &[OsString]) -> Result<Never, String> {
        // 0. Lead a process group of our own, so the group kill below reaches
        //    the program and everything it spawns and nothing else. The crate
        //    already spawns the guard with `process_group(0)`; this makes the
        //    guard correct when started any other way.
        lead_process_group()?;

        // 1. Arm the parent-death primitive FIRST, before spawning the
        //    (possibly write-capable) program. Arming, then rechecking
        //    liveness, closes the race where the parent dies in the gap
        //    between a liveness check and the arm call — a check-then-arm
        //    ordering would leave that gap open instead.
        let watcher = arm_parent_watch(parent_pid).map_err(|e| e.to_string())?;

        // 2. Re-check parent liveness immediately after arming. This only
        //    catches the (now much narrower) case of the parent dying before
        //    the arm call above — everything after this point is covered by
        //    the armed watch itself.
        if !parent_is(parent_pid) {
            return Err("parent already gone before spawn — aborting".into());
        }

        // 3. Spawn the program. No process_group(0) here: the guard leads the
        //    group, so the program inherits that pgid by default. No stdio
        //    redirection: fd 0/1/2 pass straight through from the guard, which
        //    are already the exact pipe ends the server created for the
        //    backend. The guard marker is removed: the program is not a guard.
        let mut child = Command::new(&argv[0])
            .args(&argv[1..])
            .env_remove(agent_exec::guard::GUARD_ENV)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("spawn {:?} failed: {e}", argv[0]))?;
        let child_pid = child.id();

        // 4. Block until either the child exits or the parent dies, whichever
        //    first.
        let outcome = match wait_for_either(parent_pid, child_pid, &mut child, watcher) {
            Ok(o) => o,
            Err(e) => {
                // The watch broke: the program must not outlive a guard that
                // can no longer watch for it.
                eprintln!("agent-guard: {e}");
                kill_group();
            }
        };

        match outcome {
            Outcome::ChildExited(status) => {
                // Re-check parent liveness once more immediately before
                // mirroring — closes the narrow race where the child-exit event
                // "won" the wait while the parent was also dying. If the parent
                // is ALSO gone now, prioritize the group-kill cleanup path over
                // naively mirroring the child's exit, so a descendant the child
                // spawned isn't missed.
                if !parent_is(parent_pid) {
                    kill_group();
                }
                mirror_exit(status);
            }
            Outcome::ParentDied => kill_group(),
        }
    }

    /// Make this process the leader of its own process group, unless it
    /// already is one.
    fn lead_process_group() -> Result<(), String> {
        unsafe {
            if libc::getpgrp() == libc::getpid() {
                return Ok(());
            }
            if libc::setpgid(0, 0) != 0 {
                return Err(format!(
                    "setpgid failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
        }
        Ok(())
    }

    /// Kills the whole process group (guard + program + any real descendants
    /// sharing the pgid) and never returns.
    fn kill_group() -> ! {
        unsafe {
            libc::kill(-(std::process::id() as i32), libc::SIGKILL);
        }
        // SIGKILL to our own group terminates this process too; reaching here
        // is only possible if that somehow failed, so exit explicitly rather
        // than fall through.
        std::process::exit(137);
    }

    pub(super) fn parent_is(parent_pid: u32) -> bool {
        unsafe { libc::getppid() == parent_pid as libc::pid_t }
    }

    /// Mirrors the child's exit status onto the guard's own exit, so callers
    /// see this as "the child's" exit status regardless of the guard
    /// indirection.
    fn mirror_exit(status: ExitStatus) -> ! {
        if let Some(code) = status.code() {
            std::process::exit(code);
        }
        if let Some(sig) = status.signal() {
            unsafe {
                libc::signal(sig, libc::SIG_DFL);
                libc::raise(sig);
            }
            // raise() only returns if the signal didn't terminate us — fall
            // back to a faithful shell-convention exit code.
            std::process::exit(128 + sig);
        }
        std::process::exit(1);
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod platform {
    use std::ffi::OsString;

    use super::Never;

    pub(super) fn run(_parent_pid: u32, _argv: &[OsString]) -> Result<Never, String> {
        Err(
            "not supported on this platform; on Windows the backend is placed in a \
             kill-on-close Job Object by the server itself"
                .into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn osv(items: &[&str]) -> Vec<OsString> {
        items.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_valid_argv() {
        let (pid, argv) = parse_args(osv(&["1234", "--", "codex", "exec"])).unwrap();
        assert_eq!(pid, 1234);
        assert_eq!(argv, osv(&["codex", "exec"]));
    }

    #[test]
    fn rejects_malformed_argv() {
        assert!(parse_args(osv(&[])).is_err(), "missing pid");
        assert!(
            parse_args(osv(&["x", "--", "codex"])).is_err(),
            "non-numeric"
        );
        assert!(parse_args(osv(&["0", "--", "codex"])).is_err(), "zero pid");
        assert!(parse_args(osv(&["-5", "--", "codex"])).is_err(), "negative");
        assert!(parse_args(osv(&["1234", "codex"])).is_err(), "no separator");
        assert!(parse_args(osv(&["1234", "--"])).is_err(), "no program");
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn parent_is_matches_real_parent_pid() {
        let real_parent = unsafe { libc::getppid() } as u32;
        assert!(platform::parent_is(real_parent));
        assert!(!platform::parent_is(real_parent.wrapping_add(1)));
    }
}
