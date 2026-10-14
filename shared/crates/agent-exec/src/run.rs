//! One attempt ([`run`]) and a model fallback chain of attempts
//! ([`run_with_fallback`]), with what they report while running
//! ([`RunEvent`]) and what they return ([`Outcome`], [`RunRecord`]).
//!
//! `run` refuses in a process marked as a guard and a sandbox level the
//! backend cannot run under, looks the binary up, builds the argv
//! (`argv::command`), spawns it under the guard mode
//! (`guard::spawn_contained`), writes the prompt to stdin in the backend's
//! input encoding and closes it while reading stdout and stderr concurrently,
//! and awaits the exit or the cancellation token, whichever comes first. The
//! backend's result normaliser then decides the outcome: the exit status for
//! codex and claude, the `result` event of the stream for agy (`agy.rs`), whose
//! stdout reader also reports the conversation as `RunEvent::Session` while
//! the run lasts. It writes no file and no store: everything goes out through
//! the event channel and the return value.
//!
//! There is intentionally no wall-clock timeout — a backend run can take many
//! minutes. Cancellation is the caller's `CancellationToken`: it kills the
//! process tree (process group or Job Object) and yields `Outcome::Cancelled`.
//! An exit already observed when the token fires wins: its record is the
//! result. After the exit the tree stays owned until stdout and stderr are
//! drained; a cancellation while a descendant still holds one of them open
//! kills the whole tree and yields `Outcome::Cancelled`.

use std::io;
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use crate::agy;
use crate::argv;
use crate::capture::{self, Captured};
use crate::discovery::{install_hint, which};
use crate::errkind::{self, BackendErrorKind};
use crate::failure::failure_text;
use crate::guard::{self, Contained};
use crate::spec::{Backend, OutputMode, ResultNormaliser, RunSpec};
use harness_log::Usage;

/// How long a run that was cancelled after its backend exited waits for the
/// output readers before it kills the tree: enough for readers whose pipes
/// are already closed to reach their end, far less than any descendant that
/// holds a pipe open would take.
const DRAIN_GRACE: Duration = Duration::from_millis(500);

/// How many characters of a discarded attempt's failure text are kept.
const DISCARDED_DETAIL_CHARS: usize = 2000;

/// What a run reports while it lasts, in order, to the caller's channel.
// `Finished` carries a whole record; an event is sent a few times per
// attempt, so its size costs nothing next to the process it describes.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum RunEvent {
    /// Right after the spawn. `pid` is the spawned process (on Linux and macOS
    /// with the guard, the guard, which leads the process group and exits
    /// when the program it started exits; descendants of the program may
    /// outlive it); `argv` is the real backend's command line.
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
    /// A backend session became known: dispatch's opencode runner sends it
    /// after creating or resuming one, and an agy run as soon as its `init`
    /// event names the conversation.
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
    /// The process could not be started or its stdin written, or the backend
    /// refuses the run's sandbox level (`Backend::sandbox_refusal`).
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
    /// The backend's name (`codex`, `claude`, `agy`, or a runner's own, e.g.
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
    /// Whether the run succeeded, as the backend's result normaliser decided:
    /// exit status 0 for codex and claude; for agy also a well-formed
    /// `result` event that reports success. `exit_code` stays as observed, so
    /// an agy run can exit 0 and still fail.
    pub success: bool,
    /// stdout as kept by the capture policy (the failure cap applied when the
    /// run failed); for agy, the text deltas of its event stream.
    pub stdout: String,
    /// Bytes stdout produced in all (for agy, bytes of text deltas).
    pub stdout_total: u64,
    /// Whether anything stdout produced is missing from `stdout`.
    pub stdout_truncated: bool,
    /// stderr as kept by the capture policy.
    pub stderr: String,
    /// Whether anything stderr produced is missing from `stderr`.
    pub stderr_truncated: bool,
    /// The run's answer, which a server reports as its result or reply: under
    /// the exit-status normaliser the retained `stdout` itself for `Default`
    /// and `Text` (success or failure), the result's `result` for claude
    /// `Json` and the last `agent_message` for codex `JsonStream` (`None`
    /// when that stream was truncated); for agy the `result` event's
    /// `response` whole, with a note naming the denied actions when there
    /// were any (`None` without a well-formed `result`).
    pub final_text: Option<String>,
    /// The result normaliser's own account of a failure, which the failure
    /// text carries before stdout: for agy the result's `error`, an
    /// `AGY_ERROR` line, a missing or oversized `result` event, or the
    /// actions headless agy could not get permission for. `None` under the
    /// exit-status normaliser and on success.
    pub failure_detail: Option<String>,
    /// The session id the backend reported (claude `Json`, codex
    /// `JsonStream`, the agy conversation).
    pub session_id: Option<String>,
    /// `RunSpec::backend_version`.
    pub backend_version: Option<String>,
    /// Token usage parsed from the output (claude `Json`, codex `JsonStream`,
    /// untruncated; agy's `result` event), else `None`; a server may attach
    /// usage parsed from a session file afterwards.
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

