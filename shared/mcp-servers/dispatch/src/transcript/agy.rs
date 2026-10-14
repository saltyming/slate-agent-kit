//! Antigravity CLI (agy) transcript reader.
//!
//! The ambient conversation of a working directory is
//! `<home>/.gemini/antigravity-cli/cache/last_conversations.json[<dir>]` (a
//! JSON object mapping a workspace path to a conversation id), and its steps
//! are `brain/<id>/.system_generated/logs/transcript.jsonl` (`crate::agy`):
//! one JSON object per line with `step_index`, `source` (`USER_EXPLICIT`,
//! `MODEL`), `type` (`USER_INPUT`, `PLANNER_RESPONSE`, `GENERIC`), `status`,
//! `created_at`, `content` and, on model steps, `input_tokens`,
//! `output_tokens`, `cache_read_tokens`, `thinking` and `tool_calls`.
//!
//! Redaction follows the other readers: user input and planner text pass
//! verbatim; `thinking` becomes `[thinking]`, each tool call
//! `[tool_use: <name>]`, and a `GENERIC` step's content (system-generated
//! step output, such as a tool's) `[tool_result]`.

use std::path::Path;

use serde_json::Value;

use super::Located;
use crate::agy;

/// Find the transcript of the latest conversation agy recorded for this
/// project: `project_dir` first, then the raw cwd, each matched against the
/// keys of `last_conversations.json` as written or canonicalized.
pub(crate) fn locate(project_dir: &Path, raw_cwd: &Path, home: &Path) -> Result<Located, String> {
    let index = agy::cli_home(home)
        .join("cache")
        .join("last_conversations.json");
    let content = std::fs::read_to_string(&index)
        .map_err(|e| format!("cannot read {}: {}", index.display(), e))?;
    let map: Value = serde_json::from_str(&content)
        .map_err(|e| format!("cannot parse {}: {}", index.display(), e))?;
    let Some(map) = map.as_object() else {
        return Err(format!("{} is not a JSON object", index.display()));
    };
    let id = [project_dir, raw_cwd]
        .iter()
        .find_map(|dir| conversation_for(map, dir))
        .ok_or_else(|| {
            format!(
                "no agy conversation recorded for {} in {}",
                project_dir.display(),
                index.display()
            )
        })?;
    let path = agy::transcript_path(home, &id);
    let mtime = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    Ok(Located { path, mtime })
}

/// The conversation id `map` records for `dir`: a key equal to `dir`, else a
/// key that canonicalizes to it.
fn conversation_for(map: &serde_json::Map<String, Value>, dir: &Path) -> Option<String> {
    let as_id = |v: &Value| v.as_str().filter(|s| !s.is_empty()).map(str::to_string);
    if let Some(id) = map.get(dir.to_string_lossy().as_ref()).and_then(as_id) {
        return Some(id);
    }
    let canon = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    map.iter().find_map(|(key, v)| {
        let k = Path::new(key);
        (k.canonicalize().unwrap_or_else(|_| k.to_path_buf()) == canon)
            .then(|| as_id(v))
            .flatten()
    })
}

/// Render an agy transcript file into message blocks, oldest → newest.
pub(crate) fn render_file(path: &Path) -> Vec<String> {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    content.lines().filter_map(render_line).collect()
}

/// Render one step, or `None` for a step with nothing to forward.
fn render_line(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    let content = v
        .get("content")
        .and_then(Value::as_str)
        .filter(|c| !c.trim().is_empty());
    match v.get("type")?.as_str()? {
        "USER_INPUT" => content.map(|c| format!("[user] {c}")),
        "PLANNER_RESPONSE" => {
            let mut parts: Vec<String> = Vec::new();
            if v.get("thinking")
                .and_then(Value::as_str)
                .is_some_and(|t| !t.trim().is_empty())
            {
                parts.push("[thinking]".to_string());
            }
            if let Some(c) = content {
                parts.push(c.to_string());
            }
            for call in v
                .get("tool_calls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                parts.push(format!("[tool_use: {}]", agy::tool_name(call)));
            }
            (!parts.is_empty()).then(|| format!("[assistant] {}", parts.join("\n")))
        }
        "GENERIC" => content.map(|_| "[tool_result]".to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "dispatch-agy-transcript-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    const STEPS: &str = concat!(
        r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-10-14T00:06:44Z","content":"reply ok"}"#,
        "\n",
        r#"{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-10-14T00:06:47Z","input_tokens":11936,"cache_read_tokens":0,"output_tokens":26,"thinking":"secret chain of thought","tool_calls":[{"name":"run_command","args":{"CommandLine":"rm -rf /"}}],"content":"ok"}"#,
        "\n",
        r#"{"step_index":2,"source":"MODEL","type":"GENERIC","status":"DONE","created_at":"2026-10-14T00:06:48Z","content":"raw tool output"}"#,
        "\n",
        r#"{"step_index":3,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-10-14T00:06:49Z","content":"done"}"#,
        "\n",
        "{half written",
    );

    #[test]
    fn steps_map_to_the_redaction_contract() {
        let blocks: Vec<String> = STEPS.lines().filter_map(render_line).collect();
        assert_eq!(
            blocks,
            vec![
                "[user] reply ok",
                "[assistant] [thinking]\nok\n[tool_use: run_command]",
                "[tool_result]",
                "[assistant] done",
            ]
        );
        let joined = blocks.join("\n");
        assert!(!joined.contains("secret chain"));
        assert!(!joined.contains("rm -rf"));
        assert!(!joined.contains("raw tool output"));
    }

    #[test]
    fn locate_follows_last_conversations_for_this_directory() {
        let home = scratch("locate");
        let project = home.join("proj");
        std::fs::create_dir_all(&project).unwrap();
        let project = project.canonicalize().unwrap();
        let cache = agy::cli_home(&home).join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        let index = serde_json::json!({
            project.to_string_lossy(): "conv-live",
            "/some/other/dir": "conv-other",
        });
        std::fs::write(cache.join("last_conversations.json"), index.to_string()).unwrap();

        // The conversation is named but its log is not there yet.
        assert!(locate(&project, &project, &home).is_err());

        let path = agy::transcript_path(&home, "conv-live");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, STEPS).unwrap();
        let got = locate(&project, &project, &home).unwrap();
        assert_eq!(got.path, path);
        assert_eq!(render_file(&got.path).len(), 4);

        // A directory agy never ran in has no conversation.
        let elsewhere = home.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let err = locate(&elsewhere, &elsewhere, &home).unwrap_err();
        assert!(err.contains("no agy conversation"), "{err}");

        // No cache file at all.
        let empty = scratch("locate-empty");
        assert!(locate(&project, &project, &empty).is_err());
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&empty);
    }
}
