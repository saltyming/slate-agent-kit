//! One attempt ([`run`]) and a model fallback chain of attempts
//! ([`run_with_fallback`]), with what they report while running
//! ([`RunEvent`]) and what they return ([`Outcome`], [`RunRecord`]).
//!
//! `run` refuses in a process marked as a guard, looks the binary up, resolves
//! the guard mode, builds the argv (`argv::command`), spawns it — through
//! `agent-guard` or into a Job Object when guarded — writes the prompt to
//! stdin and closes it while capturing stdout and stderr concurrently under
//! the capture policy, and awaits the exit or the cancellation token,
//! whichever comes first. It writes
//! no file and no store: everything goes out through the event channel and the
//! return value.
//!
//! There is intentionally no wall-clock timeout — a backend run can take many
//! minutes. Cancellation is the caller's `CancellationToken`: it kills the
//! process tree (process group or Job Object) and yields `Outcome::Cancelled`.

use std::io;
use std::process::Stdio;
use std::time::{Instant, SystemTime};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use crate::argv;
use crate::capture::{self, Captured};
use crate::discovery::{install_hint, which};
use crate::errkind::{self, BackendErrorKind};
use crate::failure::failure_text;
use crate::guard::{self, ProcessTree};
use crate::spec::{Backend, GuardMode, OutputMode, RunSpec};
use harness_log::Usage;

/// How many characters of a discarded attempt's failure text are kept.
const DISCARDED_DETAIL_CHARS: usize = 2000;

/// What a run reports while it lasts, in order, to the caller's channel.
// `Finished` carries a whole record; an event is sent a few times per
// attempt, so its size costs nothing next to the process it describes.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum RunEvent {
    /// Right after the spawn. `pid` is the spawned process (on Linux and macOS
    /// with the guard, the guard, which lives exactly as long as the backend
    /// subtree and leads its process group);
    /// `argv` is the real backend's command line.
    Started {
        /// The spawned process id.
        pid: Option<u32>,
        /// The backend argv, binary first; never the guard wrapper.
        argv: Vec<String>,
        /// `RunSpec::backend_version`.
        backend_version: Option<String>,
        /// Whether the process runs under the parent-death guard
        /// (`agent-guard` on Linux and macOS, a Job Object on Windows);
        /// `false` for `GuardMode::Off` and for a `GuardMode::Preferred` run
        /// that found no guard.
        guarded: bool,
    },
    /// A backend session became known (a runner that creates or resumes one,
    /// such as dispatch's opencode runner, sends it).
    Session {
        /// The backend's session id.
        session_id: String,
        /// The local port of a server-backed runner, when there is one.
        port: Option<u16>,
    },
    /// One normalized JSONL event a runner produced.
    Progress(String),
    /// Every terminal outcome, including `Cancelled` and a startup failure.
    Finished(Outcome),
}

/// How one attempt ended.
// `Record` carries a whole record; an outcome is produced once per attempt, so
// its size costs nothing next to the process it describes.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Outcome {
    /// The process was spawned and awaited; success or failure is in the
    /// record.
    Record(RunRecord),
    /// `which` found no binary.
    NotFound {
        /// The binary looked up.
        binary: String,
        /// How to install it.
        hint: String,
    },
    /// The process could not be started or its stdin written.
    Spawn(String),
    /// Awaiting the process's exit failed.
    WaitFailed(String),
    /// The cancellation token fired; the process tree was killed.
    Cancelled,
}

