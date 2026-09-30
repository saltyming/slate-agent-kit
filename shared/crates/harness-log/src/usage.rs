//! Token usage of one backend run, normalized across backends.
//!
//! [`Usage`] holds five disjoint counts of a run — `input_uncached`,
//! `cache_read`, `cache_write_5m`, `cache_write_1h`, `output` — and one
//! overlapping count, `reasoning`, the part of `output` spent on reasoning or
//! thinking, which is never added to `output`. Reasoning is inside `output`
//! for every source.
//!
//! One parser per source, each returning `None` when the source holds no usage
//! record at all: [`codex_json_stream`] (a `codex exec --json` stdout),
//! [`codex_rollout`] (a codex rollout file from a baseline line),
//! [`claude_json_result`] (a `claude -p --output-format json` stdout) and
//! [`opencode_message`] (an opencode message). A field absent from the source
//! leaves its bucket `None`, a bucket computed from a `None` operand is
//! `None`, and a sum with a `None` term is `None`; no parser substitutes zero.
//! Pricing is not this module's business: the buckets carry counts only.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Where a [`Usage`] was read from. Serialized in `snake_case`
/// (`codex_json_stream`, `codex_rollout`, `claude_json_result`,
/// `opencode_message`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageSource {
    /// `turn.completed` events of a `codex exec --json` stream.
    CodexJsonStream,
    /// `token_count` events of a codex rollout file.
    CodexRollout,
    /// The `usage` object of a `claude -p --output-format json` result.
    ClaudeJsonResult,
    /// The `tokens` object of an opencode message.
    OpencodeMessage,
}

/// Token counts of one run. `None` means the source did not report the count
/// (or it was derived from one that was not reported); zero means the source
/// reported zero. Over several turns or iterations each bucket is the sum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Where the counts came from.
    pub usage_source: UsageSource,
    /// Input tokens neither read from nor written to a prompt cache.
    pub input_uncached: Option<u64>,
    /// Input tokens read from a prompt cache.
    pub cache_read: Option<u64>,
    /// Input tokens written to a prompt cache with a five-minute lifetime (or
    /// the only cache-write count a source reports).
    pub cache_write_5m: Option<u64>,
    /// Input tokens written to a prompt cache with a one-hour lifetime.
    pub cache_write_1h: Option<u64>,
    /// Output tokens, reasoning included.
    pub output: Option<u64>,
    /// The part of `output` spent on reasoning or thinking; never added to
    /// `output`.
    pub reasoning: Option<u64>,
}

impl Usage {
    /// A usage with every bucket `None`.
    fn empty(usage_source: UsageSource) -> Self {
        Usage {
            usage_source,
            input_uncached: None,
            cache_read: None,
            cache_write_5m: None,
            cache_write_1h: None,
            output: None,
            reasoning: None,
        }
    }

    /// Bucket-wise sum; a `None` term makes that bucket `None`.
    fn plus(self, other: Usage) -> Usage {
        Usage {
            usage_source: self.usage_source,
            input_uncached: add(self.input_uncached, other.input_uncached),
            cache_read: add(self.cache_read, other.cache_read),
            cache_write_5m: add(self.cache_write_5m, other.cache_write_5m),
            cache_write_1h: add(self.cache_write_1h, other.cache_write_1h),
            output: add(self.output, other.output),
            reasoning: add(self.reasoning, other.reasoning),
        }
    }
}

fn add(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    Some(a?.saturating_add(b?))
}

/// Sum `items` bucket-wise; `None` when there are none.
fn sum(items: impl IntoIterator<Item = Usage>) -> Option<Usage> {
    items.into_iter().reduce(Usage::plus)
}

/// A non-negative integer field of `o`, or `None` when absent or not one.
fn count(o: &Value, key: &str) -> Option<u64> {
    o.get(key)?.as_u64()
}

// ── codex ─────────────────────────────────────────────────

/// The raw counts codex reports for one request or turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CodexCounts {
    input: Option<u64>,
    cached: Option<u64>,
    cache_write: Option<u64>,
    output: Option<u64>,
    reasoning: Option<u64>,
}

impl CodexCounts {
    const ZERO: CodexCounts = CodexCounts {
        input: Some(0),
        cached: Some(0),
        cache_write: Some(0),
        output: Some(0),
        reasoning: Some(0),
    };

