//! Where agy keeps a conversation's log, for `dispatch_logs` of an agy
//! execution and for the agy transcript reader.
//!
//! agy has no home override: its state is under `<home>/.gemini`, the home
//! being `HOME` (on Windows `USERPROFILE`, which the server resolves the same
//! way). A conversation's steps are appended live to
//! `<home>/.gemini/antigravity-cli/brain/<conversation_id>/.system_generated/
//! logs/transcript.jsonl`; `<home>/.gemini/antigravity-cli/cache/
//! last_conversations.json` maps a workspace path to its latest conversation.
//! An execution is identified by the conversation id its `init` event named,
//! never by the workspace's latest conversation, which another agy session in
//! the same directory can replace at any time.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// `<home>/.gemini/antigravity-cli`.
pub(crate) fn cli_home(home: &Path) -> PathBuf {
    home.join(".gemini").join("antigravity-cli")
}

/// The live log of conversation `conversation_id`.
pub(crate) fn transcript_path(home: &Path, conversation_id: &str) -> PathBuf {
    cli_home(home)
        .join("brain")
        .join(conversation_id)
        .join(".system_generated")
        .join("logs")
        .join("transcript.jsonl")
}

/// Whether `path` is the log of conversation `conversation_id`, judged by its
/// last five components (`brain/<id>/.system_generated/logs/transcript.jsonl`)
/// so that the home it was recorded under does not matter.
pub(crate) fn is_transcript_of(path: &Path, conversation_id: &str) -> bool {
    if conversation_id.is_empty() {
        return false;
    }
    let tail: Vec<&std::ffi::OsStr> = path
        .components()
        .rev()
        .take(5)
        .map(|c| c.as_os_str())
        .collect();
    tail.len() == 5
        && tail[0] == "transcript.jsonl"
        && tail[1] == "logs"
        && tail[2] == ".system_generated"
        && tail[3] == conversation_id
        && tail[4] == "brain"
}

/// A transcript tool call's name: `name`, `tool_name` or `function.name`,
/// else `?`.
pub(crate) fn tool_name(call: &Value) -> &str {
    call.get("name")
        .or_else(|| call.get("tool_name"))
        .or_else(|| call.get("function").and_then(|f| f.get("name")))
        .and_then(Value::as_str)
        .unwrap_or("?")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_transcript_path_names_the_conversation() {
        let home = Path::new("h");
        let p = transcript_path(home, "e195bb9c");
        let parts: Vec<String> = p
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            parts,
            vec![
                "h",
                ".gemini",
                "antigravity-cli",
                "brain",
                "e195bb9c",
                ".system_generated",
                "logs",
                "transcript.jsonl"
            ]
        );
        assert!(is_transcript_of(&p, "e195bb9c"));
        assert!(!is_transcript_of(&p, "other"));
        assert!(!is_transcript_of(&p, ""));
        assert!(!is_transcript_of(Path::new("transcript.jsonl"), "e195bb9c"));
        assert!(is_transcript_of(
            &transcript_path(Path::new("elsewhere"), "e195bb9c"),
            "e195bb9c"
        ));
    }
}