/// The serializable result of one attempt. Fields are added, never removed or
/// renamed, within a kit major version; a record missing a field reads it as
/// its default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunRecord {
    /// The backend's name (`codex`, `claude`, or a runner's own, e.g.
    /// `opencode`).
    pub backend: String,
    /// The backend argv, binary first.
    pub argv: Vec<String>,
    /// The model requested; `None` is the backend's default.
    pub model: Option<String>,
    /// The reasoning effort requested.
    pub reasoning_effort: Option<String>,
    /// When the attempt started, in milliseconds since the Unix epoch.
    pub started_at: u64,
    /// Wall-clock duration of the attempt in milliseconds.
    pub wall_ms: u64,
    /// The exit code; `None` when the process was ended by a signal.
    pub exit_code: Option<i32>,
    /// Whether the process exited with status 0.
    pub success: bool,
    /// stdout as kept by the capture policy (the failure cap applied when the
    /// run failed).
    pub stdout: String,
    /// Bytes stdout produced in all.
    pub stdout_total: u64,
    /// Whether anything stdout produced is missing from `stdout`.
    pub stdout_truncated: bool,
    /// stderr as kept by the capture policy.
    pub stderr: String,
    /// Whether anything stderr produced is missing from `stderr`.
    pub stderr_truncated: bool,
    /// The backend's last assistant message: all of stdout for `Default` and
    /// `Text` on success, the result's `result` for claude `Json`, the last
    /// `agent_message` for codex `JsonStream` (`None` when that stream was
    /// truncated).
    pub final_text: Option<String>,
    /// The session id the backend reported (claude `Json`, codex
    /// `JsonStream`).
    pub session_id: Option<String>,
    /// `RunSpec::backend_version`.
    pub backend_version: Option<String>,
    /// Token usage parsed from the output (claude `Json`, codex `JsonStream`,
    /// untruncated), else `None`; a server may attach usage parsed from a
    /// session file afterwards.
    pub usage: Option<Usage>,
}

/// An attempt of a fallback chain that failed in a retry-worthy way and was
/// followed by another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscardedAttempt {
    /// The model tried; `None` is the backend's default.
    pub model: Option<String>,
    /// How its failure classified.
    pub kind: BackendErrorKind,
    /// The first 2,000 characters of its failure text.
    pub detail: String,
}

impl DiscardedAttempt {
    /// The model for display: its name, or `(backend default)`.
    pub fn model_label(&self) -> &str {
        self.model.as_deref().unwrap_or("(backend default)")
    }
}

/// What a fallback chain produced.
#[derive(Debug, Clone)]
pub struct FallbackOutcome {
    /// The last attempt's outcome.
    pub outcome: Outcome,
    /// The model of the last attempt; `None` is the backend's default.
    pub model: Option<String>,
    /// The attempts before it, in order.
    pub discarded: Vec<DiscardedAttempt>,
}

/// Run `models` in order — the first entry is the primary model (`None` for
/// the backend's default), the rest the fallback chain; an empty slice is one
/// attempt with no model. Before each attempt `build(index, model)` returns
/// its `RunSpec`, so the caller prepares what the attempt needs before the
/// spawn. The chain advances while the attempt's failure text classifies as
/// retry-worthy and a model remains; it stops on success, on a failure that is
/// not retry-worthy, on the last model, and on cancellation (checked before
/// each attempt, too). Each attempt sends its own `Started` and `Finished`; an
/// attempt cancelled before it was built sends `Finished(Cancelled)` only.
pub async fn run_with_fallback<F>(
    mut build: F,
    models: &[Option<String>],
    events: &UnboundedSender<RunEvent>,
    ct: &CancellationToken,
) -> FallbackOutcome
where
    F: FnMut(usize, Option<&str>) -> RunSpec,
{
    let single = [None];
    let models: &[Option<String>] = if models.is_empty() { &single } else { models };
    let last_idx = models.len() - 1;
    let mut discarded: Vec<DiscardedAttempt> = Vec::new();

    for (idx, model) in models.iter().enumerate() {
        if ct.is_cancelled() {
            let _ = events.send(RunEvent::Finished(Outcome::Cancelled));
            return FallbackOutcome {
                outcome: Outcome::Cancelled,
                model: model.clone(),
                discarded,
            };
        }
        let spec = build(idx, model.as_deref());
        let outcome = run(&spec, events, ct).await;
        if matches!(outcome, Outcome::Cancelled) {
            return FallbackOutcome {
                outcome,
                model: model.clone(),
                discarded,
            };
        }
        if let Some(text) = failure_text(&spec.failure_text, spec.backend.as_str(), &outcome) {
            let kind = errkind::classify(&text);
            tracing::info!(
                "agent-exec: {} attempt {}/{} model={:?} failed kind={} detail={:.200}",
                spec.backend.binary(),
                idx + 1,
                models.len(),
                model,
                kind.as_str(),
                text
            );
            if kind.is_retry_worthy() && idx != last_idx {
                discarded.push(DiscardedAttempt {
                    model: model.clone(),
                    kind,
                    detail: text.chars().take(DISCARDED_DETAIL_CHARS).collect(),
                });
                continue;
            }
        }
        return FallbackOutcome {
            outcome,
            model: model.clone(),
            discarded,
        };
    }
    // Unreachable: the last attempt always returns above.
    FallbackOutcome {
        outcome: Outcome::Cancelled,
        model: None,
        discarded,
    }
}

