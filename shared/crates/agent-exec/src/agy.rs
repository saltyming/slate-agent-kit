//! The agy backend's stream: the stdin event that carries the prompt, and the
//! result normaliser that reads agy's NDJSON output.
//!
//! agy runs with `--input-format stream-json --output-format stream-json`.
//! The prompt is one `user` event line ([`user_event_line`]). stdout is read
//! as it is drained ([`read_stream`]), independently of the capture: the
//! `init` event's `conversation_id` is reported at once, `step_update` text
//! deltas feed the retained capture under the run's stdout cap, the `result`
//! event is kept whole up to [`RESULT_BOUND`], and a non-JSON line naming
//! `AGY_ERROR` is remembered. After the exit, [`normalise`] decides the
//! outcome from the exit status and what the stream held.
//!
//! Recorded agy 1.3.4 shapes: `{"event":"init","conversation_id":<id>,
//! "init":{...}}` first; `{"event":"step_update","step_update":{...,
//! "text_delta":<text>}}` while the turn runs; `{"event":"result","result":
//! {"conversation_id","status","response","error"?,"denied_actions"?,
//! "usage":{"input_tokens","output_tokens","thinking_tokens",
//! "cache_read_tokens","total_tokens"},...}}` last. A tool headless agy could
//! not get permission for is listed in `denied_actions` with `status`
//! `SUCCESS` and exit 0; exit 3 follows a partial response with an
//! `AGY_ERROR` line.

use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::capture::{Cap, CapBuffer, Captured};
use harness_log::{Usage, UsageSource};

/// The largest event line kept whole, in bytes: a `result` event past it is
/// not read and the run fails saying so.
pub(crate) const RESULT_BOUND: usize = 8 * 1024 * 1024;

/// How many leading bytes of a line past the bound are kept to tell which
/// event it was.
const OVERSIZED_HEAD: usize = 256;

/// How many characters of an `AGY_ERROR` line are kept.
const AGY_ERROR_CHARS: usize = 2000;

/// The marker agy prints on the line that explains an exit 3.
const AGY_ERROR: &str = "AGY_ERROR";

/// The stdin line that carries `prompt`: `{"event":"user","message":
/// {"role":"user","content":<prompt as a JSON string>}}` and a newline.
pub(crate) fn user_event_line(prompt: &str) -> String {
    // A `str` always serialises; the fallback only keeps this total.
    let content = serde_json::to_string(prompt).unwrap_or_else(|_| "\"\"".to_string());
    format!("{{\"event\":\"user\",\"message\":{{\"role\":\"user\",\"content\":{content}}}}}\n")
}

/// What agy's stdout held, read to its end.
#[derive(Debug, Default)]
pub(crate) struct Stream {
    /// The `step_update` text deltas, kept under the stdout cap.
    pub deltas: Captured,
    /// The `init` event's `conversation_id`.
    pub conversation_id: Option<String>,
    /// The `result` event's `result` value, when one was read.
    pub result: Option<Value>,
    /// A `result` event longer than [`RESULT_BOUND`] arrived and was dropped.
    pub result_overflow: bool,
    /// The last non-JSON stdout line that names `AGY_ERROR`.
    pub agy_error: Option<String>,
}

/// Read agy's stdout to its end. `on_session` is called with the `init`
/// event's `conversation_id` as soon as it is read, once. Lines are split on
/// raw bytes before they are decoded, so an event split across reads, or a
/// character split across reads, is reassembled whole. A read error ends the
/// stream with what was read so far.
pub(crate) async fn read_stream<R, F>(reader: Option<R>, cap: Cap, on_session: F) -> Stream
where
    R: AsyncRead + Unpin,
    F: FnMut(&str),
{
    let mut st = Reader::new(cap, on_session);
    if let Some(mut r) = reader {
        let mut chunk = [0u8; 8192];
        loop {
            let n = match r.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            st.push(&chunk[..n]);
        }
    }
    st.finish()
}

