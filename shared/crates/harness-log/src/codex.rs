//! Codex rollout discovery.
//!
//! Entry points: [`codex_home`] and [`collect`] list rollouts;
//! [`read_session_meta`] identifies one; [`newest_interactive_rollout`] finds
//! the user's own session for a directory; [`locate_by_session_id`] and
//! [`locate_by_marker`] find the rollout of a known or marked headless run;
//! [`message_text`] reads a conversational message in either schema.
//!
//! Codex writes one JSONL "rollout" per session at
//! `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<ts>-<session-uuid>.jsonl`,
//! appended live while the session runs. The first line is a `session_meta`
//! event whose payload identifies the session: its `cwd`, its id
//! (`session_id`, or `id` in older logs), and — in newer logs — the
//! `originator`/`source` pair that distinguishes an interactive session
//! (`codex-tui`/`cli`, `Codex Desktop`/`vscode`) from a headless child run
//! (`codex_exec`/`exec`, which is what `aside` consultations and `dispatch`
//! delegations spawn).
//!
//! Conversational messages have been recorded in two schemas over codex's
//! history. Every rollout observed so far (several hundred, codex 0.142–0.153)
//! carries exactly one of them, so a reader that accepts both renders each
//! message once; a rollout that mixed them would show a message twice — a
//! cosmetic duplicate, never a lost message or a mis-association:
//!
//! * legacy (codex-cli ≤ 0.147): `{"type":"event_msg","payload":{"type":
//!   "user_message"|"agent_message","message":"…"}}`;
//! * current (introduced during 0.147, universal from 0.153): `{"type":
//!   "event_msg","payload":{"type":"item_completed","item":{"type":
//!   "UserMessage"|"AgentMessage","content":[{"type":"text"|"Text","text":"…"}]}}}`.
//!
//! `message_text` reads either, so every consumer (dispatch's log curation and
//! nonce association, aside's transcript reader) stays correct across the
//! boundary. The `response_item/message` records present in both schemas are
//! not messages in this sense: their `role:"user"` entries mix harness-injected
//! context (plugin lists, environment notes) with the real prompt.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

/// How many newest cwd-candidate rollouts `newest_interactive_rollout` will
/// open before giving up. Headless children (aside/dispatch runs) can heavily
/// outnumber interactive sessions, so this is much larger than the fresh-run
/// scan caps used by dispatch.
const INTERACTIVE_SCAN_CAP: usize = 500;

/// How many newest rollouts `locate_by_marker` opens before giving up: a
/// marked run is looked up right after its own spawn, so it is among the
/// newest files.
const MARKER_SCAN_CAP: usize = 60;

/// How many opening lines of a rollout `rollout_has_marker` scans — the prompt
/// is recorded as a user message among the first events (observed at lines
/// 4–10 across codex 0.142–0.153; harness-injected context precedes it as a
/// handful of single-line records).
const MARKER_SCAN_LINES: usize = 64;

/// Identity fields from a rollout's opening `session_meta` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMeta {
    pub cwd: String,
    pub session_id: String,
    pub originator: Option<String>,
    pub source: Option<String>,
}

/// `$CODEX_HOME`, defaulting to `~/.codex`.
pub fn codex_home() -> PathBuf {
    if let Some(h) = std::env::var_os("CODEX_HOME") {
        return PathBuf::from(h);
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    PathBuf::from(home).join(".codex")
}

/// Recursively gather `rollout-*.jsonl` files (with mtimes) under `dir`.
pub fn collect(dir: &Path, out: &mut Vec<(PathBuf, SystemTime)>, depth: usize) {
    if depth > 5 {
        return;
    }
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return,
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out, depth + 1);
        } else if let Some(name) = p.file_name().and_then(|s| s.to_str())
            && name.starts_with("rollout-")
            && name.ends_with(".jsonl")
        {
            let mtime = e
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            out.push((p, mtime));
        }
    }
}