    fn from_value(o: &Value) -> CodexCounts {
        CodexCounts {
            input: count(o, "input_tokens"),
            cached: count(o, "cached_input_tokens"),
            cache_write: count(o, "cache_write_input_tokens"),
            output: count(o, "output_tokens"),
            reasoning: count(o, "reasoning_output_tokens"),
        }
    }

    /// Field-wise `self - earlier`, saturating at zero (a cumulative total
    /// never shrinks; a reset reads as no new tokens rather than a negative
    /// count).
    fn since(self, earlier: CodexCounts) -> CodexCounts {
        fn sub(a: Option<u64>, b: Option<u64>) -> Option<u64> {
            Some(a?.saturating_sub(b?))
        }
        CodexCounts {
            input: sub(self.input, earlier.input),
            cached: sub(self.cached, earlier.cached),
            cache_write: sub(self.cache_write, earlier.cache_write),
            output: sub(self.output, earlier.output),
            reasoning: sub(self.reasoning, earlier.reasoning),
        }
    }

    fn plus(self, other: CodexCounts) -> CodexCounts {
        CodexCounts {
            input: add(self.input, other.input),
            cached: add(self.cached, other.cached),
            cache_write: add(self.cache_write, other.cache_write),
            output: add(self.output, other.output),
            reasoning: add(self.reasoning, other.reasoning),
        }
    }

    /// Map to buckets: `uncached = max(0, input - cached)`,
    /// `write = max(0, min(cache_write, uncached))`,
    /// `input_uncached = uncached - write`.
    fn to_usage(self, usage_source: UsageSource) -> Usage {
        let uncached = match (self.input, self.cached) {
            (Some(i), Some(c)) => Some(i.saturating_sub(c)),
            _ => None,
        };
        let write = match (self.cache_write, uncached) {
            (Some(w), Some(u)) => Some(w.min(u)),
            _ => None,
        };
        let input_uncached = match (uncached, write) {
            (Some(u), Some(w)) => Some(u - w),
            _ => None,
        };
        Usage {
            usage_source,
            input_uncached,
            cache_read: self.cached,
            cache_write_5m: write,
            cache_write_1h: None,
            output: self.output,
            reasoning: self.reasoning,
        }
    }
}

/// Usage of a `codex exec --json` stdout: the sum over every `turn.completed`
/// event's `usage`. Lines that are not JSON are skipped. A `turn.completed`
/// without a `usage` object reports no counts, so every bucket of the sum is
/// `None`. `None` when the stream holds no `turn.completed` event.
pub fn codex_json_stream(text: &str) -> Option<Usage> {
    sum(text.lines().filter_map(|line| {
        let o: Value = serde_json::from_str(line.trim()).ok()?;
        if o.get("type")?.as_str()? != "turn.completed" {
            return None;
        }
        Some(match o.get("usage").filter(|u| u.is_object()) {
            Some(usage) => CodexCounts::from_value(usage).to_usage(UsageSource::CodexJsonStream),
            None => Usage::empty(UsageSource::CodexJsonStream),
        })
    }))
}

/// Usage of the turns a codex rollout gained after line `baseline` (the number
/// of lines the caller recorded before the run, so a resumed session counts
/// only its new turns; `0` for a fresh session).
///
/// Each `token_count` event contributes its `last_token_usage` when present,
/// else the change in `total_token_usage` since the previous event. The
/// cumulative baseline advances on every event, including those before
/// `baseline`, and a `token_count` line byte-identical to the previous one is
/// a re-emission and is skipped. `None` when the file cannot be read or holds
/// no counted event.
pub fn codex_rollout(path: &Path, baseline: usize) -> Option<Usage> {
    let text = std::fs::read_to_string(path).ok()?;
    codex_rollout_text(&text, baseline)
}

