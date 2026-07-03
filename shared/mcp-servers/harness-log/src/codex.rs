//! Codex rollout discovery.
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

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

/// How many newest cwd-candidate rollouts `newest_interactive_rollout` will
/// open before giving up. Headless children (aside/dispatch runs) can heavily
/// outnumber interactive sessions, so this is much larger than the fresh-run
/// scan caps used by dispatch.
const INTERACTIVE_SCAN_CAP: usize = 500;

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
}
