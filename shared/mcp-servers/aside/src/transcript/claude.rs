//! Claude Code transcript reader.
//!
//! Claude Code writes one `<session-uuid>.jsonl` per session under
//! `~/.claude/projects/<dashed-cwd>/`, where `<dashed-cwd>` is the project
//! path with `/` replaced by `-` (dots and underscores preserved). Entries of
//! `type` `user`/`assistant` carry a `message.content` that is either a plain
//! string or an array of content blocks (`text`, `tool_use`, `tool_result`,
//! `thinking`).

use std::path::Path;
use std::time::SystemTime;

use serde_json::Value;

use super::{Located, newest_jsonl};

fn slug(p: &Path) -> String {
    p.to_string_lossy().replace('/', "-")
}

/// Find the newest session transcript for this project. Tries the canonical
/// project-dir slug first, then the raw (uncanonicalized) cwd slug — Claude
/// slugs whatever cwd the session ran in.
pub(crate) fn locate(project_dir: &Path, raw_cwd: &Path, home: &Path) -> Result<Located, String> {
    let root = home.join(".claude").join("projects");
    let mut candidates = vec![root.join(slug(project_dir))];
    let raw = root.join(slug(raw_cwd));
    if raw != candidates[0] {
        candidates.push(raw);
    }
    for dir in &candidates {
        if dir.is_dir()
            && let Some(path) = newest_jsonl(dir)
        {
            let mtime = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            return Ok(Located { path, mtime });
        }
    }
    Err(format!(
        "no Claude transcript dir at {}",
        candidates[0].display()
    ))
}

/// Render a Claude transcript file into message blocks, oldest → newest.
pub(crate) fn render_file(path: &Path) -> Vec<String> {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    content.lines().filter_map(render_entry).collect()
}

/// Render one JSONL entry into a short plain-text block, or `None` if it
/// doesn't look like a message (e.g. summary / system entries).
fn render_entry(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    let ty = v.get("type")?.as_str()?;
    match ty {
        "user" | "assistant" => {
            let msg = v.get("message")?;
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or(ty);
            let content = msg.get("content")?;
            let body = render_content(content)?;
            if body.trim().is_empty() {
                return None;
            }
            Some(format!("[{}] {}", role, body))
        }
        _ => None,
    }
}

fn render_content(content: &Value) -> Option<String> {
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => {
            let mut parts: Vec<String> = Vec::new();
            for item in items {
                if let Some(s) = item.as_str() {
                    parts.push(s.to_string());
                    continue;
                }
                let ty = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                match ty {
                    "text" => {
                        if let Some(t) = item.get("text").and_then(|t| t.as_str()) {
                            parts.push(t.to_string());
                        }
                    }
                    "tool_use" => {
                        let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                        parts.push(format!("[tool_use: {}]", name));
                    }
                    "tool_result" => {
                        parts.push("[tool_result]".to_string());
                    }
                    "thinking" => {
                        // Don't leak thinking blocks to external advisors.
                        parts.push("[thinking]".to_string());
                    }
                    _ => {}
                }
            }
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn render_entry_redacts_tool_blocks_and_thinking() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[
            {"type":"text","text":"here is the plan"},
            {"type":"thinking","thinking":"secret chain of thought"},
            {"type":"tool_use","name":"Bash","input":{"command":"rm -rf /"}},
            {"type":"tool_result","content":"file contents"}
        ]}}"#
            .replace('\n', " ");
        let out = render_entry(&line).unwrap();
        assert!(out.contains("here is the plan"));
        assert!(out.contains("[tool_use: Bash]"));
        assert!(out.contains("[tool_result]"));
        assert!(out.contains("[thinking]"));
        assert!(!out.contains("secret chain"));
        assert!(!out.contains("rm -rf"));
        assert!(!out.contains("file contents"));
    }

    #[test]
    fn render_entry_passes_plain_text_and_skips_non_messages() {
        let user = r#"{"type":"user","message":{"role":"user","content":"fix the bug"}}"#;
        assert_eq!(render_entry(user).unwrap(), "[user] fix the bug");
        let summary = r#"{"type":"summary","summary":"..."}"#;
        assert!(render_entry(summary).is_none());
        assert!(render_entry("not json").is_none());
    }

    #[test]
    fn locate_prefers_canonical_slug_and_falls_back_to_raw() {
        let home = std::env::temp_dir().join(format!(
            "aside-claude-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&home);
        let proj_dir = home.join(".claude/projects/-w-proj");
        std::fs::create_dir_all(&proj_dir).unwrap();
        std::fs::write(proj_dir.join("s1.jsonl"), "{}\n").unwrap();

        let got = locate(Path::new("/w/proj"), Path::new("/other/raw"), &home).unwrap();
        assert_eq!(got.path, proj_dir.join("s1.jsonl"));

        // canonical slug dir absent → raw cwd slug is tried
        let raw_dir = home.join(".claude/projects/-r-aw");
        std::fs::create_dir_all(&raw_dir).unwrap();
        std::fs::write(raw_dir.join("s2.jsonl"), "{}\n").unwrap();
        let got2 = locate(Path::new("/no/such"), Path::new("/r/aw"), &home).unwrap();
        assert_eq!(got2.path, raw_dir.join("s2.jsonl"));

        let missing: PathBuf = home.join(".claude/projects/-a-bsent");
        assert!(!missing.exists());
        assert!(locate(Path::new("/a/bsent"), Path::new("/a/bsent"), &home).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }
}
