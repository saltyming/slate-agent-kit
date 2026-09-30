//! Finding a backend CLI: `PATH` lookup, the `--version` probe and the install
//! hint shown when the binary is missing.

use std::path::PathBuf;
use std::process::Stdio;

use tokio::process::Command;

use crate::reentry;
use crate::spec::{Backend, Reentry};

/// The first file named `binary` on `PATH`, or `binary.exe` on Windows. It
/// does not test executability and does not find `.cmd` shims.
pub fn which(binary: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(binary);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let exe = dir.join(format!("{binary}.exe"));
            if exe.is_file() {
                return Some(exe);
            }
        }
    }
    None
}

/// Run `<binary> --version` with the server's re-entry marker stamped and
/// return its trimmed stdout, else its trimmed stderr; `None` when the binary
/// is missing, fails to run or prints nothing. The probe is short-lived and is
/// not guarded.
pub async fn version(backend: Backend, reentry: &Reentry) -> Option<String> {
    version_of(backend.binary(), reentry).await
}

/// [`version`] for a CLI named by its binary, for a caller that also reports
/// a CLI this crate does not run as a [`Backend`].
pub async fn version_of(binary: &str, reentry: &Reentry) -> Option<String> {
    let _ = which(binary)?;
    let mut cmd = Command::new(binary);
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    reentry::stamp(&mut cmd, &reentry.name);
    let output = cmd.output().await.ok()?;
    let out = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !out.is_empty() {
        return Some(out);
    }
    let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if err.is_empty() { None } else { Some(err) }
}

/// One line telling how to install the backend's CLI.
pub fn install_hint(backend: Backend) -> String {
    match backend {
        Backend::Codex => {
            "install codex CLI (`npm i -g @openai/codex`; see https://github.com/openai/codex)"
                .to_string()
        }
        Backend::Claude => "install Claude Code CLI (`npm i -g @anthropic-ai/claude-code`; see \
             https://claude.com/claude-code)"
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_hints_name_their_cli() {
        assert!(install_hint(Backend::Codex).contains("@openai/codex"));
        assert!(install_hint(Backend::Claude).contains("@anthropic-ai/claude-code"));
    }

    #[test]
    fn which_misses_a_name_nothing_provides() {
        assert_eq!(which("agent-exec-no-such-binary-on-any-path"), None);
    }
}
