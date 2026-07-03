//! Kimi Code CLI transcript reader.
//!
//! Kimi indexes sessions in `$KIMI_CODE_HOME/session_index.jsonl` (one JSON
//! per line: `sessionId`, `sessionDir`, `workDir`) and appends the session's
//! wire-protocol log to `<sessionDir>/agents/main/wire.jsonl`. Sessions whose
//! `workDir` matches this project are preferred; because the Kimi plugin
//! manifest pins the MCP server's cwd to the plugin directory, a cwd-less
//! fallback accepts the globally newest wire log **only when it is fresh**
//! (the invoking session appended events milliseconds before this call).
//!
//! `config.update` events carry Kimi's full system prompt and are never
//! forwarded. Only the main agent's wire log is read; sub-agent logs under
//! `agents/<id>/` are out of scope.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::Value;

use super::Located;

/// Max age of the newest wire log for the cwd-less fallback to accept it as
/// "the live session".
const LIVE_SESSION_MAX_AGE: Duration = Duration::from_secs(120);

fn kimi_home(home: &Path) -> PathBuf {
    std::env::var_os("KIMI_CODE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".kimi-code"))
}

pub(crate) fn locate(project_dir: &Path, home: &Path) -> Result<Located, String> {
    locate_in(&kimi_home(home), project_dir, SystemTime::now())
}

fn locate_in(kimi_home: &Path, project_dir: &Path, now: SystemTime) -> Result<Located, String> {
    let index = kimi_home.join("session_index.jsonl");
    let content = std::fs::read_to_string(&index)
        .map_err(|e| format!("cannot read {}: {}", index.display(), e))?;

    // Dedupe by sessionId — the last index entry for a session wins.
    let mut sessions: HashMap<String, (PathBuf, PathBuf)> = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let (Some(sid), Some(dir), Some(work)) = (
            v.get("sessionId").and_then(|s| s.as_str()),
            v.get("sessionDir").and_then(|s| s.as_str()),
            v.get("workDir").and_then(|s| s.as_str()),
        ) else {
            continue;
        };
        sessions.insert(
            sid.to_string(),
            (PathBuf::from(dir), PathBuf::from(work)),
        );
    }

    let mut matching: Vec<Located> = Vec::new();
    let mut newest_any: Option<Located> = None;
    for (dir, work) in sessions.values() {
        let wire = dir.join("agents").join("main").join("wire.jsonl");
        let Some(mtime) = std::fs::metadata(&wire).ok().and_then(|m| m.modified().ok()) else {
            continue;
        };
        let loc = Located { path: wire, mtime };
        let work_canon = work.canonicalize().unwrap_or_else(|_| work.clone());
        if work_canon == project_dir {
            matching.push(loc);
        } else if newest_any.as_ref().map(|b| mtime > b.mtime).unwrap_or(true) {
            newest_any = Some(loc);
        }
    }

    // Newest matching session that actually has renderable messages — a
    // freshly-opened empty "New Session" must not shadow the real one.
    matching.sort_by(|a, b| b.mtime.cmp(&a.mtime));
    for loc in &matching {
        if !render_file(&loc.path).is_empty() {
            return Ok(loc.clone());
        }
    }
    if let Some(best) = matching.into_iter().next() {
        return Ok(best);
    }
    // cwd-less fallback, freshness-gated. A future mtime (clock skew) counts
    // as live.
    if let Some(best) = newest_any
        && now
            .duration_since(best.mtime)
            .map(|age| age <= LIVE_SESSION_MAX_AGE)
            .unwrap_or(true)
    {
        return Ok(best);
    }
    Err("no Kimi session matches this project and no session is live".to_string())
}

/// Render a Kimi wire.jsonl file into message blocks, oldest → newest.
pub(crate) fn render_file(path: &Path) -> Vec<String> {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    content.lines().filter_map(render_line).collect()
}

