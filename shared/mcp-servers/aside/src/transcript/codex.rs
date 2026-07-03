//! Codex CLI transcript reader.
//!
//! Reads the newest **interactive** rollout for this project from
//! `$CODEX_HOME/sessions` (discovery lives in the shared `harness-log`
//! crate, which also excludes headless `codex exec` children — the runs
//! aside and dispatch themselves spawn). Rollout events map to the unified
//! redaction contract; codex reasoning is API-encrypted and always rendered
//! as a `[thinking]` placeholder.

use std::path::Path;
use std::time::SystemTime;

use harness_log::codex::{codex_home, file_mtime, newest_interactive_rollout};
use serde_json::Value;

use super::Located;

pub(crate) fn locate(project_dir: &Path) -> Result<Located, String> {
    let root = codex_home().join("sessions");
    match newest_interactive_rollout(&root, project_dir) {
        Some(path) => {
            let mtime = file_mtime(&path).unwrap_or(SystemTime::UNIX_EPOCH);
            Ok(Located { path, mtime })
        }
        None => Err(format!(
            "no interactive Codex rollout for this cwd under {}",
            root.display()
        )),
    }
}

/// Render a Codex rollout file into message blocks, oldest → newest.
pub(crate) fn render_file(path: &Path) -> Vec<String> {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    content.lines().filter_map(render_line).collect()
}

fn render_line(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let o: Value = serde_json::from_str(line).ok()?;
    let t = o.get("type")?.as_str()?;
    let p = o.get("payload")?;
    let pt = p.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match (t, pt) {
        ("event_msg", "user_message") => {
            let m = p.get("message").and_then(|v| v.as_str())?;
            if m.trim().is_empty() {
                return None;
            }
            Some(format!("[user] {m}"))
        }
        ("event_msg", "agent_message") => {
            let m = p.get("message").and_then(|v| v.as_str())?;
            if m.trim().is_empty() {
                return None;
            }
            Some(format!("[assistant] {m}"))
        }
        ("response_item", "function_call") | ("response_item", "custom_tool_call") => {
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            Some(format!("[tool_use: {name}]"))
        }
        ("response_item", "web_search_call") => Some("[tool_use: web_search]".to_string()),
        ("response_item", "function_call_output")
        | ("response_item", "custom_tool_call_output") => Some("[tool_result]".to_string()),
        // Codex reasoning is encrypted (`encrypted_content`) — never forward.
        ("response_item", "reasoning") => Some("[thinking]".to_string()),
        // session_meta, turn_context, token_count, task_started/complete,
        // response_item/message (developer/system dupes), compacted: noise.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_line_maps_rollout_events_to_redaction_contract() {
        let jsonl = [
            r#"{"type":"session_meta","payload":{"session_id":"s","cwd":"/w","originator":"codex-tui","source":"cli"}}"#,
            r#"{"type":"event_msg","payload":{"type":"user_message","message":"please fix it"}}"#,
            r#"{"type":"response_item","payload":{"type":"reasoning","encrypted_content":"gAAAA-secret","summary":[]}}"#,
            r#"{"type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"ls\"}"}}"#,
            r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"c1","output":"raw output"}}"#,
            r#"{"type":"response_item","payload":{"type":"custom_tool_call","name":"apply_patch","input":"*** Begin Patch"}}"#,
            r#"{"type":"response_item","payload":{"type":"custom_tool_call_output","output":"ok"}}"#,
            r#"{"type":"response_item","payload":{"type":"web_search_call","action":{"type":"search","query":"q"}}}"#,
            r#"{"type":"event_msg","payload":{"type":"agent_message","message":"done"}}"#,
            r#"{"type":"event_msg","payload":{"type":"token_count","payload":{}}}"#,
            r#"{"type":"turn_context","payload":{"cwd":"/w"}}"#,
        ]
        .join("\n");
        let blocks: Vec<String> = jsonl.lines().filter_map(render_line).collect();
        assert_eq!(
            blocks,
            vec![
                "[user] please fix it",
                "[thinking]",
                "[tool_use: exec_command]",
                "[tool_result]",
                "[tool_use: apply_patch]",
                "[tool_result]",
                "[tool_use: web_search]",
                "[assistant] done",
            ]
        );
        // no encrypted reasoning or tool payloads leak
        let joined = blocks.join("\n");
        assert!(!joined.contains("gAAAA"));
        assert!(!joined.contains("Begin Patch"));
        assert!(!joined.contains("raw output"));
    }

    #[test]
    fn render_line_tolerates_garbage_and_empty_messages() {
        assert!(render_line("").is_none());
        assert!(render_line("{half written").is_none());
        assert!(
            render_line(r#"{"type":"event_msg","payload":{"type":"user_message","message":"  "}}"#)
                .is_none()
        );
    }
}