/// The incremental state of [`read_stream`].
struct Reader<F> {
    deltas: CapBuffer,
    line: Vec<u8>,
    /// The line in progress passed the bound: its head is kept, the rest is
    /// dropped up to the next newline.
    oversized: bool,
    stream: Stream,
    on_session: F,
}

impl<F: FnMut(&str)> Reader<F> {
    fn new(cap: Cap, on_session: F) -> Self {
        Reader {
            deltas: CapBuffer::new(cap),
            line: Vec::new(),
            oversized: false,
            stream: Stream::default(),
            on_session,
        }
    }

    fn push(&mut self, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            let (part, rest, ended) = match bytes.iter().position(|b| *b == b'\n') {
                Some(i) => (&bytes[..i], &bytes[i + 1..], true),
                None => (bytes, &bytes[bytes.len()..], false),
            };
            if !self.oversized {
                if self.line.len() + part.len() > RESULT_BOUND {
                    let room = OVERSIZED_HEAD.saturating_sub(self.line.len());
                    self.line.extend_from_slice(&part[..room.min(part.len())]);
                    self.line.truncate(OVERSIZED_HEAD);
                    self.oversized = true;
                } else {
                    self.line.extend_from_slice(part);
                }
            }
            if ended {
                self.end_line();
            }
            bytes = rest;
        }
    }

    fn end_line(&mut self) {
        let line = std::mem::take(&mut self.line);
        if std::mem::take(&mut self.oversized) {
            // Only the head is left; it says which event was dropped.
            if String::from_utf8_lossy(&line).contains("\"event\":\"result\"") {
                self.stream.result_overflow = true;
            }
            return;
        }
        self.line_done(&line);
    }

    fn line_done(&mut self, line: &[u8]) {
        let trimmed = line.trim_ascii();
        if trimmed.is_empty() {
            return;
        }
        match serde_json::from_slice::<Value>(trimmed) {
            Ok(event) => self.event(event),
            Err(_) => {
                let text = String::from_utf8_lossy(trimmed);
                if text.contains(AGY_ERROR) {
                    self.stream.agy_error = Some(text.chars().take(AGY_ERROR_CHARS).collect());
                }
            }
        }
    }

    fn event(&mut self, event: Value) {
        match event.get("event").and_then(Value::as_str) {
            Some("init") => {
                if self.stream.conversation_id.is_none()
                    && let Some(id) = event
                        .get("conversation_id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                {
                    self.stream.conversation_id = Some(id.to_string());
                    (self.on_session)(id);
                }
            }
            Some("step_update") => {
                if let Some(delta) = event
                    .get("step_update")
                    .and_then(|s| s.get("text_delta"))
                    .and_then(Value::as_str)
                {
                    self.deltas.push(delta.as_bytes());
                }
            }
            Some("result") => {
                self.stream.result = event.get("result").cloned();
                self.stream.result_overflow = false;
            }
            // agy ignores an unknown event with a warning; so does this.
            _ => {}
        }
    }

    fn finish(mut self) -> Stream {
        // A last line without its newline still counts.
        if self.oversized || !self.line.is_empty() {
            self.end_line();
        }
        self.stream.deltas = self.deltas.finish();
        self.stream
    }
}

/// The outcome [`normalise`] decided.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Normalised {
    pub success: bool,
    /// The `response`, with a note naming the denied actions when there were
    /// any; `None` without a well-formed `result`.
    pub final_text: Option<String>,
    /// The result's token counts, mapped onto the usage buckets.
    pub counts: Option<Counts>,
    /// Why the run failed, in agy's own terms; `None` on success, and on a
    /// non-zero exit that came with a successful result (the exit code and
    /// stderr then say it).
    pub failure_detail: Option<String>,
    pub session_id: Option<String>,
    pub usage: Option<Usage>,
}