/// Parse the opening `session_meta` line of a rollout.
pub fn read_session_meta(path: &Path) -> Option<SessionMeta> {
    use std::io::{BufRead, BufReader};
    let f = std::fs::File::open(path).ok()?;
    let mut first = String::new();
    BufReader::new(f).read_line(&mut first).ok()?;
    let o: Value = serde_json::from_str(first.trim()).ok()?;
    if o.get("type")?.as_str()? != "session_meta" {
        return None;
    }
    let p = o.get("payload")?;
    let cwd = p.get("cwd")?.as_str()?.to_string();
    // session_id is the canonical field; older codex logs carried only `id`.
    let session_id = p
        .get("session_id")
        .or_else(|| p.get("id"))
        .and_then(|v| v.as_str())?
        .to_string();
    let originator = p
        .get("originator")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let source = p.get("source").and_then(|v| v.as_str()).map(str::to_string);
    Some(SessionMeta {
        cwd,
        session_id,
        originator,
        source,
    })
}

/// Whether this session is a headless child run (`codex exec`) rather than an
/// interactive session. Missing fields (older logs) count as interactive —
/// fail-open is harmless for transcript discovery.
pub fn is_exec_child(m: &SessionMeta) -> bool {
    m.source.as_deref() == Some("exec") || m.originator.as_deref() == Some("codex_exec")
}

/// Who authored a conversational rollout message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Agent,
}

/// The conversational text a rollout event records, if it is a user or agent
/// message in either schema (see the module doc). Returns the text unmodified —
/// possibly empty — so callers decide how to treat blank messages.
pub fn message_text(o: &Value) -> Option<(MessageRole, String)> {
    if o.get("type")?.as_str()? != "event_msg" {
        return None;
    }
    let p = o.get("payload")?;
    match p.get("type")?.as_str()? {
        "user_message" => Some((MessageRole::User, p.get("message")?.as_str()?.to_string())),
        "agent_message" => Some((MessageRole::Agent, p.get("message")?.as_str()?.to_string())),
        "item_completed" => {
            let item = p.get("item")?;
            let role = match item.get("type")?.as_str()? {
                "UserMessage" => MessageRole::User,
                "AgentMessage" => MessageRole::Agent,
                _ => return None,
            };
            Some((role, item_text(item)))
        }
        _ => None,
    }
}

