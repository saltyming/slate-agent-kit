//! Keeping a backend's process tree from outliving the server that spawned it.
//!
//! On Linux and macOS a guarded backend is started through `agent-guard`, a
//! separate executable built from this crate (`src/bin/agent-guard/`):
//! `agent-guard <server pid> -- <program> <args...>` leads the process group,
//! kills the group when the server dies and otherwise mirrors the backend's
//! exit status. On Windows the backend is placed in a kill-on-close Job Object
//! inside the server process (see `winjob.rs`), and no executable is used.
//!
//! Entry points: [`spawn_contained`] starts a command under a [`GuardMode`]
//! and is the one spawn path for every guarded process (`run` uses it for
//! backends, dispatch for its `opencode serve`); [`ProcessTree`] kills the
//! whole tree on cancel. Beneath it, [`locate`] finds `agent-guard`, [`wrap`]
//! turns a command into its guarded form and [`protect`] attaches the Job
//! Object on Windows.
//!
//! Boundary: this module never re-invokes the running executable and never
//! runs the guard itself, so no program that links the crate needs guard code
//! in its `main`. The guard process carries [`GUARD_ENV`]; `agent-guard`
//! removes it from the backend's environment, and `run` refuses to start a
//! backend in a process that carries it.

use std::io;
use std::path::{Path, PathBuf};

use tokio::process::{Child, Command};

use crate::spec::GuardMode;

#[cfg(windows)]
mod winjob;

/// The environment variable the crate sets on every guard process. A process
/// that finds it in its own environment was started as a guard and is not one
/// (the real guard removes it from the program it starts), so `run` refuses
/// to start a backend there.
pub const GUARD_ENV: &str = "AGENT_EXEC_GUARD";

/// The guard executable's name, without the platform's executable suffix.
pub const GUARD_BINARY: &str = "agent-guard";

/// The path of the `agent-guard` executable: beside the running executable
/// when a file of that name is there, else the first one on `PATH`; `None`
/// when neither exists. Like [`crate::which`], it tests for a file, not for
/// executability.
pub fn locate() -> Option<PathBuf> {
    let name = format!("{GUARD_BINARY}{}", std::env::consts::EXE_SUFFIX);
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let beside = dir.join(&name);
        if beside.is_file() {
            return Some(beside);
        }
    }
    crate::which(GUARD_BINARY)
}

/// One line telling how to make `agent-guard` available, for the `Spawn`
/// outcome of a `GuardMode::Required` run that did not find it. It names no
/// path or number, so the classifier reads it as `Other` and a fallback chain
/// does not retry every model against the same missing executable.
pub fn install_hint() -> String {
    format!(
        "`{GUARD_BINARY}` was not found beside the server executable or on PATH; install a kit \
         release that ships `{GUARD_BINARY}` beside the servers, or build it with `cargo build \
         --release -p agent-exec --bin {GUARD_BINARY}` and place it beside the server"
    )
}

/// The `Spawn` text for a `GuardMode::Required` start of `label` that found no
/// guard (Linux and macOS; Windows looks no executable up).
#[cfg(any(not(windows), test))]
pub(crate) fn missing_message(label: &str) -> String {
    format!(
        "cannot start {label} under the parent-death guard: {}",
        install_hint()
    )
}

/// A process started by [`spawn_contained`].
pub struct Contained {
    /// The spawned process: on Linux and macOS with the guard, `agent-guard`,
    /// which leads the process group and exits when the program it started
    /// exits.
    pub child: Child,
    /// Kills the whole tree: the process group on Unix, the Job Object on
    /// Windows.
    pub tree: ProcessTree,
    /// Whether the parent-death guard holds the process (`agent-guard` on
    /// Linux and macOS, a kill-on-close Job Object on Windows).
    pub guarded: bool,
}

