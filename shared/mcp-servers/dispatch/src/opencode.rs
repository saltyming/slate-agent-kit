//! OpenCode server-backed dispatch runner.
//!
//! Unlike the Codex backend, OpenCode is driven through a short-lived local
//! `opencode serve` process. That gives dispatch stable session ids, abort, and
//! event streaming without scraping an interactive transcript. The log file we
//! expose through `dispatch_logs` is dispatch-owned normalized JSONL; every event
//! keeps the native OpenCode event under `native` for debugging.
//!
//! Entry point: [`run`], one attempt. It reports through `agent_exec::RunEvent`
//! exactly as the crate's process runs do — `Started` right after the server
//! spawn, `Session` once the session exists and its log is initialized,
//! `Progress` for each normalized JSONL line it appends to the log, `Finished`
//! with every terminal outcome — so the executor writes the store for every
//! backend in one place. The `opencode serve` process runs under the
//! parent-death guard (`agent-guard` on Linux and macOS, a Job Object on
//! Windows) and carries dispatch's re-entry marker.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime};

use agent_exec::guard::{self, ProcessTree};
use agent_exec::{Outcome, Reentry, RunEvent, RunRecord};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use crate::backend::REENTRY_ENV;

/// The binary looked up on `PATH`, and the backend's name in records.
pub const BINARY: &str = "opencode";

const STARTUP_TIMEOUT_MS: u64 = 10_000;
const STARTUP_POLL_MS: u64 = 150;
const SERVER_STDERR_CAP: usize = 16 * 1024;

pub struct RunSpec<'a> {
    pub id: &'a str,
    pub working_dir: &'a Path,
    pub sandbox: &'a str,
    pub model: Option<&'a str>,
    pub reasoning_effort: Option<&'a str>,
    pub prompt: &'a str,
    pub state_dir: &'a Path,
    pub backend_version: Option<&'a str>,
    pub resume_session: Option<&'a str>,
    pub rollout_path: Option<&'a Path>,
}

/// One line telling how to install the opencode CLI.
pub fn install_hint() -> String {
    "install opencode CLI (see https://opencode.ai/docs/cli/)".to_string()
}

/// `opencode --version` with dispatch's re-entry marker stamped; `None` when
/// the binary is missing, fails to run or prints nothing.
pub async fn version(reentry: &Reentry) -> Option<String> {
    agent_exec::version_of(BINARY, reentry).await
}

/// The dispatch-owned log of an opencode session: the log a resumed task
/// inherits (`rollout_path`), else a fresh file under the state directory.
pub fn log_path(rollout_path: Option<&Path>, state_dir: &Path, session_id: &str) -> PathBuf {
    rollout_path
        .map(PathBuf::from)
        .unwrap_or_else(|| opencode_log_path(state_dir, session_id))
}

/// Perform one opencode attempt and return its outcome, after sending
/// `Started` (once the server process exists), `Session`, `Progress` and
/// `Finished` to `events`. Send errors (the receiver dropped) are ignored.
pub async fn run(
    spec: RunSpec<'_>,
    events: &UnboundedSender<RunEvent>,
    ct: &CancellationToken,
) -> Outcome {
    let outcome = run_inner(&spec, events, ct).await;
    let _ = events.send(RunEvent::Finished(outcome.clone()));
    outcome
}

