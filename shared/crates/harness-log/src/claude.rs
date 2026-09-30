//! Claude Code session file location.
//!
//! Claude Code persists each session as
//! `~/.claude/projects/<dashed-working-dir>/<session-uuid>.jsonl`, appended
//! live. A caller that pins the session id at spawn (`--session-id`) knows the
//! file name in advance; [`claude_session_path`] predicts the directory from
//! the working directory and [`find_claude_session`] finds the file by name
//! when the prediction misses. Reading the file's contents is the caller's
//! business.

use std::path::{Path, PathBuf};

/// The session log path for a pinned claude session in `working_dir`.
/// `working_dir` must already be canonical. Claude's slug is *approximately*
/// the canonical cwd with path separators (and, on Windows, the drive colon)
/// replaced by `-` — but the exact formula has edge cases (a leading-dot path
/// component is observed to slug as `-`, not `.`), so callers must be prepared
/// to fall back to [`find_claude_session`] when this path does not exist.
pub fn claude_session_path(working_dir: &Path, sid: &str) -> PathBuf {
    let slug = working_dir.to_string_lossy().replace(['/', '\\', ':'], "-");
    claude_projects_root()
        .join(slug)
        .join(format!("{sid}.jsonl"))
}

/// `~/.claude/projects` (`HOME`, else `USERPROFILE`).
pub fn claude_projects_root() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    PathBuf::from(home).join(".claude").join("projects")
}

/// Locate a pinned claude session by scanning `~/.claude/projects/*/<sid>.jsonl`.
/// The sid is a caller-pinned UUID, so a filename match is positive identity
/// regardless of which slug directory Claude chose for the working dir.
pub fn find_claude_session(sid: &str) -> Option<PathBuf> {
    find_claude_session_in(&claude_projects_root(), sid)
}

fn find_claude_session_in(root: &Path, sid: &str) -> Option<PathBuf> {
    if sid.is_empty() {
        return None;
    }
    let name = format!("{sid}.jsonl");
    let rd = std::fs::read_dir(root).ok()?;
    for e in rd.flatten() {
        let candidate = e.path().join(&name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "harness-log-claude-test-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn claude_session_path_is_deterministic_slug() {
        // Compare path components, not a rendered string — the string form is
        // separator-dependent (`\` on Windows) while the components are not.
        let p = claude_session_path(Path::new("/w/proj"), "sid-1");
        let tail: Vec<_> = p.iter().rev().take(4).collect();
        assert_eq!(tail[0], "sid-1.jsonl", "{}", p.display());
        assert_eq!(tail[1], "-w-proj", "{}", p.display());
        assert_eq!(tail[2], "projects", "{}", p.display());
        assert_eq!(tail[3], ".claude", "{}", p.display());

        // Windows-style cwd: backslashes and the drive colon slug to `-`.
        let w = claude_session_path(Path::new(r"C:\w\proj"), "sid-2");
        let wtail: Vec<_> = w.iter().rev().take(2).collect();
        assert_eq!(wtail[0], "sid-2.jsonl", "{}", w.display());
        assert_eq!(wtail[1], "C--w-proj", "{}", w.display());
    }

    #[test]
    fn find_claude_session_scans_project_dirs_by_pinned_sid() {
        let root = test_root("find");
        // Claude chose a slug the formula can't predict (leading-dot mangling).
        let dir = root.join("-w--hidden-proj");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pinned-sid.jsonl"), "{}\n").unwrap();
        assert_eq!(
            find_claude_session_in(&root, "pinned-sid"),
            Some(dir.join("pinned-sid.jsonl"))
        );
        assert_eq!(find_claude_session_in(&root, "absent-sid"), None);
        assert_eq!(find_claude_session_in(&root, ""), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