/// Start `command` under `mode`, so that it can never run outside the
/// containment the mode asks for.
///
/// On Linux and macOS the command is wrapped in `agent-guard` (looked up with
/// [`locate`]; `Required` without it fails before anything starts, `Preferred`
/// runs unguarded, `Off` never looks) and leads a new process group. On
/// Windows the process is created suspended, placed in a Job Object
/// (kill-on-close unless `mode` is `Off`, where the job only lets a cancel end
/// the tree), and only then resumed, so nothing it starts can escape the job;
/// a `Required` start whose job cannot be attached is killed and fails.
///
/// `configure` sets the stdio (and anything else) on the command that is
/// actually spawned, after wrapping; the child is always `kill_on_drop`.
/// `label` names the process in the error texts: `cannot start <label> under
/// the parent-death guard: …`, `spawn agent-guard for <label> failed: …` (or
/// `spawn <label> failed: …` unguarded), `Job Object setup for <label> failed:
/// …`, `resume <label> after its spawn failed: …`.
pub async fn spawn_contained(
    command: Command,
    mode: GuardMode,
    label: &str,
    configure: impl FnOnce(&mut Command),
) -> Result<Contained, String> {
    #[cfg(not(windows))]
    let guard_path = match mode {
        GuardMode::Off => None,
        GuardMode::Preferred => locate(),
        GuardMode::Required => Some(locate().ok_or_else(|| missing_message(label))?),
    };
    #[cfg(not(windows))]
    let mut cmd = match &guard_path {
        Some(p) => wrap(&command, p),
        None => command,
    };
    #[cfg(windows)]
    let mut cmd = command;
    configure(&mut cmd);
    cmd.kill_on_drop(true);
    // Unix: a new process group with pgid == child pid, so a group kill reaps
    // the guard, the program AND anything it spawned; kill_on_drop only reaps
    // the direct child.
    #[cfg(unix)]
    cmd.process_group(0);
    // Windows: start suspended, so the Job Object holds the process before it
    // can start anything; resumed right after the job is attached.
    #[cfg(windows)]
    cmd.creation_flags(CREATE_SUSPENDED);

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            #[cfg(not(windows))]
            let what = if guard_path.is_some() {
                format!("{GUARD_BINARY} for {label}")
            } else {
                label.to_string()
            };
            #[cfg(windows)]
            let what = label.to_string();
            return Err(format!("spawn {what} failed: {e}"));
        }
    };
    #[cfg(not(windows))]
    let contained = Contained {
        tree: ProcessTree::unprotected(&child),
        child,
        guarded: guard_path.is_some(),
    };
    #[cfg(windows)]
    let contained = attach(child, mode, label).await?;
    Ok(contained)
}

/// Windows: place a just-spawned, suspended child in its Job Object and
/// resume it.
#[cfg(windows)]
async fn attach(mut child: Child, mode: GuardMode, label: &str) -> Result<Contained, String> {
    let attached = if mode == GuardMode::Off {
        // A plain job, only so that a cancel ends the whole tree; not a guard.
        // Without it the process runs on and a cancel ends it alone.
        contain(&mut child).map(|t| (t, false))
    } else {
        protect(&mut child).map(|t| (t, true))
    };
    let (tree, guarded) = match attached {
        Ok(pair) => pair,
        Err(e) if mode == GuardMode::Required => {
            // Continuing would leave the work running with no orphan
            // protection at all.
            ProcessTree::unprotected(&child).kill(&mut child);
            let _ = child.wait().await;
            return Err(format!("Job Object setup for {label} failed: {e}"));
        }
        Err(e) => {
            tracing::warn!(
                "agent-exec: Job Object setup for {label} failed: {e}; running without it"
            );
            (ProcessTree::unprotected(&child), false)
        }
    };
    if let Err(e) = resume(&child) {
        tree.kill(&mut child);
        let _ = child.wait().await;
        return Err(format!("resume {label} after its spawn failed: {e}"));
    }
    Ok(Contained {
        child,
        tree,
        guarded,
    })
}

/// The guarded form of `command`: `<guard_path> <this pid> -- <program>
/// <args...>`, with `command`'s environment changes and directory carried
/// over, [`GUARD_ENV`] set, and on Unix a new process group, so the guard
/// leads the group the tree is killed by. Stdio and `kill_on_drop` are left
/// to the caller. The program is resolved by the guard on its `PATH`, as the
/// unguarded command would be.
pub fn wrap(command: &Command, guard_path: &Path) -> Command {
    let std = command.as_std();
    let mut out = Command::new(guard_path);
    out.arg(std::process::id().to_string())
        .arg("--")
        .arg(std.get_program())
        .args(std.get_args());
    for (k, v) in std.get_envs() {
        match v {
            Some(v) => {
                out.env(k, v);
            }
            None => {
                out.env_remove(k);
            }
        }
    }
    out.env(GUARD_ENV, "1");
    if let Some(dir) = std.get_current_dir() {
        out.current_dir(dir);
    }
    #[cfg(unix)]
    out.process_group(0);
    out
}

/// A spawned child's process tree, for killing it as a whole.
///
/// On Unix the tree is the child's process group (`run` spawns every child,
/// guarded or not, with `process_group(0)`). On Windows it is the child's Job
/// Object when one was attached (kill-on-close by [`protect`] for a guarded
/// run, plain for `GuardMode::Off`); dropping the tree closes the job, which
/// kills whatever is still in a kill-on-close job.
pub struct ProcessTree {
    #[cfg(unix)]
    pid: Option<u32>,
    #[cfg(windows)]
    job: Option<winjob::WinJob>,
}

impl ProcessTree {
    /// A tree with no Job Object: killing it kills the process group on Unix,
    /// the direct child only on Windows.
    pub fn unprotected(child: &Child) -> Self {
        #[cfg(not(unix))]
        let _ = child;
        ProcessTree {
            #[cfg(unix)]
            pid: child.id(),
            #[cfg(windows)]
            job: None,
        }
    }