/// Decide the outcome of an agy run from its exit status (`exit_ok`: exited
/// with 0), its stdout stream and its retained stderr.
pub(crate) fn normalise(exit_ok: bool, s: &Stream, stderr: &str) -> Normalised {
    let result = s
        .result
        .as_ref()
        .filter(|r| r.get("status").and_then(Value::as_str).is_some());
    let session_id = s.conversation_id.clone().or_else(|| {
        s.result
            .as_ref()
            .and_then(|r| r.get("conversation_id"))
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
    });
    let Some(r) = result else {
        let detail = agy_error_line(s, stderr).unwrap_or_else(|| {
            if s.result_overflow {
                format!(
                    "agy's result event exceeded the {} MB bound and was not read",
                    RESULT_BOUND / (1024 * 1024)
                )
            } else if s.result.is_some() {
                "agy's result event was not well-formed (no status)".to_string()
            } else {
                "agy ended without a result event".to_string()
            }
        });
        return Normalised {
            success: false,
            final_text: None,
            counts: None,
            failure_detail: Some(detail),
            session_id,
            usage: None,
        };
    };

    let status = r.get("status").and_then(Value::as_str).unwrap_or_default();
    let response = r
        .get("response")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let error = r
        .get("error")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|e| !e.is_empty());
    let denied = denied_names(r);
    let denied_blocked = !denied.is_empty() && response.trim().is_empty();
    let success = exit_ok && status == "SUCCESS" && error.is_none() && !denied_blocked;

    let mut final_text = response.to_string();
    if !denied.is_empty() && !response.is_empty() {
        final_text.push_str(&format!("\n\n[headless agy denied: {}]", denied.join(", ")));
    }
    let failure_detail = if success {
        None
    } else if let Some(e) = error {
        Some(e.to_string())
    } else if let Some(line) = agy_error_line(s, stderr) {
        Some(line)
    } else if denied_blocked {
        Some(
            denied
                .iter()
                .map(|name| format!("{name} needed a permission headless agy cannot grant"))
                .collect::<Vec<_>>()
                .join("; "),
        )
    } else if status != "SUCCESS" {
        Some(format!("agy reported status {status}"))
    } else {
        None
    };
    Normalised {
        success,
        final_text: Some(final_text),
        counts: counts(r),
        failure_detail,
        session_id,
        usage: usage(r),
    }
}