/// What one attempt of a fallback chain produced: its outcome, the text the
/// classifier reads when it failed (`None` for a success or a cancellation),
/// and the name the attempt is logged under.
#[derive(Debug, Clone)]
pub struct AttemptResult {
    /// The attempt's outcome.
    pub outcome: Outcome,
    /// The failure text, as [`failure_text`] builds it; `None` when the
    /// attempt did not fail.
    pub failure_text: Option<String>,
    /// The backend's name, for the log line of a failed attempt.
    pub label: String,
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
    fallback_chain(
        |idx: usize, model: Option<String>| {
            // Built before the attempt's future starts, so the caller's
            // preparation is done when the spawn happens.
            let spec = build(idx, model.as_deref());
            async move {
                let outcome = run(&spec, events, ct).await;
                AttemptResult {
                    failure_text: failure_text(&spec.failure_text, spec.backend.as_str(), &outcome),
                    label: spec.backend.binary().to_string(),
                    outcome,
                }
            }
        },
        models,
        events,
        ct,
    )
    .await
}

/// The chain behind [`run_with_fallback`], for an attempt that is not a
/// `RunSpec` of this crate (a backend driven some other way): `attempt(index,
/// model)` performs one attempt, reports through `events` itself and returns
/// its [`AttemptResult`]. The order, the stopping rules and the discarded
/// attempts are those of [`run_with_fallback`].
pub async fn fallback_chain<F, Fut>(
    mut attempt: F,
    models: &[Option<String>],
    events: &UnboundedSender<RunEvent>,
    ct: &CancellationToken,
) -> FallbackOutcome
where
    F: FnMut(usize, Option<String>) -> Fut,
    Fut: Future<Output = AttemptResult>,
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
        let AttemptResult {
            outcome,
            failure_text: text,
            label,
        } = attempt(idx, model.clone()).await;
        if matches!(outcome, Outcome::Cancelled) {
            return FallbackOutcome {
                outcome,
                model: model.clone(),
                discarded,
            };
        }
        if let Some(text) = text {
            let kind = errkind::classify(&text);
            tracing::info!(
                "agent-exec: {} attempt {}/{} model={:?} failed kind={} detail={:.200}",
                label,
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
    if let Some(refusal) = spec.backend.sandbox_refusal(spec.sandbox) {
        return Outcome::Spawn(refusal.to_string());
    }
    if which(binary).is_none() {
        return Outcome::NotFound {
            binary: binary.to_string(),
            hint: install_hint(spec.backend),
        };
    }

    let (built, argv) = argv::command(spec);
    let contained = guard::spawn_contained(built, spec.guard, binary, |cmd| {
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
    })
    .await;
    let Contained {
        mut child,
        tree,
        guarded,
    } = match contained {
        Ok(c) => c,
        Err(msg) => return Outcome::Spawn(msg),
    };
    let pid = child.id();
    let _ = events.send(RunEvent::Started {
        pid,
        argv: argv.clone(),
        backend_version: spec.backend_version.clone(),
        guarded,
    });

    // stdout is read by the backend's normaliser: kept under the cap as it
    // comes (exit status), or parsed as events whose text deltas are kept
    // under the cap (agy), the conversation reported the moment it is read.
    let stdout = child.stdout.take();
    let stdout_cap = spec.capture.stdout_cap;
    let out_task = match spec.backend.result_normaliser() {
        ResultNormaliser::ExitStatus => tokio::spawn(async move {
            StdoutRead {
                captured: capture::read_capped(stdout, stdout_cap).await,
                stream: None,
            }
        }),
        ResultNormaliser::AgyStreamJson => {
            let session_events = events.clone();
            tokio::spawn(async move {
                let mut stream = agy::read_stream(stdout, stdout_cap, move |id: &str| {
                    let _ = session_events.send(RunEvent::Session {
                        session_id: id.to_string(),
                        port: None,
                    });
                })
                .await;
                StdoutRead {
                    captured: std::mem::take(&mut stream.deltas),
                    stream: Some(stream),
                }
            })
        }
    };
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
    let input = spec.backend.input_encoding().encode(&spec.prompt);
    let input: &[u8] = &input;
    let mut writer = Some(Box::pin(async move {
        stdin.write_all(input).await?;
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
                // An exit that has already happened wins over a cancellation
                // arriving at the same time: its record is the result.
                if let Ok(Some(st)) = child.try_wait() {
                    drop(writer.take());
                    break st;
                }
                // The child is still live, so the tree is killed while its id
                // is unambiguously ours, then reaped.
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

    // Drain the readers. The tree stays owned until they end: a descendant
    // that inherited stdout or stderr and outlives the backend keeps the pipe
    // open, and a cancellation then kills the whole tree (the process group
    // still exists while a member lives; the Job Object holds the rest).
    let mut readers = Box::pin(async { (out_task.await, err_task.await) });
    let drained = tokio::select! {
        biased;
        pair = &mut readers => Some(pair),
        _ = ct.cancelled() => None,
    };
    let drained = match drained {
        Some(pair) => Some(pair),
        // Readers of pipes nothing holds any more reach their end at once;
        // only a held pipe outlasts the grace.
        None => tokio::time::timeout(DRAIN_GRACE, &mut readers).await.ok(),
    };
    let Some((out, err)) = drained else {
        tree.kill(&mut child);
        abort_readers();
        return Outcome::Cancelled;
    };
    tree.release();
    let StdoutRead {
        captured: out,
        stream,
    } = out.unwrap_or_default();
    let err: Captured = err.unwrap_or_default();

    let agy_outcome = stream
        .as_ref()
        .map(|s| agy::normalise(status.success(), s, &err.text));
    let success = match &agy_outcome {
        Some(n) => n.success,
        None => status.success(),
    };
    let (stdout, stdout_truncated) = if success {
        (out.text, out.truncated)
    } else {
        let (clipped, cut) = capture::clip(&out.text, spec.capture.failure_stdout_cap);
        (clipped, out.truncated || cut)
    };
    let parsed = match agy_outcome {
        Some(n) => Parsed {
            structured: true,
            final_text: n.final_text,
            failure_detail: n.failure_detail,
            session_id: n.session_id,
            usage: n.usage,
        },
        None => {
            let parsed = parse_output(spec.backend, spec.output_mode, &stdout, stdout_truncated);
            let final_text = match spec.output_mode {
                OutputMode::Json | OutputMode::JsonStream if parsed.structured => parsed.final_text,
                // The retained stdout is the answer, on success and failure.
                _ => Some(stdout.clone()),
            };
            Parsed {
                final_text,
                ..parsed
            }
        }
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
        final_text: parsed.final_text,
        failure_detail: parsed.failure_detail,
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

/// One step of the wait loop in `run_inner`.
enum Step {
    Cancelled,
    Wrote(io::Result<()>),
    Exited(io::Result<std::process::ExitStatus>),
}

/// What stdout's reader produced: the retained text and, for agy, the event
/// stream it parsed.
#[derive(Default)]
struct StdoutRead {
    captured: Captured,
    stream: Option<agy::Stream>,
}

/// What a structured output mode yields besides the raw text.
#[derive(Default)]
struct Parsed {
    /// Whether the (backend, mode) pair is a structured one at all.
    structured: bool,
    final_text: Option<String>,
    failure_detail: Option<String>,
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
                failure_detail: None,
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
                failure_detail: None,
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
                guard::missing_message(backend.binary()),
            ] {
                assert_eq!(errkind::classify(&text), BackendErrorKind::Other, "{text}");
            }
            for sandbox in [
                crate::Sandbox::ReadOnly,
                crate::Sandbox::WorkspaceWrite,
                crate::Sandbox::DangerFullAccess,
            ] {
                if let Some(text) = backend.sandbox_refusal(sandbox) {
                    assert_eq!(errkind::classify(text), BackendErrorKind::Other, "{text}");
                }
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