async fn run_inner(
    spec: &RunSpec<'_>,
    events: &UnboundedSender<RunEvent>,
    ct: &CancellationToken,
) -> Outcome {
    let started_at = unix_ms();
    let clock = Instant::now();
    if agent_exec::which(BINARY).is_none() {
        return Outcome::NotFound {
            binary: BINARY.to_string(),
            hint: install_hint(),
        };
    }

    let port = match open_port() {
        Ok(p) => p,
        Err(e) => return Outcome::Spawn(format!("allocate localhost port failed: {e}")),
    };
    let base_url = format!("http://127.0.0.1:{port}");
    let argv = server_argv(port);
    let (mut child, tree) = match spawn_server(spec.working_dir, &argv) {
        Ok(pair) => pair,
        Err(e) => return Outcome::Spawn(e),
    };
    let _ = events.send(RunEvent::Started {
        pid: child.id(),
        argv: argv.clone(),
        backend_version: spec.backend_version.map(str::to_string),
        guarded: true,
    });
    let stderr = child.stderr.take();
    let stderr_task = tokio::spawn(read_capped_opt(stderr, SERVER_STDERR_CAP));
    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            stop_server(&tree, &mut child).await;
            return Outcome::WaitFailed(format!("http client build failed: {e}"));
        }
    };

    if let Err(e) = wait_health(&client, &base_url, ct).await {
        stop_server(&tree, &mut child).await;
        let stderr = stderr_task.await.unwrap_or_default();
        return Outcome::WaitFailed(format!("{e}\n{}", stderr.trim()));
    }

    let session_id = match spec.resume_session {
        Some(s) => s.to_string(),
        None => match create_session(&client, &base_url, spec).await {
            Ok(s) => s,
            Err(e) => {
                stop_server(&tree, &mut child).await;
                let stderr = stderr_task.await.unwrap_or_default();
                return Outcome::WaitFailed(format!("{e}\n{}", stderr.trim()));
            }
        },
    };

    let log = Log {
        path: log_path(spec.rollout_path, spec.state_dir, &session_id),
        events: events.clone(),
    };
    if let Err(e) = init_log(&log, &session_id, spec.working_dir, spec.prompt).await {
        stop_server(&tree, &mut child).await;
        return Outcome::WaitFailed(format!("initialize opencode log failed: {e}"));
    }
    let _ = events.send(RunEvent::Session {
        session_id: session_id.clone(),
        port: Some(port),
    });

    let event_ct = ct.child_token();
    let event_client = client.clone();
    let event_base = base_url.clone();
    let event_sid = session_id.clone();
    let event_log = log.clone();
    let event_task_ct = event_ct.clone();
    let event_task = tokio::spawn(async move {
        stream_events(
            event_client,
            event_base,
            event_sid,
            event_log,
            event_task_ct,
        )
        .await
    });

    let message = send_message(&client, &base_url, &session_id, spec);
    let response = tokio::select! {
        biased;
        _ = ct.cancelled() => {
            let _ = abort_session(&client, &base_url, &session_id).await;
            event_task.abort();
            stop_server(&tree, &mut child).await;
            return Outcome::Cancelled;
        }
        r = message => r,
    };

    event_ct.cancel();
    let _ = event_task.await;
    stop_server(&tree, &mut child).await;
    let stderr = stderr_task.await.unwrap_or_default();

    let record = |exit_code: i32, stdout: String, stdout_total: u64, stderr: String| RunRecord {
        backend: BINARY.to_string(),
        argv: argv.clone(),
        model: spec.model.map(str::to_string),
        reasoning_effort: spec.reasoning_effort.map(str::to_string),
        started_at,
        wall_ms: clock.elapsed().as_millis() as u64,
        exit_code: Some(exit_code),
        success: exit_code == 0,
        final_text: (exit_code == 0).then(|| stdout.clone()),
        stdout,
        stdout_total,
        stdout_truncated: false,
        stderr,
        stderr_truncated: false,
        session_id: Some(session_id.clone()),
        backend_version: spec.backend_version.map(str::to_string),
        usage: None,
    };
    match response {
        Ok(v) => {
            let text = assistant_text(&v);
            let _ = log
                .append(agent_message(&text, json!({"final": true})))
                .await;
            let _ = log.append(task_complete(&text, None)).await;
            let error = v.pointer("/info/error").filter(|e| !e.is_null());
            if let Some(e) = error {
                Outcome::Record(record(1, text, 0, format!("opencode error: {e}\n{stderr}")))
            } else {
                let total = text.len() as u64;
                Outcome::Record(record(0, text, total, stderr))
            }
        }
        Err(e) => {
            let _ = log.append(task_complete("", Some(&e))).await;
            Outcome::Record(record(1, String::new(), 0, format!("{e}\n{stderr}")))
        }
    }
}

/// The `opencode serve` argv for `port`, recorded as the run's argv.
fn server_argv(port: u16) -> Vec<String> {
    vec![
        BINARY.to_string(),
        "serve".to_string(),
        "--hostname".to_string(),
        "127.0.0.1".to_string(),
        "--port".to_string(),
        port.to_string(),
    ]
}

