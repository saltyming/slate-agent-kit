//! Optional transcript forwarding — native, multi-harness.
//!
//! aside forwards a REDACTED tail of the invoking harness's own session log:
//! - Claude Code:   `~/.claude/projects/<dashed-cwd>/<uuid>.jsonl`
//! - Codex CLI:     `$CODEX_HOME/sessions/**/rollout-*.jsonl` (interactive
//!   sessions only — headless `codex exec` children are excluded so aside
//!   never forwards its own prior consultation as "the conversation")
//! - Kimi Code CLI: `$KIMI_CODE_HOME/session_index.jsonl` →
//!   `<sessionDir>/agents/main/wire.jsonl`
//!
//! Source selection, in precedence order:
//! 1. `ASIDE_TRANSCRIPT_DIR` — explicit directory of Claude-schema `.jsonl`
//!    files (tests / unusual setups). Installers do not set this.
//! 2. `ASIDE_HARNESS` = `claude` | `codex` | `kimi` — set by the installer at
//!    MCP registration time; deterministic.
//! 3. Auto-detect — each reader locates a candidate for this project and the
//!    newest source file wins. The invoking harness appended to its log
//!    milliseconds before this MCP call, so newest-wins picks the live one.
//!
//! Redaction contract (identical across harnesses): user/assistant text passes
//! verbatim; tool calls become `[tool_use: <name>]` (name only); tool results
//! become `[tool_result]`; reasoning/thinking becomes `[thinking]`. Missing
//! sources, unparseable lines, or unknown shapes degrade to `Unavailable` —
//! the caller's `question` and `context` always reach the backend.

mod claude;
mod codex;
mod kimi;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

const DEFAULT_TAIL: usize = 80;
const MAX_RENDERED_BYTES: usize = 100 * 1024; // 100 KB

/// Result of attempting to build a transcript section for the prompt.
pub enum TranscriptOutcome {
    /// Transcript rendered successfully. `rendered` is the plain text to
    /// splice into the prompt; a `[transcript source: ...]` note (and a
    /// truncation header when trimming occurred) is already prepended.
    Ok { rendered: String },
    /// No transcript source found for this project. The caller should proceed
    /// without transcript and surface the reason so the user can pass
    /// `context` explicitly if they need it.
    Unavailable(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Harness {
    Claude,
    Codex,
    Kimi,
}

impl Harness {
    fn name(self) -> &'static str {
        match self {
            Harness::Claude => "claude",
            Harness::Codex => "codex",
            Harness::Kimi => "kimi",
        }
    }
}

/// A located transcript source file.
#[derive(Debug, Clone)]
struct Located {
    path: PathBuf,
    mtime: SystemTime,
}

fn harness_from_env() -> Option<Harness> {
    let v = std::env::var("ASIDE_HARNESS").ok()?;
    let v = v.trim().to_ascii_lowercase();
    if v.is_empty() {
        return None;
    }
    match v.as_str() {
        "claude" => Some(Harness::Claude),
        "codex" => Some(Harness::Codex),
        "kimi" => Some(Harness::Kimi),
        other => {
            tracing::warn!(
                "ASIDE_HARNESS has unknown value {other:?}; falling back to auto-detect"
            );
            None
        }
    }
}

/// The project directory this session is about. MCP servers are not always
/// spawned in the project (the Kimi plugin pins cwd to the plugin dir), so an
/// explicit env override wins over the process cwd — same chain as dispatch.
fn resolve_project_dir(cwd: &Path) -> PathBuf {
    for key in [
        "SLATE_PROJECT_DIR",
        "AGENT_KIT_PROJECT_DIR",
        "CLAUDE_PROJECT_DIR",
    ] {
        if let Some(v) = std::env::var_os(key)
            && !v.is_empty()
        {
            let p = PathBuf::from(v);
            return p.canonicalize().unwrap_or(p);
        }
    }
    cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf())
}

/// Render the current session's transcript tail as plain text suitable for
/// prompt inclusion. `tail` defaults to `DEFAULT_TAIL` messages when `None`.
pub fn render_transcript(cwd: &Path, home: &Path, tail: Option<u32>) -> TranscriptOutcome {
    // Explicit override: a directory of Claude-schema <uuid>.jsonl files.
    if let Some(dir) = std::env::var_os("ASIDE_TRANSCRIPT_DIR")
        && !dir.is_empty()
    {
        let dir = PathBuf::from(dir);
        if !dir.exists() {
            return TranscriptOutcome::Unavailable(format!(
                "ASIDE_TRANSCRIPT_DIR={} does not exist",
                dir.display()
            ));
        }
        return match newest_jsonl(&dir) {
            Some(p) => finish(claude::render_file(&p), tail, "override", &p),
            None => TranscriptOutcome::Unavailable(format!(
                "no .jsonl files in {}",
                dir.display()
            )),
        };
    }

    let project_dir = resolve_project_dir(cwd);

    match harness_from_env() {
        Some(h) => match locate_for(h, cwd, &project_dir, home) {
            Ok(loc) => finish(render_for_path(h, &loc.path), tail, h.name(), &loc.path),
            Err(reason) => {
                TranscriptOutcome::Unavailable(format!("{}: {}", h.name(), reason))
            }
        },
        None => {
            // Auto-detect: the newest located source wins.
            let mut attempts: Vec<String> = Vec::new();
            let mut best: Option<(Harness, Located)> = None;
            for h in [Harness::Claude, Harness::Codex, Harness::Kimi] {
                match locate_for(h, cwd, &project_dir, home) {
                    Ok(loc) => {
                        if best
                            .as_ref()
                            .map(|(_, b)| loc.mtime > b.mtime)
                            .unwrap_or(true)
                        {
                            best = Some((h, loc));
                        }
                    }
                    Err(reason) => attempts.push(format!("{}: {}", h.name(), reason)),
                }
            }
            match best {
                Some((h, loc)) => {
                    finish(render_for_path(h, &loc.path), tail, h.name(), &loc.path)
                }
                None => TranscriptOutcome::Unavailable(format!(
                    "no transcript source found — {}",
                    attempts.join("; ")
                )),
            }
        }
    }
}