/// Perform one attempt and return its outcome, after sending `Started` (when
/// the process was spawned) and `Finished` to `events`. Send errors (the
/// receiver dropped) are ignored.
pub async fn run(
    spec: &RunSpec,
    events: &UnboundedSender<RunEvent>,
    ct: &CancellationToken,
) -> Outcome {
    let outcome = run_inner(spec, events, ct).await;
    let _ = events.send(RunEvent::Finished(outcome.clone()));
    outcome
}

async fn run_inner(
    spec: &RunSpec,
    events: &UnboundedSender<RunEvent>,
    ct: &CancellationToken,
) -> Outcome {
    let started_at = unix_ms();
    let clock = Instant::now();
    let binary = spec.backend.binary();
    // Tripwire: a process that carries the guard marker was started as a
    // guard and is not one; it must not start backends (each would start
    // another guard).
    if std::env::var_os(guard::GUARD_ENV).is_some() {
        return Outcome::Spawn(tripwire_message(binary));
    }
    if ct.is_cancelled() {
        return Outcome::Cancelled;
    }
    if which(binary).is_none() {
        return Outcome::NotFound {
            binary: binary.to_string(),
            hint: install_hint(spec.backend),
        };
    }

    // Linux and macOS: the guard is the `agent-guard` executable, looked up
    // before anything is spawned. Windows attaches a Job Object after the
    // spawn instead and looks nothing up.
    #[cfg(not(windows))]
    let guard_path = match spec.guard {
        GuardMode::Off => None,
        GuardMode::Preferred => guard::locate(),
        GuardMode::Required => match guard::locate() {
            Some(p) => Some(p),
            None => {
                return Outcome::Spawn(guard_missing_message(binary));
            }
        },
    };

    let (built, argv) = argv::command(spec);
    #[cfg(not(windows))]
    let mut cmd = match &guard_path {
        Some(p) => guard::wrap(&built, p),
        None => built,
    };
    #[cfg(windows)]
    let mut cmd = built;
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    #[cfg(unix)]
    {
        // New process group with pgid == child pid, so a group kill on cancel
        // reaps the guard, the backend AND any shells or test runners it
        // spawned. kill_on_drop only reaps the direct child.
        cmd.process_group(0);
    }
    // Start suspended: the Job Object must hold the process before it can
    // start anything, or a child it starts at once would be outside the job
    // and outlive it. Resumed right after the job is attached.
    #[cfg(windows)]
    cmd.creation_flags(guard::CREATE_SUSPENDED);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            #[cfg(not(windows))]
            let what = if guard_path.is_some() {
                format!("{} for {binary}", guard::GUARD_BINARY)
            } else {
                binary.to_string()
            };
            #[cfg(windows)]
            let what = binary.to_string();
            return Outcome::Spawn(format!("spawn {what} failed: {e}"));
        }
    };
    let pid = child.id();

    #[cfg(not(windows))]
    let (tree, guarded) = (ProcessTree::unprotected(&child), guard_path.is_some());
    #[cfg(windows)]
    let (tree, guarded) = {
        let mode = spec.guard;
        let attached = if mode == GuardMode::Off {
            // A plain job, only so that a cancel ends the whole tree; not a
            // guard. Without it the run proceeds and a cancel ends the
            // backend alone.
            guard::contain(&mut child).map(|t| (t, false))
        } else {
            guard::protect(&mut child).map(|t| (t, true))
        };
        let (tree, guarded) = match attached {
            Ok(pair) => pair,
            Err(e) if mode == GuardMode::Required => {
                // Continuing would leave the work running with no orphan
                // protection at all.
                ProcessTree::unprotected(&child).kill(&mut child);
                let _ = child.wait().await;
                return Outcome::Spawn(format!("Job Object setup for {binary} failed: {e}"));
            }
            Err(e) => {
                tracing::warn!(
                    "agent-exec: Job Object setup for {binary} failed: {e}; running without it"
                );
                (ProcessTree::unprotected(&child), false)
            }
        };
        if let Err(e) = guard::resume(&child) {
            tree.kill(&mut child);
            let _ = child.wait().await;
            return Outcome::Spawn(format!("resume {binary} after its spawn failed: {e}"));
        }
        (tree, guarded)
    };
    let _ = events.send(RunEvent::Started {
        pid,
        argv: argv.clone(),
        backend_version: spec.backend_version.clone(),
        guarded,
    });

    let out_task = tokio::spawn(capture::read_capped(
        child.stdout.take(),
        spec.capture.stdout_cap,
    ));
    let err_task = tokio::spawn(capture::read_capped(
        child.stderr.take(),
        spec.capture.stderr_cap,
    ));
    let (out_abort, err_abort) = (out_task.abort_handle(), err_task.abort_handle());
    let abort_readers = || {
        out_abort.abort();
        err_abort.abort();
    };

    // Write the prompt, then close stdin (the writer owns it, so it is closed
    // when the writer finishes or is dropped). The readers already drain
    // stdout and stderr, and the writer runs alongside the wait for the exit,
    // so neither a large prompt nor a large output can block the other side.
    let Some(mut stdin) = child.stdin.take() else {
        tree.kill(&mut child);
        let _ = child.wait().await;
        abort_readers();
        return Outcome::Spawn(format!("{binary} stdin pipe unavailable"));
    };
    let prompt = spec.prompt.as_bytes();
    let mut writer = Some(Box::pin(async move {
        stdin.write_all(prompt).await?;
        stdin.shutdown().await
    }));

    let status = loop {
        let step = tokio::select! {
            biased;
            _ = ct.cancelled() => Step::Cancelled,
            r = async {
                match writer.as_mut() {
                    Some(w) => w.await,
                    None => std::future::pending().await,
                }
            } => Step::Wrote(r),
            r = child.wait() => Step::Exited(r),
        };
        match step {
            Step::Cancelled => {
                // The child is still live (wait did not complete), so the tree
                // is killed while its id is unambiguously ours, then reaped.
                tree.kill(&mut child);
                drop(writer.take());
                let _ = child.wait().await;
                abort_readers();
                return Outcome::Cancelled;
            }
            // Written and closed.
            Step::Wrote(Ok(())) => writer = None,
            // The child closed its stdin (typically: it exited before reading
            // the prompt). Not a failure of its own: the exit status and the
            // captured output decide, so the child's own error text reaches
            // the classifier.
            Step::Wrote(Err(e)) if e.kind() == io::ErrorKind::BrokenPipe => writer = None,
            Step::Wrote(Err(e)) => {
                tree.kill(&mut child);
                drop(writer.take());
                let _ = child.wait().await;
                abort_readers();
                return Outcome::Spawn(format!("write prompt to {binary} failed: {e}"));
            }
            Step::Exited(Err(e)) => {
                tree.kill(&mut child);
                drop(writer.take());
                abort_readers();
                return Outcome::WaitFailed(format!("wait failed: {e}"));
            }
            Step::Exited(Ok(st)) => {
                // The child is gone; an unfinished write can only block now.
                drop(writer.take());
                break st;
            }
        }
    };
    tree.release();

    // Drain the readers. A descendant that inherited stdout and outlives the
    // backend keeps the pipe open; cancellation still ends the wait.
    let drained = tokio::select! {
        biased;
        _ = ct.cancelled() => None,
        pair = async { (out_task.await, err_task.await) } => Some(pair),
    };
    let Some((out, err)) = drained else {
        abort_readers();
        return Outcome::Cancelled;
    };
    let out: Captured = out.unwrap_or_default();
    let err: Captured = err.unwrap_or_default();

    let success = status.success();
    let (stdout, stdout_truncated) = if success {
        (out.text, out.truncated)
    } else {
        let (clipped, cut) = capture::clip(&out.text, spec.capture.failure_stdout_cap);
        (clipped, out.truncated || cut)
    };
    let parsed = parse_output(spec.backend, spec.output_mode, &stdout, stdout_truncated);
    let final_text = match spec.output_mode {
        OutputMode::Json | OutputMode::JsonStream if parsed.structured => parsed.final_text,
        _ if success => Some(stdout.clone()),
        _ => None,
    };

    Outcome::Record(RunRecord {
        backend: binary.to_string(),
        argv,
        model: spec.model.clone(),
        reasoning_effort: spec.reasoning_effort.clone(),
        started_at,
        wall_ms: clock.elapsed().as_millis() as u64,
        exit_code: status.code(),
        success,
        stdout,
        stdout_total: out.total,
        stdout_truncated,
        stderr: err.text,
        stderr_truncated: err.truncated,
        final_text,
        session_id: parsed.session_id,
        backend_version: spec.backend_version.clone(),
        usage: parsed.usage,
    })
}