/// Start `opencode serve` under the parent-death guard, which is required:
/// on Linux and macOS through `agent-guard` (the guard leads the process group
/// and `opencode serve` runs as its child, so the whole subtree dies with
/// dispatch even under a hard `SIGKILL`); on Windows in a kill-on-close Job
/// Object, whose setup failure kills the process and fails the run rather than
/// leave it running unprotected. The returned tree kills the whole subtree.
fn spawn_server(working_dir: &Path, argv: &[String]) -> Result<(Child, ProcessTree), String> {
    let mut built = Command::new(&argv[0]);
    built.args(&argv[1..]).current_dir(working_dir);
    // Mark the opencode server (and its whole subtree) as dispatch-spawned so a
    // nested dispatch submit/steer from within the opencode session is refused.
    agent_exec::reentry::stamp(&mut built, REENTRY_ENV);

    #[cfg(not(windows))]
    let mut cmd = match guard::locate() {
        Some(path) => guard::wrap(&built, &path),
        None => {
            return Err(format!(
                "cannot start opencode serve under the parent-death guard: {}",
                guard::install_hint()
            ));
        }
    };
    #[cfg(windows)]
    let mut cmd = built;

    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut child = cmd.spawn().map_err(|e| {
        #[cfg(not(windows))]
        {
            format!(
                "spawn {} for opencode serve failed: {e}",
                guard::GUARD_BINARY
            )
        }
        #[cfg(windows)]
        {
            format!("spawn opencode serve failed: {e}")
        }
    })?;
    #[cfg(windows)]
    let tree = match guard::protect(&mut child) {
        Ok(t) => t,
        Err(e) => {
            let _ = child.start_kill();
            return Err(format!("Job Object setup for opencode serve failed: {e}"));
        }
    };
    #[cfg(not(windows))]
    let tree = ProcessTree::unprotected(&child);
    Ok((child, tree))
}

/// Kill the server's whole tree while its id is ours, then reap it.
async fn stop_server(tree: &ProcessTree, child: &mut Child) {
    tree.kill(child);
    let _ = child.wait().await;
}

/// The dispatch-owned log file, plus the channel each appended line is also
/// reported on as `RunEvent::Progress`.
#[derive(Clone)]
struct Log {
    path: PathBuf,
    events: UnboundedSender<RunEvent>,
}

impl Log {
    /// Append one normalized event as a JSONL line and report it.
    async fn append(&self, v: Value) -> std::io::Result<()> {
        let line = v.to_string();
        append_line(&self.path, &line).await?;
        let _ = self.events.send(RunEvent::Progress(line));
        Ok(())
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn open_port() -> std::io::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

async fn wait_health(
    client: &reqwest::Client,
    base_url: &str,
    ct: &CancellationToken,
) -> Result<(), String> {
    let mut waited = 0u64;
    while waited < STARTUP_TIMEOUT_MS {
        if ct.is_cancelled() {
            return Err("cancelled while waiting for opencode server startup".to_string());
        }
        if let Ok(resp) = client.get(format!("{base_url}/global/health")).send().await
            && resp.status().is_success()
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(STARTUP_POLL_MS)).await;
        waited += STARTUP_POLL_MS;
    }
    Err("opencode server did not become healthy before startup timeout".to_string())
}

async fn create_session(
    client: &reqwest::Client,
    base_url: &str,
    spec: &RunSpec<'_>,
) -> Result<String, String> {
    let mut body = json!({
        "title": format!("dispatch {}", spec.id),
        "metadata": { "dispatch_task": spec.id },
        "permission": permission_rules(spec.sandbox),
    });
    if let Some((provider, model)) = parse_provider_model(spec.model)? {
        body["model"] = json!({
            "providerID": provider,
            "id": model,
        });
        if let Some(v) = spec.reasoning_effort {
            body["model"]["variant"] = json!(v);
        }
    }
    let resp = client
        .post(format!("{base_url}/session"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("create opencode session failed: {e}"))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("read create-session response failed: {e}"))?;
    if !status.is_success() {
        return Err(format!("create opencode session returned {status}: {text}"));
    }
    let v: Value = serde_json::from_str(&text)
        .map_err(|e| format!("parse create-session response failed: {e}: {text}"))?;
    v.get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("create opencode session response had no id: {v}"))
}