/// The `display_name` (else the `action`) of each denied action, in order.
fn denied_names(result: &Value) -> Vec<String> {
    result
        .get("denied_actions")
        .and_then(Value::as_array)
        .map(|actions| {
            actions
                .iter()
                .map(|a| {
                    a.get("display_name")
                        .or_else(|| a.get("action"))
                        .and_then(Value::as_str)
                        .unwrap_or("an action")
                        .to_string()
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The `AGY_ERROR` line of the run: from stdout, else the last stderr line
/// that names it.
fn agy_error_line(s: &Stream, stderr: &str) -> Option<String> {
    s.agy_error.clone().or_else(|| {
        stderr
            .lines()
            .rev()
            .find(|l| l.contains(AGY_ERROR))
            .map(|l| l.trim().chars().take(AGY_ERROR_CHARS).collect())
    })
}

/// The token counts of the result's `usage`, mapped onto [`Usage`]'s
/// buckets: input = `input_tokens`, cache read = `cache_read_tokens` (a
/// bucket beside input: it exceeded `input_tokens` in one probe), output =
/// `output_tokens` (reasoning included: `thinking_tokens` never exceeded it,
/// and `total_tokens = input_tokens + output_tokens`), reasoning =
/// `thinking_tokens`. agy reports no cache write. A field absent from the
/// source leaves its bucket `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Counts {
    pub input: Option<u64>,
    pub cache_read: Option<u64>,
    pub output: Option<u64>,
    pub reasoning: Option<u64>,
}

/// The result's token counts, or `None` when it carries no `usage` object.
pub(crate) fn counts(result: &Value) -> Option<Counts> {
    let u = result.get("usage")?.as_object()?;
    let n = |key: &str| u.get(key).and_then(Value::as_u64);
    Some(Counts {
        input: n("input_tokens"),
        cache_read: n("cache_read_tokens"),
        output: n("output_tokens"),
        reasoning: n("thinking_tokens"),
    })
}

/// The record's usage: [`counts`] as a [`Usage`] read from
/// `UsageSource::AgyStreamJson`.
fn usage(result: &Value) -> Option<Usage> {
    let c = counts(result)?;
    Some(Usage {
        usage_source: UsageSource::AgyStreamJson,
        input_uncached: c.input,
        cache_read: c.cache_read,
        cache_write_5m: None,
        cache_write_1h: None,
        output: c.output,
        reasoning: c.reasoning,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use tokio::io::ReadBuf;

    use super::*;
    use crate::capture::{CapturePolicy, Keep, Unit};

    const ID: &str = "e195bb9c-5d1f-4c4e-9a0e-3f1d2b7c8a90";

    /// The recorded agy 1.3.4 json envelope of a one-word answer.
    fn envelope() -> String {
        format!(
            r#"{{"conversation_id":"{ID}","status":"SUCCESS","response":"ok\n","duration_seconds":11.25,"num_turns":1,"usage":{{"input_tokens":11936,"output_tokens":26,"thinking_tokens":25,"cache_read_tokens":0,"total_tokens":11962}}}}"#
        )
    }

    fn init() -> String {
        format!(
            r#"{{"event":"init","conversation_id":"{ID}","init":{{"cwd":"/w","tools":["view_file","grep_search","run_command","write_to_file"],"permission_mode":"request-review"}}}}"#
        )
    }

    fn step(index: u32, state: &str, kind: &str, delta: Option<&str>) -> String {
        let mut su = serde_json::json!({
            "conversation_id": ID, "step_index": index, "state": state, "step_type": kind,
        });
        if let Some(d) = delta {
            su["text_delta"] = Value::from(d);
        }
        serde_json::json!({"event": "step_update", "step_update": su}).to_string()
    }

    fn result_line(result: &str) -> String {
        format!(r#"{{"event":"result","result":{result}}}"#)
    }

    /// The recorded stream-json shape of a successful run.
    fn success_stream() -> String {
        [
            init(),
            step(0, "DONE", "user_input", None),
            step(1, "ACTIVE", "agent_response", Some("ok")),
            step(1, "DONE", "agent_response", Some("\n")),
            result_line(&envelope()),
        ]
        .join("\n")
            + "\n"
    }

    const JETSKI: &str = "jetski: no output produced — a tool required the \"command\" \
        permission that headless mode cannot prompt for, so it was auto-denied. Add an \
        allow-rule under permissions.allow in settings.json (e.g. command(<target>)). \
        Alternatively, re-run with --dangerously-skip-permissions to auto-approve all tools.";

    fn denial(response: &str) -> String {
        let result = serde_json::json!({
            "conversation_id": ID, "status": "SUCCESS", "response": response,
            "duration_seconds": 5.17, "num_turns": 1,
            "usage": {"input_tokens": 9000, "output_tokens": 12, "thinking_tokens": 0,
                      "cache_read_tokens": 0, "total_tokens": 9012},
            "denied_actions": [{"action": "command", "display_name": "RunCommand"}],
        });
        [init(), result_line(&result.to_string())].join("\n") + "\n"
    }

    /// A reader that returns `chunks` one per read, so a test controls where
    /// reads split the stream.
    struct Chunked(VecDeque<Vec<u8>>);

    impl AsyncRead for Chunked {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            if let Some(chunk) = self.0.pop_front() {
                buf.put_slice(&chunk);
            }
            Poll::Ready(Ok(()))
        }
    }

    fn chunked(bytes: &[u8], size: usize) -> Chunked {
        Chunked(bytes.chunks(size).map(<[u8]>::to_vec).collect())
    }

    async fn read_with(bytes: &[u8], size: usize, cap: Cap) -> (Stream, Vec<String>) {
        let mut sessions = Vec::new();
        let s = read_stream(Some(chunked(bytes, size)), cap, |id: &str| {
            sessions.push(id.to_string())
        })
        .await;
        (s, sessions)
    }

    async fn read(text: &str) -> Stream {
        read_with(text.as_bytes(), 8192, CapturePolicy::dispatch().stdout_cap)
            .await
            .0
    }

    #[test]
    fn the_stdin_line_escapes_quotes_newlines_and_keeps_non_ascii() {
        let prompt = "say \"hi\" \\ then\nnext line\ttab — ünïcödé 😀";
        let line = user_event_line(prompt);
        assert_eq!(
            line,
            "{\"event\":\"user\",\"message\":{\"role\":\"user\",\"content\":\
             \"say \\\"hi\\\" \\\\ then\\nnext line\\ttab — ünïcödé 😀\"}}\n"
        );
        // One line, ending in the only newline, that parses back to the prompt.
        assert_eq!(line.matches('\n').count(), 1);
        assert!(line.ends_with('\n'));
        let v: Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(v["event"], "user");
        assert_eq!(v["message"]["role"], "user");
        assert_eq!(v["message"]["content"], prompt);
        assert_eq!(
            crate::spec::InputEncoding::AgyStreamJson
                .encode(prompt)
                .as_ref(),
            line.as_bytes()
        );
        assert_eq!(
            crate::spec::InputEncoding::Verbatim.encode(prompt).as_ref(),
            prompt.as_bytes()
        );
    }

    #[tokio::test]
    async fn a_recorded_success_gives_response_session_deltas_and_usage() {
        let (s, sessions) = read_with(
            success_stream().as_bytes(),
            8192,
            CapturePolicy::dispatch().stdout_cap,
        )
        .await;
        assert_eq!(sessions, vec![ID.to_string()]);
        assert_eq!(s.deltas.text, "ok\n");
        assert_eq!(s.deltas.total, 3);
        assert!(!s.deltas.truncated);
        let n = normalise(true, &s, "");
        assert!(n.success);
        assert_eq!(n.final_text.as_deref(), Some("ok\n"));
        assert_eq!(n.failure_detail, None);
        assert_eq!(n.session_id.as_deref(), Some(ID));
        let c = n.counts.expect("counts");
        assert_eq!(
            c,
            Counts {
                input: Some(11936),
                cache_read: Some(0),
                output: Some(26),
                reasoning: Some(25),
            }
        );
        let u = n.usage.expect("usage");
        assert_eq!(u.usage_source, UsageSource::AgyStreamJson);
        assert_eq!(u.input_uncached, Some(11936));
        assert_eq!(u.cache_read, Some(0));
        assert_eq!(u.cache_write_5m, None);
        assert_eq!(u.cache_write_1h, None);
        assert_eq!(u.output, Some(26));
        assert_eq!(u.reasoning, Some(25));
        assert_eq!(
            serde_json::to_value(u.usage_source).unwrap(),
            Value::from("agy_stream_json")
        );
    }

    #[tokio::test]
    async fn usage_leaves_an_absent_field_unreported() {
        let s = read(&result_line(
            r#"{"status":"SUCCESS","response":"x","usage":{"input_tokens":5,"output_tokens":2}}"#,
        ))
        .await;
        let c = normalise(true, &s, "").counts.unwrap();
        assert_eq!(c.input, Some(5));
        assert_eq!(c.output, Some(2));
        assert_eq!(c.cache_read, None);
        assert_eq!(c.reasoning, None);
        let none = read(&result_line(r#"{"status":"SUCCESS","response":"x"}"#)).await;
        assert_eq!(normalise(true, &none, "").counts, None);
        assert_eq!(normalise(true, &none, "").usage, None);
    }

    #[tokio::test]
    async fn a_denial_with_an_empty_response_fails_with_exit_zero() {
        let s = read(&denial("")).await;
        let n = normalise(true, &s, JETSKI);
        assert!(!n.success);
        assert_eq!(
            n.failure_detail.as_deref(),
            Some("RunCommand needed a permission headless agy cannot grant")
        );
        assert_eq!(n.final_text.as_deref(), Some(""));
        assert_eq!(n.session_id.as_deref(), Some(ID));
        assert!(n.counts.is_some());
    }

    #[tokio::test]
    async fn a_denial_beside_a_response_succeeds_with_a_note() {
        let s = read(&denial("I could not run the tests.")).await;
        let n = normalise(true, &s, JETSKI);
        assert!(n.success);
        assert_eq!(n.failure_detail, None);
        assert_eq!(
            n.final_text.as_deref(),
            Some("I could not run the tests.\n\n[headless agy denied: RunCommand]")
        );
    }

    #[tokio::test]
    async fn status_error_fails_with_the_error_text() {
        // The recorded result of a malformed stdin message, with exit 0.
        let s = read(&result_line(&format!(
            r#"{{"conversation_id":"{ID}","status":"ERROR","response":"","error":"stream input \"user\" message is missing the \"message\" field","duration_seconds":0,"num_turns":0,"usage":{{"input_tokens":0,"output_tokens":0,"thinking_tokens":0,"cache_read_tokens":0,"total_tokens":0}}}}"#
        )))
        .await;
        let n = normalise(true, &s, "");
        assert!(!n.success);
        assert_eq!(
            n.failure_detail.as_deref(),
            Some("stream input \"user\" message is missing the \"message\" field")
        );
        // The conversation comes from the result when no init preceded it.
        assert_eq!(n.session_id.as_deref(), Some(ID));

        // An error beside status SUCCESS fails too; a bare non-SUCCESS status
        // says so.
        let s = read(&result_line(
            r#"{"status":"SUCCESS","response":"x","error":"boom"}"#,
        ))
        .await;
        assert_eq!(
            normalise(true, &s, "").failure_detail.as_deref(),
            Some("boom")
        );
        let s = read(&result_line(r#"{"status":"CANCELLED","response":"x"}"#)).await;
        let n = normalise(true, &s, "");
        assert!(!n.success);
        assert_eq!(
            n.failure_detail.as_deref(),
            Some("agy reported status CANCELLED")
        );
    }

    #[tokio::test]
    async fn exit_three_reports_the_agy_error_line() {
        let out = [
            init(),
            step(1, "ACTIVE", "agent_response", Some("partial ans")),
            "AGY_ERROR: the model stream ended unexpectedly".to_string(),
        ]
        .join("\n");
        let s = read(&out).await;
        let n = normalise(false, &s, "");
        assert!(!n.success);
        assert_eq!(
            n.failure_detail.as_deref(),
            Some("AGY_ERROR: the model stream ended unexpectedly")
        );
        assert_eq!(n.final_text, None);
        assert_eq!(s.deltas.text, "partial ans");

        // The line may come on stderr instead.
        let s = read(&init()).await;
        let n = normalise(
            false,
            &s,
            "warning: x\nAGY_ERROR: quota exhausted mid-turn\n",
        );
        assert_eq!(
            n.failure_detail.as_deref(),
            Some("AGY_ERROR: quota exhausted mid-turn")
        );
    }

    #[tokio::test]
    async fn exit_zero_without_a_result_event_fails() {
        let s = read(&[init(), step(1, "ACTIVE", "agent_response", Some("hm"))].join("\n")).await;
        let n = normalise(true, &s, "");
        assert!(!n.success);
        assert_eq!(
            n.failure_detail.as_deref(),
            Some("agy ended without a result event")
        );
        assert_eq!(n.session_id.as_deref(), Some(ID));

        let s = read(&result_line(r#"{"response":"no status"}"#)).await;
        let n = normalise(true, &s, "");
        assert!(!n.success);
        assert_eq!(
            n.failure_detail.as_deref(),
            Some("agy's result event was not well-formed (no status)")
        );
        // Nothing at all.
        let n = normalise(true, &read("").await, "");
        assert_eq!(
            n.failure_detail.as_deref(),
            Some("agy ended without a result event")
        );
    }

    #[tokio::test]
    async fn a_result_over_the_bound_fails_saying_so() {
        let big = "x".repeat(RESULT_BOUND + 10);
        let out = [
            init(),
            result_line(&format!(r#"{{"status":"SUCCESS","response":"{big}"}}"#)),
        ]
        .join("\n")
            + "\n";
        let s = read(&out).await;
        assert!(s.result_overflow);
        assert!(s.result.is_none());
        let n = normalise(true, &s, "");
        assert!(!n.success);
        assert_eq!(
            n.failure_detail.as_deref(),
            Some("agy's result event exceeded the 8 MB bound and was not read")
        );
        // An oversized line of another kind is dropped without that verdict,
        // and the stream goes on.
        let out = [
            step(1, "ACTIVE", "agent_response", Some(&big)),
            result_line(r#"{"status":"SUCCESS","response":"fine"}"#),
        ]
        .join("\n");
        let s = read(&out).await;
        assert!(!s.result_overflow);
        assert!(normalise(true, &s, "").success);
    }

    #[tokio::test]
    async fn events_split_across_reads_and_characters_are_reassembled() {
        let deltas = ["héllo ", "wörld 😀", "\n— done"];
        let mut lines = vec![init()];
        for d in deltas {
            lines.push(step(1, "ACTIVE", "agent_response", Some(d)));
        }
        lines.push(result_line(
            r#"{"status":"SUCCESS","response":"héllo wörld 😀\n— done"}"#,
        ));
        let out = lines.join("\n");
        let whole = deltas.concat();
        // Every split size, including one byte at a time, which cuts every
        // multi-byte character and every line.
        for size in [1, 2, 3, 5, 7, 64] {
            let (s, sessions) =
                read_with(out.as_bytes(), size, CapturePolicy::dispatch().stdout_cap).await;
            assert_eq!(sessions, vec![ID.to_string()], "size {size}");
            assert_eq!(s.deltas.text, whole, "size {size}");
            assert!(!s.deltas.text.contains('\u{FFFD}'));
            let n = normalise(true, &s, "");
            assert!(n.success, "size {size}");
            assert_eq!(n.final_text.as_deref(), Some(whole.as_str()));
        }
    }

    #[tokio::test]
    async fn output_beyond_the_capture_limit_still_reads_the_result() {
        let delta = "d".repeat(1000);
        let mut lines = vec![init()];
        for _ in 0..50 {
            lines.push(step(1, "ACTIVE", "agent_response", Some(&delta)));
        }
        let response = delta.repeat(50);
        lines.push(result_line(&format!(
            r#"{{"status":"SUCCESS","response":"{response}"}}"#
        )));
        let cap = Cap {
            limit: 4096,
            keep: Keep::Head,
            unit: Unit::Bytes,
        };
        let (s, _) = read_with(lines.join("\n").as_bytes(), 8192, cap).await;
        assert_eq!(s.deltas.text.len(), 4096);
        assert_eq!(s.deltas.total, 50_000);
        assert!(s.deltas.truncated);
        let n = normalise(true, &s, "");
        assert!(n.success);
        assert_eq!(n.final_text.as_deref(), Some(response.as_str()));
    }

    #[tokio::test]
    async fn the_session_is_reported_once_from_init_before_the_result() {
        let mut order = Vec::new();
        let out = [init(), init(), result_line(&envelope())].join("\n");
        let mut s = Reader::new(CapturePolicy::dispatch().stdout_cap, |id: &str| {
            order.push(format!("session {id}"))
        });
        // The session is known as soon as the init line ends, before any
        // other event is read.
        s.push(format!("{}\n", init()).as_bytes());
        assert_eq!(s.stream.conversation_id.as_deref(), Some(ID));
        assert!(s.stream.result.is_none());
        s.push(out.as_bytes());
        let st = s.finish();
        assert!(st.result.is_some());
        assert_eq!(order, vec![format!("session {ID}")]);
    }
}