/// The `Spawn` text of a run refused in a process that carries the guard
/// marker. Like every crate-made `Spawn` text it classifies as `Other`, so a
/// fallback chain stops on it.
fn tripwire_message(binary: &str) -> String {
    format!(
        "refusing to start {binary}: this process carries {} and so was started as a guard, \
         which never starts backends",
        guard::GUARD_ENV
    )
}

/// The `Spawn` text of a `GuardMode::Required` run that found no guard
/// (Linux and macOS; Windows looks no executable up).
#[cfg(any(not(windows), test))]
fn guard_missing_message(binary: &str) -> String {
    format!(
        "cannot start {binary} under the parent-death guard: {}",
        guard::install_hint()
    )
}

/// One step of the wait loop in `run_inner`.
enum Step {
    Cancelled,
    Wrote(io::Result<()>),
    Exited(io::Result<std::process::ExitStatus>),
}

/// What a structured output mode yields besides the raw text.
#[derive(Default)]
struct Parsed {
    /// Whether the (backend, mode) pair is a structured one at all.
    structured: bool,
    final_text: Option<String>,
    session_id: Option<String>,
    usage: Option<Usage>,
}

fn parse_output(backend: Backend, mode: OutputMode, stdout: &str, truncated: bool) -> Parsed {
    match (backend, mode) {
        (Backend::Claude, OutputMode::Json) => {
            let v: Option<Value> = serde_json::from_str(stdout.trim()).ok();
            let str_field = |key: &str| {
                v.as_ref()
                    .and_then(|v| v.get(key))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            };
            Parsed {
                structured: true,
                final_text: str_field("result"),
                session_id: str_field("session_id"),
                usage: if truncated {
                    None
                } else {
                    harness_log::usage::claude_json_result(stdout)
                },
            }
        }
        (Backend::Codex, OutputMode::JsonStream) => {
            let mut session_id = None;
            let mut last_message = None;
            for line in stdout.lines() {
                let Ok(o) = serde_json::from_str::<Value>(line.trim()) else {
                    continue;
                };
                match o.get("type").and_then(Value::as_str) {
                    Some("thread.started") => {
                        session_id = o
                            .get("thread_id")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                    }
                    Some("item.completed") => {
                        let item = o.get("item");
                        if item.and_then(|i| i.get("type")).and_then(Value::as_str)
                            == Some("agent_message")
                            && let Some(text) =
                                item.and_then(|i| i.get("text")).and_then(Value::as_str)
                        {
                            last_message = Some(text.to_string());
                        }
                    }
                    _ => {}
                }
            }
            Parsed {
                structured: true,
                // A head-truncated stream lost its later messages.
                final_text: if truncated { None } else { last_message },
                session_id,
                usage: if truncated {
                    None
                } else {
                    harness_log::usage::codex_json_stream(stdout)
                },
            }
        }
        _ => Parsed::default(),
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_json_output_gives_text_session_and_usage() {
        let out = r#"{"type":"result","result":"the answer","session_id":"s-9",
"usage":{"input_tokens":3,"output_tokens":4}}"#;
        let p = parse_output(Backend::Claude, OutputMode::Json, out, false);
        assert_eq!(p.final_text.as_deref(), Some("the answer"));
        assert_eq!(p.session_id.as_deref(), Some("s-9"));
        assert_eq!(p.usage.and_then(|u| u.output), Some(4));
        let cut = parse_output(Backend::Claude, OutputMode::Json, out, true);
        assert_eq!(cut.usage, None);
    }

    #[test]
    fn codex_json_stream_gives_last_message_thread_and_usage() {
        let out = r#"{"type":"thread.started","thread_id":"th-1"}
{"type":"item.completed","item":{"id":"i0","type":"reasoning","text":"thinking"}}
{"type":"item.completed","item":{"id":"i1","type":"agent_message","text":"first"}}
{"type":"item.completed","item":{"id":"i2","type":"agent_message","text":"final"}}
{"type":"turn.completed","usage":{"input_tokens":10,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":2,"reasoning_output_tokens":1}}"#;
        let p = parse_output(Backend::Codex, OutputMode::JsonStream, out, false);
        assert_eq!(p.final_text.as_deref(), Some("final"));
        assert_eq!(p.session_id.as_deref(), Some("th-1"));
        assert_eq!(p.usage.and_then(|u| u.input_uncached), Some(10));
        let cut = parse_output(Backend::Codex, OutputMode::JsonStream, out, true);
        assert_eq!(cut.final_text, None);
        assert_eq!(cut.usage, None);
        assert_eq!(cut.session_id.as_deref(), Some("th-1"));
    }

    #[test]
    fn plain_modes_parse_nothing() {
        for (b, m) in [
            (Backend::Codex, OutputMode::Default),
            (Backend::Claude, OutputMode::Text),
            (Backend::Codex, OutputMode::Json),
            (Backend::Claude, OutputMode::JsonStream),
        ] {
            let p = parse_output(b, m, r#"{"type":"result","result":"x"}"#, false);
            assert!(!p.structured);
            assert_eq!(p.usage, None);
        }
    }

    #[test]
    fn record_round_trips_and_reads_missing_fields_as_defaults() {
        let r = RunRecord {
            backend: "codex".into(),
            argv: vec!["codex".into(), "exec".into()],
            exit_code: Some(0),
            success: true,
            stdout: "ok".into(),
            stdout_total: 2,
            ..RunRecord::default()
        };
        let s = serde_json::to_string(&r).unwrap();
        let back: RunRecord = serde_json::from_str(&s).unwrap();
        assert_eq!(back, r);
        let partial: RunRecord = serde_json::from_str(r#"{"backend":"claude"}"#).unwrap();
        assert_eq!(partial.backend, "claude");
        assert_eq!(partial.usage, None);
    }

    #[test]
    fn crate_made_spawn_texts_are_not_retry_worthy() {
        for backend in Backend::all() {
            for text in [
                tripwire_message(backend.binary()),
                guard_missing_message(backend.binary()),
            ] {
                assert_eq!(errkind::classify(&text), BackendErrorKind::Other, "{text}");
            }
        }
    }

    #[test]
    fn discarded_attempt_labels_the_default_model() {
        let d = DiscardedAttempt {
            model: None,
            kind: BackendErrorKind::RateLimited,
            detail: String::new(),
        };
        assert_eq!(d.model_label(), "(backend default)");
    }
}