async fn send_message(
    client: &reqwest::Client,
    base_url: &str,
    session_id: &str,
    spec: &RunSpec<'_>,
) -> Result<Value, String> {
    let mut body = json!({
        "parts": [{ "type": "text", "text": spec.prompt }],
    });
    if let Some((provider, model)) = parse_provider_model(spec.model)? {
        body["model"] = json!({
            "providerID": provider,
            "modelID": model,
        });
    }
    if let Some(v) = spec.reasoning_effort {
        body["variant"] = json!(v);
    }
    let resp = client
        .post(format!("{base_url}/session/{session_id}/message"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("send opencode message failed: {e}"))?;
    response_json(resp).await
}

async fn abort_session(
    client: &reqwest::Client,
    base_url: &str,
    session_id: &str,
) -> Result<(), String> {
    let resp = client
        .post(format!("{base_url}/session/{session_id}/abort"))
        .send()
        .await
        .map_err(|e| format!("abort opencode session failed: {e}"))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("abort opencode session returned {}", resp.status()))
    }
}

async fn response_json(resp: reqwest::Response) -> Result<Value, String> {
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("read response failed: {e}"))?;
    if !status.is_success() {
        return Err(format!("opencode API returned {status}: {text}"));
    }
    serde_json::from_str(&text).map_err(|e| format!("parse opencode response failed: {e}: {text}"))
}

fn parse_provider_model(model: Option<&str>) -> Result<Option<(&str, &str)>, String> {
    let Some(raw) = model.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let Some((provider, model_id)) = raw.split_once('/') else {
        return Err(format!(
            "opencode model must be provider/model (for example anthropic/claude-sonnet-4), got {raw:?}"
        ));
    };
    if provider.trim().is_empty() || model_id.trim().is_empty() {
        return Err(format!(
            "opencode model must be provider/model, got {raw:?}"
        ));
    }
    Ok(Some((provider.trim(), model_id.trim())))
}

fn permission_rules(sandbox: &str) -> Value {
    match sandbox {
        "read-only" => json!([
            { "permission": "edit", "pattern": "*", "action": "deny" },
            { "permission": "bash", "pattern": "*", "action": "deny" },
            { "permission": "task", "pattern": "*", "action": "deny" },
            { "permission": "external_directory", "pattern": "*", "action": "deny" },
            { "permission": "question", "pattern": "*", "action": "deny" }
        ]),
        "danger-full-access" => json!([
            { "permission": "external_directory", "pattern": "*", "action": "allow" },
            { "permission": "question", "pattern": "*", "action": "deny" }
        ]),
        _ => json!([
            { "permission": "external_directory", "pattern": "*", "action": "deny" },
            { "permission": "question", "pattern": "*", "action": "deny" }
        ]),
    }
}

fn opencode_log_path(state_dir: &Path, session_id: &str) -> PathBuf {
    state_dir
        .join("logs")
        .join("opencode")
        .join(format!("{session_id}.jsonl"))
}

async fn init_log(
    log: &Log,
    session_id: &str,
    working_dir: &Path,
    prompt: &str,
) -> std::io::Result<()> {
    if let Some(parent) = log.path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if !log.path.exists() {
        log.append(json!({
                "type": "session_meta",
                "payload": {
                    "backend": "opencode",
                    "session_id": session_id,
                    "cwd": working_dir.to_string_lossy(),
                }
        }))
        .await?;
    }
    log.append(json!({"type": "event_msg", "payload": {"type": "task_started"}}))
        .await?;
    log.append(json!({"type": "event_msg", "payload": {"type": "user_message", "message": prompt}, "backend": "opencode"}))
        .await
}

async fn stream_events(
    client: reqwest::Client,
    base_url: String,
    session_id: String,
    log: Log,
    ct: CancellationToken,
) {
    let resp = match client.get(format!("{base_url}/event")).send().await {
        Ok(r) if r.status().is_success() => r,
        _ => return,
    };
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    loop {
        let next = tokio::select! {
            biased;
            _ = ct.cancelled() => break,
            n = stream.next() => n,
        };
        let Some(Ok(bytes)) = next else { break };
        buf.push_str(&String::from_utf8_lossy(&bytes));
        while let Some((block, rest)) = take_sse_block(&buf) {
            buf = rest;
            if let Some(v) = parse_sse_data(&block) {
                for event in normalize_event(&session_id, &v) {
                    let _ = log.append(event).await;
                }
            }
        }
    }
}