fn codex_rollout_text(text: &str, baseline: usize) -> Option<Usage> {
    let mut cumulative = CodexCounts::ZERO;
    let mut previous: Option<&str> = None;
    let mut counted: Vec<Usage> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        let Some(info) = token_count_info(line) else {
            continue;
        };
        if previous == Some(line) {
            continue;
        }
        previous = Some(line);
        let last = info
            .get("last_token_usage")
            .filter(|v| v.is_object())
            .map(CodexCounts::from_value);
        let total = info
            .get("total_token_usage")
            .filter(|v| v.is_object())
            .map(CodexCounts::from_value);
        let delta = match (last, total) {
            (Some(l), _) => Some(l),
            (None, Some(t)) => Some(t.since(cumulative)),
            (None, None) => None,
        };
        cumulative = match (total, delta) {
            (Some(t), _) => t,
            (None, Some(d)) => cumulative.plus(d),
            (None, None) => cumulative,
        };
        if index >= baseline
            && let Some(d) = delta
        {
            counted.push(d.to_usage(UsageSource::CodexRollout));
        }
    }
    sum(counted)
}

/// The `info` object of a rollout `token_count` event, if `line` is one that
/// carries it (`info` is `null` on events that report only rate limits).
fn token_count_info(line: &str) -> Option<Value> {
    if !line.contains("token_count") {
        return None;
    }
    let o: Value = serde_json::from_str(line).ok()?;
    if o.get("type")?.as_str()? != "event_msg" {
        return None;
    }
    let p = o.get("payload")?;
    if p.get("type")?.as_str()? != "token_count" {
        return None;
    }
    p.get("info").filter(|v| v.is_object()).cloned()
}

// ── claude ────────────────────────────────────────────────

/// Usage of a `claude -p --output-format json` stdout: the result's `usage`,
/// summed over `usage.iterations` when that array is present and non-empty.
/// The stdout is one result object; an array of events (the `--verbose` form)
/// is read from its last `result` element. `None` when no `usage` object is
/// found.
pub fn claude_json_result(text: &str) -> Option<Usage> {
    let v: Value = serde_json::from_str(text.trim()).ok()?;
    let result = match &v {
        Value::Array(items) => items
            .iter()
            .rev()
            .find(|i| i.get("type").and_then(Value::as_str) == Some("result"))?,
        other => other,
    };
    let usage = result.get("usage").filter(|u| u.is_object())?;
    match usage.get("iterations").and_then(Value::as_array) {
        Some(iterations) if !iterations.is_empty() => sum(iterations.iter().map(claude_counts)),
        _ => Some(claude_counts(usage)),
    }
}

fn claude_counts(u: &Value) -> Usage {
    let creation = u.get("cache_creation");
    let details = u.get("output_tokens_details");
    Usage {
        usage_source: UsageSource::ClaudeJsonResult,
        input_uncached: count(u, "input_tokens"),
        cache_read: count(u, "cache_read_input_tokens"),
        cache_write_5m: creation.and_then(|c| count(c, "ephemeral_5m_input_tokens")),
        cache_write_1h: creation.and_then(|c| count(c, "ephemeral_1h_input_tokens")),
        output: count(u, "output_tokens"),
        reasoning: details.and_then(|d| count(d, "thinking_tokens")),
    }
}

// ── opencode ──────────────────────────────────────────────