/// Concatenate the text blocks of an `item_completed` message item. The block
/// type is `text` on `UserMessage` items and `Text` on `AgentMessage` items, so
/// it is matched case-insensitively.
fn item_text(item: &Value) -> String {
    item.get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| {
                    b.get("type")
                        .and_then(Value::as_str)
                        .is_some_and(|t| t.eq_ignore_ascii_case("text"))
                })
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Whether `cwd` (from a rollout's session_meta) refers to the same directory
/// as `want`, comparing the raw strings first and then canonicalized paths.
pub fn cwd_matches(cwd: &str, want: &str, want_canon: Option<&Path>) -> bool {
    if cwd == want {
        return true;
    }
    if let Some(wc) = want_canon
        && Path::new(cwd).canonicalize().ok().as_deref() == Some(wc)
    {
        return true;
    }
    false
}

pub fn file_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// The newest **interactive** rollout under `root` whose session cwd matches
/// `cwd` — headless `codex exec` children (aside consultations, dispatch
/// delegations) are excluded so a transcript reader never mistakes its own
/// prior child run for the user's conversation.
pub fn newest_interactive_rollout(root: &Path, cwd: &Path) -> Option<PathBuf> {
    let mut files = Vec::new();
    collect(root, &mut files, 0);
    files.sort_by_key(|f| std::cmp::Reverse(f.1)); // newest first
    let want = cwd.to_string_lossy().to_string();
    let want_canon = cwd.canonicalize().ok();
    for (path, _) in files.into_iter().take(INTERACTIVE_SCAN_CAP) {
        if let Some(meta) = read_session_meta(&path)
            && cwd_matches(&meta.cwd, &want, want_canon.as_deref())
            && !is_exec_child(&meta)
        {
            return Some(path);
        }
    }
    None
}

/// Locate a rollout by its codex session id. The session id is the trailing
/// UUID of the rollout filename (`rollout-<ts>-<sid>.jsonl`), so this is a
/// cheap, exact filename match with no scan cap — used to resume a session
/// whose id is already known and as a deterministic re-locate.
pub fn locate_by_session_id(sid: &str) -> Option<PathBuf> {
    locate_by_session_id_in(&codex_home().join("sessions"), sid)
}

fn locate_by_session_id_in(root: &Path, sid: &str) -> Option<PathBuf> {
    if sid.is_empty() {
        return None;
    }
    let mut files = Vec::new();
    collect(root, &mut files, 0);
    let suffix = format!("-{sid}.jsonl");
    files.into_iter().map(|(p, _)| p).find(|p| {
        p.file_name()
            .and_then(|s| s.to_str())
            .map(|n| n.ends_with(&suffix))
            .unwrap_or(false)
    })
}

/// Find the rollout a headless run produced by the marker its caller embedded
/// in the prompt: the newest rollout matching `working_dir` whose opening
/// events include a user message containing `marker`. Positive identity —
/// survives a concurrent same-cwd codex run that a snapshot or time gate alone
/// could not distinguish. Returns the path and the session id.
pub fn locate_by_marker(working_dir: &Path, marker: &str) -> Option<(PathBuf, String)> {
    locate_by_marker_in(&codex_home().join("sessions"), working_dir, marker)
}

fn locate_by_marker_in(root: &Path, working_dir: &Path, marker: &str) -> Option<(PathBuf, String)> {
    if marker.is_empty() {
        return None;
    }
    let mut files = Vec::new();
    collect(root, &mut files, 0);
    files.sort_by_key(|f| std::cmp::Reverse(f.1)); // newest first
    let want = working_dir.to_string_lossy().to_string();
    let want_canon = working_dir.canonicalize().ok();
    for (path, _) in files.into_iter().take(MARKER_SCAN_CAP) {
        if let Some(meta) = read_session_meta(&path)
            && cwd_matches(&meta.cwd, &want, want_canon.as_deref())
            && rollout_has_marker(&path, marker)
        {
            return Some((path, meta.session_id));
        }
    }
    None
}

/// Scan the opening lines of a rollout for `marker` inside a user message —
/// either schema (legacy `user_message` event or `item_completed`
/// `UserMessage` item). Reads at most `MARKER_SCAN_LINES` lines: the prompt is
/// recorded among the first events. `marker` is the caller's full marker
/// text, delimiters included, so a successor run whose marker extends this one
/// (`<base>-retry1`) is not claimed for `<base>`.
pub fn rollout_has_marker(path: &Path, marker: &str) -> bool {
    use std::io::{BufRead, BufReader};
    if marker.is_empty() {
        return false;
    }
    let f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    for line in BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .take(MARKER_SCAN_LINES)
    {
        let o: Value = match serde_json::from_str(line.trim()) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some((MessageRole::User, text)) = message_text(&o)
            && text.contains(marker)
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_root(tag: &str) -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("harness-log-test-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    fn write_rollout(dir: &Path, sid: &str, meta_extra: Value, ts: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(format!("rollout-{ts}-{sid}.jsonl"));
        let mut payload = json!({"session_id": sid, "cwd": "/w"});
        if let (Some(obj), Some(extra)) = (payload.as_object_mut(), meta_extra.as_object()) {
            for (k, v) in extra {
                obj.insert(k.clone(), v.clone());
            }
        }
        let meta = json!({"type":"session_meta","payload": payload});
        std::fs::write(&path, format!("{meta}\n")).unwrap();
        path
    }

    #[test]
    fn session_meta_reads_identity_fields_and_legacy_id() {
        let root = test_root("meta");
        let day = root.join("2026/07/03");
        let p = write_rollout(
            &day,
            "sid-1",
            json!({"originator":"codex-tui","source":"cli"}),
            "2026-07-03T00-00-01",
        );
        let m = read_session_meta(&p).unwrap();
        assert_eq!(m.cwd, "/w");
        assert_eq!(m.session_id, "sid-1");
        assert_eq!(m.originator.as_deref(), Some("codex-tui"));
        assert_eq!(m.source.as_deref(), Some("cli"));
        assert!(!is_exec_child(&m));

        // legacy log: only `id`, no originator/source → interactive (fail-open)
        std::fs::create_dir_all(&day).unwrap();
        let legacy = day.join("rollout-2026-07-03T00-00-02-legacy.jsonl");
        std::fs::write(
            &legacy,
            format!(
                "{}\n",
                json!({"type":"session_meta","payload":{"id":"legacy-id","cwd":"/w"}})
            ),
        )
        .unwrap();
        let m2 = read_session_meta(&legacy).unwrap();
        assert_eq!(m2.session_id, "legacy-id");
        assert_eq!(m2.originator, None);
        assert!(!is_exec_child(&m2));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn exec_children_are_detected_by_source_or_originator() {
        let by_source = SessionMeta {
            cwd: "/w".into(),
            session_id: "a".into(),
            originator: None,
            source: Some("exec".into()),
        };
        let by_originator = SessionMeta {
            cwd: "/w".into(),
            session_id: "b".into(),
            originator: Some("codex_exec".into()),
            source: None,
        };
        assert!(is_exec_child(&by_source));
        assert!(is_exec_child(&by_originator));
    }

    #[test]
    fn newest_interactive_skips_exec_children() {
        let root = test_root("interactive");
        let day = root.join("2026/07/03");
        let interactive = write_rollout(
            &day,
            "old-tui",
            json!({"originator":"codex-tui","source":"cli"}),
            "2026-07-03T00-00-01",
        );
        // newer exec child (what aside/dispatch spawn) must NOT win
        let exec_child = write_rollout(
            &day,
            "new-exec",
            json!({"originator":"codex_exec","source":"exec"}),
            "2026-07-03T00-00-02",
        );
        // make the exec child strictly newer by mtime
        let newer = SystemTime::now();
        let f = std::fs::File::options()
            .append(true)
            .open(&exec_child)
            .unwrap();
        f.set_modified(newer).unwrap();
        let older = newer - std::time::Duration::from_secs(60);
        let f2 = std::fs::File::options()
            .append(true)
            .open(&interactive)
            .unwrap();
        f2.set_modified(older).unwrap();

        assert_eq!(
            newest_interactive_rollout(&root, Path::new("/w")),
            Some(interactive)
        );
        // different cwd → nothing
        assert_eq!(newest_interactive_rollout(&root, Path::new("/x")), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn message_text_reads_both_schemas_and_ignores_response_items() {
        let legacy_user: Value = serde_json::from_str(
            r#"{"type":"event_msg","payload":{"type":"user_message","message":"fix it"}}"#,
        )
        .unwrap();
        assert_eq!(
            message_text(&legacy_user),
            Some((MessageRole::User, "fix it".to_string()))
        );
        let legacy_agent: Value = serde_json::from_str(
            r#"{"type":"event_msg","payload":{"type":"agent_message","message":"done"}}"#,
        )
        .unwrap();
        assert_eq!(
            message_text(&legacy_agent),
            Some((MessageRole::Agent, "done".to_string()))
        );

        // 0.153 item schema: `text` blocks on UserMessage, `Text` on AgentMessage.
        let item_user: Value = serde_json::from_str(
            r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","id":"u","content":[{"type":"text","text":"fix it"},{"type":"image","url":"x"}]}}}"#,
        )
        .unwrap();
        assert_eq!(
            message_text(&item_user),
            Some((MessageRole::User, "fix it".to_string()))
        );
        let item_agent: Value = serde_json::from_str(
            r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"AgentMessage","id":"a","content":[{"type":"Text","text":"part 1"},{"type":"Text","text":"part 2"}],"phase":"final_answer"}}}"#,
        )
        .unwrap();
        assert_eq!(
            message_text(&item_agent),
            Some((MessageRole::Agent, "part 1\npart 2".to_string()))
        );

        // Not messages: other completed items, and the response_item mirror
        // (whose role=user entries include injected context).
        for raw in [
            r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"CommandExecution","command":["ls"]}}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"fix it"}]}}"#,
            r#"{"type":"event_msg","payload":{"type":"task_started"}}"#,
            r#"{"type":"session_meta","payload":{"cwd":"/w"}}"#,
        ] {
            let o: Value = serde_json::from_str(raw).unwrap();
            assert_eq!(message_text(&o), None, "{raw}");
        }
    }

    fn write_marked(dir: &Path, sid: &str, cwd: &str, user_msg: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(format!("rollout-2026-06-27T00-00-00-{sid}.jsonl"));
        let meta = json!({"type":"session_meta","payload":{"session_id":sid,"cwd":cwd}});
        let um = json!({"type":"event_msg","payload":{"type":"user_message","message":user_msg}});
        std::fs::write(&path, format!("{meta}\n{um}\n")).unwrap();
        path
    }

    /// A codex >= 0.153 rollout: the prompt appears only as a `response_item`
    /// mirror and an `item_completed` `UserMessage` item — no `user_message`.
    fn write_marked_v2(dir: &Path, sid: &str, cwd: &str, user_msg: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(format!("rollout-2026-09-05T00-00-00-{sid}.jsonl"));
        let meta = json!({"type":"session_meta","payload":{"session_id":sid,"cwd":cwd}});
        let injected = json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<plugins>x</plugins>"}]}});
        let mirror = json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":user_msg}]}});
        let item = json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","id":"u1","content":[{"type":"text","text":user_msg}]}}});
        std::fs::write(&path, format!("{meta}\n{injected}\n{mirror}\n{item}\n")).unwrap();
        path
    }

    #[test]
    fn locate_by_session_id_matches_filename_suffix() {
        let root = test_root("by-sid");
        let day = root.join("2026/06/27");
        write_marked(&day, "aaa-111", "/w", "x");
        let target = write_marked(&day, "bbb-222", "/w", "y");
        assert_eq!(locate_by_session_id_in(&root, "bbb-222"), Some(target));
        assert_eq!(locate_by_session_id_in(&root, "no-such-sid"), None);
        assert_eq!(locate_by_session_id_in(&root, ""), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn locate_by_marker_picks_the_marked_rollout_in_both_schemas() {
        let root = test_root("by-marker");
        let day = root.join("2026/06/27");
        // identical cwd on every file — only the marker disambiguates
        write_marked(&day, "aaa", "/w", "an unrelated codex run");
        let marked = write_marked(&day, "bbb", "/w", "do it [task: d-7:N42]");
        assert_eq!(
            locate_by_marker_in(&root, Path::new("/w"), "[task: d-7:N42]"),
            Some((marked, "bbb".to_string()))
        );
        let marked_v2 = write_marked_v2(&day, "ccc", "/w", "do it [task: d-9:N9]");
        assert_eq!(
            locate_by_marker_in(&root, Path::new("/w"), "[task: d-9:N9]"),
            Some((marked_v2, "ccc".to_string()))
        );
        assert!(locate_by_marker_in(&root, Path::new("/w"), "[task: absent]").is_none());
        assert!(locate_by_marker_in(&root, Path::new("/x"), "[task: d-7:N42]").is_none());
        assert!(locate_by_marker_in(&root, Path::new("/w"), "").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rollout_has_marker_requires_the_full_marker() {
        let root = test_root("marker-exact");
        let day = root.join("2026/06/27");
        let successor = write_marked(&day, "ccc", "/w", "do it [task: d-3:N3-restart]");
        assert!(rollout_has_marker(&successor, "[task: d-3:N3-restart]"));
        assert!(!rollout_has_marker(&successor, "[task: d-3:N3]"));
        assert!(!rollout_has_marker(&successor, ""));
        // the marker only in the response_item mirror of a v2 rollout is not
        // enough on its own; here the item carries a different text
        let bare = write_marked_v2(&day, "ddd", "/w", "mentions d-3:N3 in passing");
        assert!(!rollout_has_marker(&bare, "[task: d-3:N3]"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