fn take_sse_block(buf: &str) -> Option<(String, String)> {
    let (idx, delim_len) = match (buf.find("\r\n\r\n"), buf.find("\n\n")) {
        (Some(crlf), Some(lf)) if crlf < lf => (crlf, 4),
        (Some(crlf), None) => (crlf, 4),
        (_, Some(lf)) => (lf, 2),
        (None, None) => return None,
    };
    let block = buf[..idx].to_string();
    let rest = buf[idx + delim_len..].to_string();
    Some((block, rest))
}

fn parse_sse_data(block: &str) -> Option<Value> {
    let data = block
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n");
    if data.is_empty() || data == "[DONE]" {
        return None;
    }
    serde_json::from_str(&data).ok()
}

fn normalize_event(session_id: &str, native: &Value) -> Vec<Value> {
    let props = native.get("properties").unwrap_or(&Value::Null);
    if props
        .get("sessionID")
        .and_then(Value::as_str)
        .is_some_and(|sid| sid != session_id)
    {
        return Vec::new();
    }
    let Some(kind) = native.get("type").and_then(Value::as_str) else {
        return Vec::new();
    };
    match kind {
        "message.part.updated" => normalize_part_updated(props, native),
        "message.part.delta" => Vec::new(),
        "file.edited" => {
            let mut changes = serde_json::Map::new();
            changes.insert(
                props
                    .get("file")
                    .and_then(Value::as_str)
                    .unwrap_or("(unknown)")
                    .to_string(),
                json!({ "type": "change" }),
            );
            vec![json!({
                "type": "event_msg",
                "payload": {
                    "type": "patch_apply_end",
                    "success": true,
                    "changes": Value::Object(changes),
                },
                "backend": "opencode",
                "native": native,
            })]
        }
        "session.error" => vec![json!({
            "type": "event_msg",
            "payload": {
                "type": "agent_message",
                "message": format!("opencode error: {}", props.get("error").map(Value::to_string).unwrap_or_default()),
            },
            "backend": "opencode",
            "native": native,
        })],
        "session.idle" => vec![json!({
            "type": "event_msg",
            "payload": { "type": "task_complete", "last_agent_message": "opencode session idle" },
            "backend": "opencode",
            "native": native,
        })],
        _ => Vec::new(),
    }
}

fn normalize_part_updated(props: &Value, native: &Value) -> Vec<Value> {
    let Some(part) = props.get("part") else {
        return Vec::new();
    };
    let part_id = part_id(part);
    match part.get("type").and_then(Value::as_str) {
        Some("text") => part
            .get("text")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(|text| vec![agent_message_with_part_id(text, native.clone(), part_id)])
            .unwrap_or_default(),
        Some("reasoning") => part
            .get("text")
            .and_then(Value::as_str)
            .map(|text| vec![reasoning_part(text, native.clone(), part_id)])
            .unwrap_or_default(),
        Some("tool") => normalize_tool_part(part, native, part_id),
        _ => Vec::new(),
    }
}

fn normalize_tool_part(part: &Value, native: &Value, part_id: Option<&str>) -> Vec<Value> {
    let Some(state) = part.get("state") else {
        return Vec::new();
    };
    match state.get("status").and_then(Value::as_str) {
        Some("pending") => Vec::new(),
        Some("running") => vec![tool_call(part, state, native.clone(), part_id)],
        Some("completed") => vec![tool_result(
            state_string(state, "output").unwrap_or_default(),
            native.clone(),
            part_id,
        )],
        Some("error") => vec![tool_result(
            state_string(state, "error").unwrap_or_else(|| "tool failed".to_string()),
            native.clone(),
            part_id,
        )],
        _ => Vec::new(),
    }
}

fn part_id(part: &Value) -> Option<&str> {
    part.get("id").and_then(Value::as_str)
}

fn state_string(state: &Value, key: &str) -> Option<String> {
    state
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| state.get(key).map(Value::to_string))
}