/// Usage of one opencode message (the message info object carrying `tokens`).
/// opencode reports `output` without reasoning, so `output = tokens.output +
/// tokens.reasoning`. `None` when the message has no `tokens` object.
pub fn opencode_message(value: &Value) -> Option<Usage> {
    let t = value.get("tokens").filter(|t| t.is_object())?;
    let cache = t.get("cache");
    let reasoning = count(t, "reasoning");
    Some(Usage {
        input_uncached: count(t, "input"),
        cache_read: cache.and_then(|c| count(c, "read")),
        cache_write_5m: cache.and_then(|c| count(c, "write")),
        output: add(count(t, "output"), reasoning),
        reasoning,
        ..Usage::empty(UsageSource::OpencodeMessage)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── codex json stream ──

    const CODEX_STREAM: &str = r#"{"type":"thread.started","thread_id":"t-1"}
{"type":"turn.started"}
{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"first"}}
{"type":"turn.completed","usage":{"input_tokens":1000,"cached_input_tokens":600,"cache_write_input_tokens":100,"output_tokens":50,"reasoning_output_tokens":20}}
not json at all
{"type":"turn.started"}
{"type":"turn.completed","usage":{"input_tokens":500,"cached_input_tokens":400,"cache_write_input_tokens":0,"output_tokens":30,"reasoning_output_tokens":10}}
"#;

    #[test]
    fn codex_stream_sums_turns() {
        let u = codex_json_stream(CODEX_STREAM).unwrap();
        assert_eq!(u.usage_source, UsageSource::CodexJsonStream);
        // turn 1: uncached 400, write 100 → input 300; turn 2: uncached 100, write 0
        assert_eq!(u.input_uncached, Some(300 + 100));
        assert_eq!(u.cache_read, Some(600 + 400));
        assert_eq!(u.cache_write_5m, Some(100));
        assert_eq!(u.cache_write_1h, None, "codex reports no 1h cache write");
        assert_eq!(u.output, Some(80));
        assert_eq!(u.reasoning, Some(30));
    }

    #[test]
    fn codex_stream_clamps_cache_write_to_uncached_input() {
        let text = r#"{"type":"turn.completed","usage":{"input_tokens":100,"cached_input_tokens":80,"cache_write_input_tokens":500,"output_tokens":1,"reasoning_output_tokens":0}}"#;
        let u = codex_json_stream(text).unwrap();
        assert_eq!(u.cache_write_5m, Some(20), "write clamped to uncached");
        assert_eq!(u.input_uncached, Some(0));

        // cached above input: uncached saturates at zero, so does write
        let text = r#"{"type":"turn.completed","usage":{"input_tokens":10,"cached_input_tokens":80,"cache_write_input_tokens":5,"output_tokens":1,"reasoning_output_tokens":0}}"#;
        let u = codex_json_stream(text).unwrap();
        assert_eq!(u.cache_write_5m, Some(0));
        assert_eq!(u.input_uncached, Some(0));
    }

    #[test]
    fn codex_stream_absent_fields_stay_none() {
        // no cache_write_input_tokens: write is None, and input_uncached is
        // derived from it, so it is None too; reasoning absent → None
        let text = r#"{"type":"turn.completed","usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":7}}"#;
        let u = codex_json_stream(text).unwrap();
        assert_eq!(u.cache_read, Some(20));
        assert_eq!(u.cache_write_5m, None);
        assert_eq!(u.input_uncached, None);
        assert_eq!(u.output, Some(7));
        assert_eq!(u.reasoning, None);

        // a None term in a sum makes the bucket None
        let two = format!(
            "{text}\n{}",
            r#"{"type":"turn.completed","usage":{"input_tokens":1,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":4}}"#
        );
        let u = codex_json_stream(&two).unwrap();
        assert_eq!(u.reasoning, None);
        assert_eq!(u.output, Some(8));
    }

    #[test]
    fn codex_stream_turn_without_usage_makes_every_bucket_none() {
        let text = format!(
            "{}\n{}",
            r#"{"type":"turn.completed","usage":{"input_tokens":1,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":0}}"#,
            r#"{"type":"turn.completed"}"#
        );
        let u = codex_json_stream(&text).unwrap();
        assert_eq!(u, Usage::empty(UsageSource::CodexJsonStream));
    }

    #[test]
    fn codex_stream_without_turn_completed_is_none() {
        assert_eq!(codex_json_stream(""), None);
        assert_eq!(
            codex_json_stream(r#"{"type":"thread.started","thread_id":"t"}"#),
            None
        );
    }

    // ── codex rollout ──

    fn token_count(last: Option<Value>, total: Option<Value>) -> String {
        let mut info = serde_json::Map::new();
        if let Some(l) = last {
            info.insert("last_token_usage".into(), l);
        }
        if let Some(t) = total {
            info.insert("total_token_usage".into(), t);
        }
        json!({"type":"event_msg","payload":{"type":"token_count","info":info}}).to_string()
    }

    fn counts(input: u64, cached: u64, write: u64, output: u64, reasoning: u64) -> Value {
        json!({"input_tokens":input,"cached_input_tokens":cached,
               "cache_write_input_tokens":write,"output_tokens":output,
               "reasoning_output_tokens":reasoning})
    }

    #[test]
    fn rollout_prefers_last_token_usage() {
        let lines = [
            json!({"type":"session_meta","payload":{"session_id":"s","cwd":"/w"}}).to_string(),
            token_count(
                Some(counts(100, 0, 0, 10, 2)),
                Some(counts(100, 0, 0, 10, 2)),
            ),
            token_count(
                Some(counts(50, 40, 0, 5, 1)),
                Some(counts(150, 40, 0, 15, 3)),
            ),
        ];
        let u = codex_rollout_text(&lines.join("\n"), 0).unwrap();
        assert_eq!(u.usage_source, UsageSource::CodexRollout);
        assert_eq!(u.input_uncached, Some(100 + 10));
        assert_eq!(u.cache_read, Some(40));
        assert_eq!(u.output, Some(15));
        assert_eq!(u.reasoning, Some(3));
    }

    #[test]
    fn rollout_baseline_counts_only_the_resumed_turns() {
        // A session that ran once (lines 0..=2), then was resumed: the caller
        // recorded 3 lines before the resumed run. Events carry only totals,
        // so the first new delta must be taken against the pre-baseline total.
        let lines = [
            json!({"type":"session_meta","payload":{"session_id":"s","cwd":"/w"}}).to_string(),
            token_count(None, Some(counts(100, 0, 0, 10, 0))),
            token_count(None, Some(counts(300, 100, 0, 30, 5))),
            // resumed run starts here (line index 3)
            json!({"type":"event_msg","payload":{"type":"user_message","message":"again"}})
                .to_string(),
            token_count(None, Some(counts(500, 250, 0, 45, 9))),
        ];
        let text = lines.join("\n");
        let u = codex_rollout_text(&text, 3).unwrap();
        // delta: input 200, cached 150 → uncached 50; output 15; reasoning 4
        assert_eq!(u.input_uncached, Some(50));
        assert_eq!(u.cache_read, Some(150));
        assert_eq!(u.output, Some(15));
        assert_eq!(u.reasoning, Some(4));

        // from the start, every event counts
        let all = codex_rollout_text(&text, 0).unwrap();
        assert_eq!(all.output, Some(45));
        assert_eq!(all.cache_read, Some(250));

        // nothing after the baseline → None
        assert_eq!(codex_rollout_text(&text, lines.len()), None);
    }

    #[test]
    fn rollout_skips_a_byte_identical_repeated_event() {
        let ev = token_count(
            Some(counts(100, 0, 0, 10, 1)),
            Some(counts(100, 0, 0, 10, 1)),
        );
        let text = [ev.clone(), ev.clone(), ev].join("\n");
        let u = codex_rollout_text(&text, 0).unwrap();
        assert_eq!(u.output, Some(10), "re-emissions are not counted");
        assert_eq!(u.input_uncached, Some(100));
    }

    #[test]
    fn rollout_ignores_rate_limit_only_events_and_unreadable_files() {
        let text = json!({"type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{}}})
            .to_string();
        assert_eq!(codex_rollout_text(&text, 0), None);
        let missing = std::env::temp_dir().join(format!(
            "harness-log-usage-missing-{}.jsonl",
            std::process::id()
        ));
        assert_eq!(codex_rollout(&missing, 0), None);
    }

    #[test]
    fn rollout_reads_a_file() {
        let path = std::env::temp_dir().join(format!(
            "harness-log-usage-rollout-{}.jsonl",
            std::process::id()
        ));
        std::fs::write(
            &path,
            token_count(Some(counts(10, 5, 1, 3, 1)), None) + "\n",
        )
        .unwrap();
        let u = codex_rollout(&path, 0).unwrap();
        assert_eq!(u.input_uncached, Some(4));
        assert_eq!(u.cache_write_5m, Some(1));
        let _ = std::fs::remove_file(&path);
    }

    // ── claude ──

    const CLAUDE_RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,
"result":"answer","session_id":"s-1",
"usage":{"input_tokens":12,"cache_read_input_tokens":3000,"cache_creation_input_tokens":700,
"cache_creation":{"ephemeral_5m_input_tokens":500,"ephemeral_1h_input_tokens":200},
"output_tokens":90,"output_tokens_details":{"thinking_tokens":40}}}"#;

    #[test]
    fn claude_result_maps_every_bucket() {
        let u = claude_json_result(CLAUDE_RESULT).unwrap();
        assert_eq!(u.usage_source, UsageSource::ClaudeJsonResult);
        assert_eq!(u.input_uncached, Some(12));
        assert_eq!(u.cache_read, Some(3000));
        assert_eq!(u.cache_write_5m, Some(500));
        assert_eq!(u.cache_write_1h, Some(200));
        assert_eq!(u.output, Some(90));
        assert_eq!(u.reasoning, Some(40));
    }

    #[test]
    fn claude_result_sums_iterations() {
        let text = r#"{"type":"result","usage":{"input_tokens":999,"output_tokens":999,
"iterations":[
 {"input_tokens":10,"cache_read_input_tokens":100,"cache_creation":{"ephemeral_5m_input_tokens":1,"ephemeral_1h_input_tokens":0},"output_tokens":5,"output_tokens_details":{"thinking_tokens":2}},
 {"input_tokens":20,"cache_read_input_tokens":200,"cache_creation":{"ephemeral_5m_input_tokens":3,"ephemeral_1h_input_tokens":4},"output_tokens":6,"output_tokens_details":{"thinking_tokens":1}}
]}}"#;
        let u = claude_json_result(text).unwrap();
        assert_eq!(u.input_uncached, Some(30));
        assert_eq!(u.cache_read, Some(300));
        assert_eq!(u.cache_write_5m, Some(4));
        assert_eq!(u.cache_write_1h, Some(4));
        assert_eq!(u.output, Some(11));
        assert_eq!(u.reasoning, Some(3));
    }

    #[test]
    fn claude_result_absent_fields_stay_none() {
        let u =
            claude_json_result(r#"{"type":"result","usage":{"input_tokens":5,"output_tokens":0}}"#)
                .unwrap();
        assert_eq!(u.input_uncached, Some(5));
        assert_eq!(u.output, Some(0), "a reported zero stays zero");
        assert_eq!(u.cache_read, None);
        assert_eq!(u.cache_write_5m, None);
        assert_eq!(u.cache_write_1h, None);
        assert_eq!(u.reasoning, None);

        assert_eq!(
            claude_json_result(r#"{"type":"result","result":"x"}"#),
            None
        );
        assert_eq!(claude_json_result("plain text answer"), None);
    }

    #[test]
    fn claude_result_reads_the_last_result_of_an_event_array() {
        let text = format!(r#"[{{"type":"system"}},{CLAUDE_RESULT}]"#);
        assert_eq!(claude_json_result(&text).unwrap().output, Some(90));
    }

    // ── opencode ──

    #[test]
    fn opencode_output_adds_reasoning() {
        let m = json!({"id":"msg_1","role":"assistant",
            "tokens":{"input":120,"output":30,"reasoning":70,"cache":{"read":400,"write":60}}});
        let u = opencode_message(&m).unwrap();
        assert_eq!(u.usage_source, UsageSource::OpencodeMessage);
        assert_eq!(u.input_uncached, Some(120));
        assert_eq!(u.cache_read, Some(400));
        assert_eq!(u.cache_write_5m, Some(60));
        assert_eq!(u.cache_write_1h, None);
        assert_eq!(
            u.output,
            Some(100),
            "output = tokens.output + tokens.reasoning"
        );
        assert_eq!(u.reasoning, Some(70));
    }

    #[test]
    fn opencode_absent_fields_stay_none() {
        let u = opencode_message(&json!({"tokens":{"input":1,"output":2}})).unwrap();
        assert_eq!(u.reasoning, None);
        assert_eq!(u.output, None, "a sum with an absent term is None");
        assert_eq!(u.cache_read, None);
        assert_eq!(opencode_message(&json!({"role":"assistant"})), None);
    }

    #[test]
    fn usage_round_trips_through_serde() {
        let u = claude_json_result(CLAUDE_RESULT).unwrap();
        let s = serde_json::to_string(&u).unwrap();
        assert!(s.contains(r#""usage_source":"claude_json_result""#), "{s}");
        let back: Usage = serde_json::from_str(&s).unwrap();
        assert_eq!(back, u);
        // an older record without a bucket still reads, as None
        let old: Usage =
            serde_json::from_str(r#"{"usage_source":"codex_rollout","output":3}"#).unwrap();
        assert_eq!(old.output, Some(3));
        assert_eq!(old.reasoning, None);
    }
}