fn locate_for(
    h: Harness,
    raw_cwd: &Path,
    project_dir: &Path,
    home: &Path,
) -> Result<Located, String> {
    match h {
        Harness::Claude => claude::locate(project_dir, raw_cwd, home),
        Harness::Codex => codex::locate(project_dir),
        Harness::Kimi => kimi::locate(project_dir, home),
    }
}

fn render_for_path(h: Harness, path: &Path) -> Vec<String> {
    match h {
        Harness::Claude => claude::render_file(path),
        Harness::Codex => codex::render_file(path),
        Harness::Kimi => kimi::render_file(path),
    }
}

/// Locate the newest `.jsonl` file in `dir`, by modification time.
pub(crate) fn newest_jsonl(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut best: Option<(PathBuf, SystemTime)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let mtime = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        match &best {
            None => best = Some((path, mtime)),
            Some((_, prev)) if mtime > *prev => best = Some((path, mtime)),
            _ => {}
        }
    }
    best.map(|(p, _)| p)
}

/// Apply the tail limit and byte budget to rendered message blocks, then
/// prepend the source note (and a truncation header when trimming occurred).
///
/// Byte budget: if the result exceeds `MAX_RENDERED_BYTES`, messages are
/// dropped from the **front** (keeping the most recent).
fn finish(messages: Vec<String>, tail: Option<u32>, source: &str, path: &Path) -> TranscriptOutcome {
    if messages.is_empty() {
        return TranscriptOutcome::Unavailable(format!(
            "transcript at {} has no renderable messages",
            path.display()
        ));
    }
    let total = messages.len();
    let want_tail = tail.map(|n| n as usize).unwrap_or(DEFAULT_TAIL).max(1);
    let start = total.saturating_sub(want_tail);
    let mut kept: Vec<&str> = messages[start..].iter().map(|s| s.as_str()).collect();
    let mut kept_count = kept.len();

    let mut rendered = kept.join("\n\n");
    let mut trimmed = false;
    while rendered.len() > MAX_RENDERED_BYTES && kept.len() > 1 {
        kept.remove(0);
        kept_count -= 1;
        rendered = kept.join("\n\n");
        trimmed = true;
    }

    let file = path.file_name().and_then(|s| s.to_str()).unwrap_or("?");
    let mut head = format!("[transcript source: {source} {file}]");
    if trimmed || kept_count < total {
        head = format!(
            "{head}\n[transcript truncated: kept last {kept_count} of {total} messages]"
        );
    }
    TranscriptOutcome::Ok {
        rendered: format!("{head}\n\n{rendered}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(n: usize) -> Vec<String> {
        (1..=n).map(|i| format!("[user] message {i}")).collect()
    }

    #[test]
    fn finish_applies_tail_and_notes_source() {
        let out = finish(blocks(5), Some(2), "claude", Path::new("/x/abc.jsonl"));
        let TranscriptOutcome::Ok { rendered } = out else {
            panic!("expected Ok");
        };
        assert!(rendered.starts_with("[transcript source: claude abc.jsonl]"));
        assert!(rendered.contains("[transcript truncated: kept last 2 of 5 messages]"));
        assert!(rendered.contains("message 4"));
        assert!(rendered.contains("message 5"));
        assert!(!rendered.contains("message 3"));
    }

    #[test]
    fn finish_full_tail_has_no_truncation_header() {
        let out = finish(blocks(3), Some(10), "codex", Path::new("/x/r.jsonl"));
        let TranscriptOutcome::Ok { rendered } = out else {
            panic!("expected Ok");
        };
        assert!(rendered.starts_with("[transcript source: codex r.jsonl]"));
        assert!(!rendered.contains("truncated"));
    }

    #[test]
    fn finish_trims_from_front_on_byte_budget() {
        let big = "x".repeat(60 * 1024);
        let messages = vec![
            format!("[user] old {big}"),
            format!("[assistant] mid {big}"),
            "[user] newest".to_string(),
        ];
        let out = finish(messages, None, "kimi", Path::new("/x/wire.jsonl"));
        let TranscriptOutcome::Ok { rendered } = out else {
            panic!("expected Ok");
        };
        assert!(rendered.contains("[user] newest"));
        assert!(!rendered.contains("[user] old"));
        assert!(rendered.contains("[transcript truncated:"));
    }

    #[test]
    fn finish_empty_is_unavailable() {
        match finish(Vec::new(), None, "claude", Path::new("/x/a.jsonl")) {
            TranscriptOutcome::Unavailable(r) => assert!(r.contains("no renderable")),
            _ => panic!("expected Unavailable"),
        }
    }
}