fn tool_call(part: &Value, state: &Value, native: Value, part_id: Option<&str>) -> Value {
    let mut event = json!({
        "type": "response_item",
        "payload": {
            "type": "custom_tool_call",
            "name": part.get("tool").and_then(Value::as_str).unwrap_or("tool"),
            "input": state.get("input").map(Value::to_string).unwrap_or_default(),
        },
        "backend": "opencode",
        "native": native,
    });
    add_part_id(&mut event, part_id);
    event
}

fn tool_result(output: String, native: Value, part_id: Option<&str>) -> Value {
    let mut event = json!({
        "type": "response_item",
        "payload": {
            "type": "custom_tool_call_output",
            "output": output,
        },
        "backend": "opencode",
        "native": native,
    });
    add_part_id(&mut event, part_id);
    event
}

fn assistant_text(v: &Value) -> String {
    v.get("parts")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| v.to_string())
}

fn agent_message(text: &str, native: Value) -> Value {
    agent_message_with_part_id(text, native, None)
}

fn agent_message_with_part_id(text: &str, native: Value, part_id: Option<&str>) -> Value {
    let mut event = json!({
        "type": "event_msg",
        "payload": { "type": "agent_message", "message": text },
        "backend": "opencode",
        "native": native,
    });
    add_part_id(&mut event, part_id);
    event
}

fn reasoning_part(text: &str, native: Value, part_id: Option<&str>) -> Value {
    let mut event = json!({
        "type": "response_item",
        "payload": { "type": "reasoning", "text": text },
        "backend": "opencode",
        "native": native,
    });
    add_part_id(&mut event, part_id);
    event
}

fn add_part_id(event: &mut Value, part_id: Option<&str>) {
    if let Some(part_id) = part_id {
        event["payload"]["partID"] = json!(part_id);
    }
}

fn task_complete(last: &str, error: Option<&str>) -> Value {
    json!({
        "type": "event_msg",
        "payload": {
            "type": "task_complete",
            "last_agent_message": error.unwrap_or(last),
        },
        "backend": "opencode",
    })
}

async fn append_line(path: &Path, line: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut f = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    f.write_all(line.as_bytes()).await?;
    f.write_all(b"\n").await
}