fn render_line(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    match v.get("type")?.as_str()? {
        "context.append_message" => {
            let msg = v.get("message")?;
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
            let mut parts: Vec<String> = Vec::new();
            if let Some(items) = msg.get("content").and_then(|c| c.as_array()) {
                for item in items {
                    if let Some(t) = item.get("text").and_then(|t| t.as_str())
                        && !t.trim().is_empty()
                    {
                        parts.push(t.to_string());
                    }
                }
            }
            if let Some(calls) = msg.get("toolCalls").and_then(|c| c.as_array()) {
                for call in calls {
                    let name = call.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                    parts.push(format!("[tool_use: {name}]"));
                }
            }
            if parts.is_empty() {
                return None;
            }
            Some(format!("[{role}] {}", parts.join("\n")))
        }
        "context.append_loop_event" => {
            let ev = v.get("event")?;
            match ev.get("type")?.as_str()? {
                "content.part" => {
                    let part = ev.get("part")?;
                    match part.get("type")?.as_str()? {
                        "text" => {
                            let t = part.get("text").and_then(|t| t.as_str())?;
                            if t.trim().is_empty() {
                                return None;
                            }
                            Some(format!("[assistant] {t}"))
                        }
                        // Reasoning — never forward the content.
                        "think" => Some("[thinking]".to_string()),
                        _ => None,
                    }
                }
                "tool.call" => {
                    let name = ev.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                    Some(format!("[tool_use: {name}]"))
                }
                "tool.result" => Some("[tool_result]".to_string()),
                // step.begin / step.end and friends: noise.
                _ => None,
            }
        }
        // Skipped by design:
        // - `config.update` carries the FULL system prompt — never forward.
        // - `turn.prompt` duplicates the user `context.append_message`.
        // - metadata / usage.record / permission.* / tools.* / plan_mode.* /
        //   turn.cancel: noise.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn render_line_maps_wire_events_and_never_leaks_system_prompt() {
        let jsonl = [
            r#"{"type":"metadata","protocol_version":"1.4","created_at":1}"#,
            r#"{"type":"config.update","profileName":"agent","systemPrompt":"TOP-SECRET-SYSTEM-PROMPT"}"#,
            r#"{"type":"context.append_message","message":{"role":"user","content":[{"type":"text","text":"do the task"}],"toolCalls":[],"origin":{"kind":"user"}}}"#,
            r#"{"type":"turn.prompt","input":[{"type":"text","text":"do the task"}],"origin":{"kind":"user"}}"#,
            r#"{"type":"context.append_loop_event","event":{"type":"step.begin","stepUuid":"s1"}}"#,
            r#"{"type":"context.append_loop_event","event":{"type":"content.part","part":{"type":"think","think":"private reasoning"},"stepUuid":"s1"}}"#,
            r#"{"type":"context.append_loop_event","event":{"type":"tool.call","toolCallId":"c1","name":"Read","args":{"path":"/etc/passwd"}}}"#,
            r#"{"type":"context.append_loop_event","event":{"type":"tool.result","toolCallId":"c1","result":{"output":"root:x:0:0"}}}"#,
            r#"{"type":"context.append_loop_event","event":{"type":"content.part","part":{"type":"text","text":"finished"},"stepUuid":"s1"}}"#,
            r#"{"type":"usage.record","usage":{}}"#,
        ]
        .join("\n");
        let blocks: Vec<String> = jsonl.lines().filter_map(render_line).collect();
        assert_eq!(
            blocks,
            vec![
                "[user] do the task",
                "[thinking]",
                "[tool_use: Read]",
                "[tool_result]",
                "[assistant] finished",
            ]
        );
        let joined = blocks.join("\n");
        assert!(!joined.contains("TOP-SECRET"));
        assert!(!joined.contains("private reasoning"));
        assert!(!joined.contains("/etc/passwd"));
        assert!(!joined.contains("root:x"));
    }

    #[test]
    fn injection_messages_render_as_user_blocks() {
        let line = r#"{"type":"context.append_message","message":{"role":"user","content":[{"type":"text","text":"<system-reminder>note</system-reminder>"}],"origin":{"kind":"injection","variant":"plan_mode"}}}"#;
        // Parity with Claude, which forwards system-reminders embedded in user
        // messages; the prompt's role framing defuses them.
        assert_eq!(
            render_line(line).unwrap(),
            "[user] <system-reminder>note</system-reminder>"
        );
    }

    fn setup_home(tag: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!(
            "aside-kimi-test-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        home
    }

    fn add_session(home: &Path, sid: &str, work: &Path, wire_lines: &str) -> PathBuf {
        let sdir = home.join("sessions").join(sid);
        let agent = sdir.join("agents/main");
        std::fs::create_dir_all(&agent).unwrap();
        let wire = agent.join("wire.jsonl");
        std::fs::write(&wire, wire_lines).unwrap();
        let entry = json!({
            "sessionId": sid,
            "sessionDir": sdir.to_string_lossy(),
            "workDir": work.to_string_lossy(),
        });
        let index = home.join("session_index.jsonl");
        let mut content = std::fs::read_to_string(&index).unwrap_or_default();
        content.push_str(&format!("{entry}\n"));
        std::fs::write(&index, content).unwrap();
        wire
    }

    #[test]
    fn locate_prefers_workdir_match_over_newer_other_project() {
        let home = setup_home("match");
        let proj = home.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let proj = proj.canonicalize().unwrap();
        let other = home.join("other");
        std::fs::create_dir_all(&other).unwrap();

        let mine = add_session(&home, "session_a", &proj, "{}\n");
        let theirs = add_session(&home, "session_b", &other, "{}\n");
        // make the other project's wire strictly newer
        std::fs::File::options()
            .append(true)
            .open(&theirs)
            .unwrap()
            .set_modified(SystemTime::now() + Duration::from_secs(5))
            .unwrap();

        let got = locate_in(&home, &proj, SystemTime::now()).unwrap();
        assert_eq!(got.path, mine);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn locate_skips_empty_newest_session_for_older_renderable_one() {
        let home = setup_home("shadow");
        let proj = home.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let proj = proj.canonicalize().unwrap();

        let full = add_session(
            &home,
            "session_full",
            &proj,
            "{\"type\":\"context.append_message\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"hello\"}]}}\n",
        );
        let empty = add_session(&home, "session_empty", &proj, "{\"type\":\"metadata\"}\n");
        std::fs::File::options()
            .append(true)
            .open(&empty)
            .unwrap()
            .set_modified(SystemTime::now() + Duration::from_secs(5))
            .unwrap();

        let got = locate_in(&home, &proj, SystemTime::now()).unwrap();
        assert_eq!(got.path, full, "empty newest session must not shadow the renderable one");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn locate_fallback_is_freshness_gated() {
        let home = setup_home("fresh");
        let other = home.join("other");
        std::fs::create_dir_all(&other).unwrap();
        let wire = add_session(&home, "session_x", &other, "{}\n");
        let unrelated = home.join("nomatch");

        // fresh wire → accepted even though workDir doesn't match
        let got = locate_in(&home, &unrelated, SystemTime::now()).unwrap();
        assert_eq!(got.path, wire);

        // stale wire → rejected
        let stale_now = SystemTime::now() + Duration::from_secs(600);
        assert!(locate_in(&home, &unrelated, stale_now).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn locate_dedupes_index_by_session_id_last_wins() {
        let home = setup_home("dedupe");
        let proj = home.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let proj = proj.canonicalize().unwrap();
        let other = home.join("other");
        std::fs::create_dir_all(&other).unwrap();

        // same sessionId listed twice: first for proj, later re-pointed elsewhere
        add_session(&home, "session_dup", &proj, "{}\n");
        add_session(&home, "session_dup", &other, "{}\n");

        // last entry wins → no workDir match for proj, and the wire is fresh,
        // so the fallback returns it; with a stale clock it errors instead.
        let stale_now = SystemTime::now() + Duration::from_secs(600);
        assert!(locate_in(&home, &proj, stale_now).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }
}