    /// Kill the whole tree (SIGKILL to the process group on Unix; terminate
    /// the Job Object on Windows, or the direct child when there is none). Call
    /// it before reaping the child, so the group id cannot have been reused.
    pub fn kill(&self, child: &mut Child) {
        #[cfg(unix)]
        if let Some(pid) = self.pid {
            // The child leads its own group (pgid == pid); a negative pid
            // signals the whole group.
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
        #[cfg(windows)]
        if let Some(job) = &self.job {
            job.terminate(1);
        }
        let _ = child.start_kill();
    }

    /// Release the tree after the child exited on its own: on Windows this
    /// closes the Job Object, which also ends any descendant still in it; on
    /// Unix there is nothing to release.
    pub fn release(self) {
        #[cfg(windows)]
        drop(self.job);
    }
}

/// Protect a just-spawned child. On Windows this attaches a kill-on-close Job
/// Object; if that fails the error is returned and the caller decides (`run`
/// kills the child for `GuardMode::Required` and runs on unguarded for
/// `GuardMode::Preferred`). `run` creates the child suspended and resumes it
/// only after this returns, so nothing the child starts can escape the job. On
/// Unix the guard executable already protects the tree, and this only records
/// the process group.
pub fn protect(child: &mut Child) -> io::Result<ProcessTree> {
    #[cfg(windows)]
    {
        let handle = child
            .raw_handle()
            .ok_or_else(|| io::Error::other("missing process handle"))?;
        let job = winjob::WinJob::protect(handle as windows_sys::Win32::Foundation::HANDLE, true)?;
        Ok(ProcessTree { job: Some(job) })
    }
    #[cfg(not(windows))]
    {
        Ok(ProcessTree::unprotected(child))
    }
}

/// Place a just-spawned (suspended) child in a Job Object that is NOT
/// kill-on-close (Windows): `ProcessTree::kill` then ends the whole tree on
/// cancel, but the tree does not die with the server. `run` uses it for
/// `GuardMode::Off`, so cancellation kills the tree in every mode.
#[cfg(windows)]
pub(crate) fn contain(child: &mut Child) -> io::Result<ProcessTree> {
    let handle = child
        .raw_handle()
        .ok_or_else(|| io::Error::other("missing process handle"))?;
    let job = winjob::WinJob::protect(handle as windows_sys::Win32::Foundation::HANDLE, false)?;
    Ok(ProcessTree { job: Some(job) })
}

/// The process creation flag `run` spawns every Windows backend with: the
/// process starts suspended, so it can start nothing before its Job Object
/// holds it.
#[cfg(windows)]
pub(crate) const CREATE_SUSPENDED: u32 = windows_sys::Win32::System::Threading::CREATE_SUSPENDED;

/// Resume a child created with [`CREATE_SUSPENDED`] (Windows).
#[cfg(windows)]
pub(crate) fn resume(child: &Child) -> io::Result<()> {
    let pid = child
        .id()
        .ok_or_else(|| io::Error::other("the process has no id"))?;
    winjob::resume_process(pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_starts_the_guard_with_the_backend_argv_and_marker() {
        let mut cmd = Command::new("codex");
        cmd.args(["-s", "read-only", "exec"])
            .env("MARK", "1")
            .env_remove("DROPPED")
            .current_dir(std::env::temp_dir());
        let guard = std::env::temp_dir().join("some-dir").join(GUARD_BINARY);
        let wrapped = wrap(&cmd, &guard);
        let std = wrapped.as_std();
        assert_eq!(std.get_program(), guard.as_os_str());
        let args: Vec<String> = std
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                std::process::id().to_string(),
                "--".into(),
                "codex".into(),
                "-s".into(),
                "read-only".into(),
                "exec".into(),
            ]
        );
        let envs: Vec<_> = std.get_envs().collect();
        use std::ffi::OsStr;
        assert!(envs.contains(&(OsStr::new("MARK"), Some(OsStr::new("1")))));
        assert!(envs.contains(&(OsStr::new("DROPPED"), None)));
        assert!(envs.contains(&(OsStr::new(GUARD_ENV), Some(OsStr::new("1")))));
        assert_eq!(std.get_current_dir(), Some(std::env::temp_dir().as_path()));
        // Never the running executable: nothing can re-execute itself.
        if let Ok(me) = std::env::current_exe() {
            assert_ne!(std.get_program(), me.as_os_str());
        }
    }

    #[test]
    fn install_hint_names_the_guard_and_how_to_get_it() {
        let hint = install_hint();
        assert!(hint.contains(GUARD_BINARY), "{hint}");
        assert!(hint.contains("cargo build"), "{hint}");
        // A Required run without the guard must not walk a fallback chain.
        assert_eq!(
            crate::errkind::classify(&hint),
            crate::errkind::BackendErrorKind::Other
        );
    }
}