async fn read_capped_opt<R: AsyncRead + Unpin>(reader: Option<R>, cap: usize) -> String {
    let Some(mut r) = reader else {
        return String::new();
    };
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    while let Ok(n) = r.read(&mut chunk).await {
        if n == 0 {
            break;
        }
        if buf.len() < cap {
            let take = (cap - buf.len()).min(n);
            buf.extend_from_slice(&chunk[..take]);
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_provider_model() {
        assert_eq!(
            parse_provider_model(Some("anthropic/claude-sonnet-4")).unwrap(),
            Some(("anthropic", "claude-sonnet-4"))
        );
        assert!(parse_provider_model(Some("claude-sonnet-4")).is_err());
        assert_eq!(parse_provider_model(None).unwrap(), None);
    }

    #[test]
    fn normalizes_opencode_tool_and_text_events() {
        let running = tool_part_event("ses_1", "part-tool", "running", json!({"file": "a.txt"}));
        let text = json!({
            "type": "message.part.updated",
            "properties": {
                "sessionID": "ses_1",
                "part": {
                    "id": "part-text",
                    "sessionID": "ses_1",
                    "messageID": "msg_1",
                    "type": "text",
                    "text": "done"
                },
                "time": 1
            }
        });
        let other = json!({
            "type": "message.part.updated",
            "properties": {
                "sessionID": "ses_2",
                "part": {
                    "id": "part-other",
                    "sessionID": "ses_2",
                    "messageID": "msg_2",
                    "type": "text",
                    "text": "wrong"
                },
                "time": 1
            }
        });
        let delta = json!({
            "type": "message.part.delta",
            "properties": {
                "sessionID": "ses_1",
                "messageID": "msg_1",
                "partID": "part-text",
                "field": "text",
                "delta": "ignored"
            }
        });

        let tool = normalize_event("ses_1", &running);
        assert_eq!(tool.len(), 1);
        assert_eq!(tool[0]["payload"]["type"], "custom_tool_call");
        assert_eq!(tool[0]["payload"]["name"], "edit");
        assert_eq!(tool[0]["payload"]["input"], "{\"file\":\"a.txt\"}");
        assert_eq!(tool[0]["payload"]["partID"], "part-tool");

        let text = normalize_event("ses_1", &text);
        assert_eq!(text.len(), 1);
        assert_eq!(text[0]["payload"]["type"], "agent_message");
        assert_eq!(text[0]["payload"]["message"], "done");
        assert_eq!(text[0]["payload"]["partID"], "part-text");

        assert!(normalize_event("ses_1", &other).is_empty());
        assert!(normalize_event("ses_1", &delta).is_empty());
    }

    #[test]
    fn normalizes_opencode_tool_terminal_states() {
        let completed = tool_part_event(
            "ses_1",
            "part-completed",
            "completed",
            json!({"output": "ok"}),
        );
        let error = tool_part_event("ses_1", "part-error", "error", json!({"error": "failed"}));
        let pending = tool_part_event("ses_1", "part-pending", "pending", json!({}));

        let completed = normalize_event("ses_1", &completed);
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0]["payload"]["type"], "custom_tool_call_output");
        assert_eq!(completed[0]["payload"]["output"], "ok");
        assert_eq!(completed[0]["payload"]["partID"], "part-completed");

        let error = normalize_event("ses_1", &error);
        assert_eq!(error.len(), 1);
        assert_eq!(error[0]["payload"]["type"], "custom_tool_call_output");
        assert_eq!(error[0]["payload"]["output"], "failed");
        assert_eq!(error[0]["payload"]["partID"], "part-error");

        assert!(normalize_event("ses_1", &pending).is_empty());
    }

    #[test]
    fn normalizes_opencode_reasoning_text() {
        let reasoning = json!({
            "type": "message.part.updated",
            "properties": {
                "sessionID": "ses_1",
                "part": {
                    "id": "part-reasoning",
                    "sessionID": "ses_1",
                    "messageID": "msg_1",
                    "type": "reasoning",
                    "text": "checking assumptions"
                },
                "time": 1
            }
        });

        let events = normalize_event("ses_1", &reasoning);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["payload"]["type"], "reasoning");
        assert_eq!(events[0]["payload"]["text"], "checking assumptions");
        assert_eq!(events[0]["payload"]["partID"], "part-reasoning");
    }

    #[test]
    fn parses_crlf_sse_data_blocks() {
        let raw = "event: message\r\ndata: {\"type\":\"message.part.updated\",\"properties\":{\"sessionID\":\"ses_1\",\"part\":{\"id\":\"part-text\",\"sessionID\":\"ses_1\",\"messageID\":\"msg_1\",\"type\":\"text\",\"text\":\"done\"},\"time\":1}}\r\n\r\n";
        let (block, rest) = take_sse_block(raw).unwrap();
        assert!(rest.is_empty());
        let parsed = parse_sse_data(&block).unwrap();
        assert_eq!(parsed["type"], "message.part.updated");
        assert_eq!(parsed["properties"]["part"]["type"], "text");
        let events = normalize_event("ses_1", &parsed);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["payload"]["message"], "done");
    }

    #[test]
    fn permission_rules_are_api_rulesets() {
        let rules = permission_rules("read-only");
        let arr = rules.as_array().expect("ruleset array");
        assert!(
            arr.iter()
                .any(|r| r["permission"] == "edit" && r["action"] == "deny")
        );
    }

    fn tool_part_event(session_id: &str, part_id: &str, status: &str, extra: Value) -> Value {
        let mut state = json!({
            "status": status,
            "input": {"file": "a.txt"},
        });
        if let Some(output) = extra.get("output") {
            state["output"] = output.clone();
        }
        if let Some(error) = extra.get("error") {
            state["error"] = error.clone();
        }
        json!({
            "type": "message.part.updated",
            "properties": {
                "sessionID": session_id,
                "part": {
                    "id": part_id,
                    "sessionID": session_id,
                    "messageID": "msg_1",
                    "type": "tool",
                    "callID": "call_1",
                    "tool": "edit",
                    "state": state
                },
                "time": 1
            }
        })
    }
}
