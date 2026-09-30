//! Server registration and native configuration, shared across the three harnesses.
//!
//! Owns the per-server environment, the command lines of the harness CLIs, and
//! the types the install steps exchange. The per-harness behaviour lives in the
//! `claude`, `codex` and `kimi` submodules. It does not decide when to register;
//! `install` and `mcp` do.
//!
//! Main entry points: [`Ctx`], [`server_env`], [`shell_quote`], [`Outcome`],
//! [`SubagentPrefs`] and [`run_cli`].

pub mod claude;
pub mod codex;
pub mod kimi;

use crate::config::Change;
use crate::env::{Env, Harness};
use crate::error::{Error, Result};
use crate::manifest::Registration;
use std::path::{Path, PathBuf};
use std::process::Output;

/// Inputs shared by registration and native configuration.
#[derive(Debug, Clone)]
pub struct Ctx {
    /// Target harness.
    pub harness: Harness,
    /// Resolved harness home.
    pub home: PathBuf,
    /// The harness's default home (`~/.claude` and so on).
    pub default_home: PathBuf,
    /// Binary folder.
    pub bin_dir: PathBuf,
    /// Workspace roots, as OS path strings.
    pub roots: Vec<String>,
    /// Servers to register.
    pub servers: Vec<String>,
    /// Version written into the Kimi plugin manifest.
    pub version: String,
}

impl Ctx {
    /// True when the home is the harness's default location.
    pub fn is_default_home(&self) -> bool {
        crate::util::same_path(&self.home, &self.default_home)
    }

    /// The roots joined as an OS path list, or `None` when no roots were given.
    pub fn roots_value(&self) -> Option<String> {
        if self.roots.is_empty() {
            return None;
        }
        std::env::join_paths(&self.roots)
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    }

    /// The folder dispatch keeps its state in.
    pub fn state_home(&self) -> PathBuf {
        match self.harness {
            Harness::Claude => self.home.clone(),
            Harness::Codex | Harness::Kimi => self.home.join("slate-agent-kit"),
        }
    }
}

/// The environment variables a server is registered with.
pub fn server_env(ctx: &Ctx, server: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let kimi_custom = ctx.harness == Harness::Kimi && !ctx.is_default_home();
    match server {
        "aside" => {
            out.push(("ASIDE_HARNESS".to_string(), ctx.harness.name().to_string()));
            if kimi_custom {
                out.push((
                    "KIMI_CODE_HOME".to_string(),
                    ctx.home.to_string_lossy().into_owned(),
                ));
            }
        }
        "dispatch" => {
            out.push((
                "SLATE_AGENT_STATE_HOME".to_string(),
                ctx.state_home().to_string_lossy().into_owned(),
            ));
            if let Some(r) = ctx.roots_value() {
                out.push(("DISPATCH_EXTRA_ROOTS".to_string(), r));
            }
            if kimi_custom {
                out.push((
                    "KIMI_CODE_HOME".to_string(),
                    ctx.home.to_string_lossy().into_owned(),
                ));
            }
        }
        "palette" => {
            if let Some(r) = ctx.roots_value() {
                out.push(("PALETTE_EXTRA_ROOTS".to_string(), r));
            }
        }
        _ => {}
    }
    out
}

/// The prefix that sets `var` to `value` for one command, in the shell the platform's
/// users run: `VAR='value' ` in a POSIX shell, `$env:VAR='value'; ` in PowerShell.
pub fn env_prefix(platform: &str, var: &str, value: &str) -> String {
    if platform.contains("windows") {
        format!("$env:{var}='{}'; ", value.replace('\'', "''"))
    } else {
        format!("{var}={} ", shell_quote(value))
    }
}

/// Quotes `s` for display in the shell the platform's users run: PowerShell on
/// Windows, a POSIX shell elsewhere.
pub fn arg_quote(platform: &str, s: &str) -> String {
    if platform.contains("windows") {
        let plain = !s.is_empty()
            && s.chars().all(|c| {
                c.is_ascii_alphanumeric()
                    || matches!(
                        c,
                        '/' | '.' | '_' | '-' | '=' | ':' | ',' | '+' | '@' | '\\'
                    )
            });
        if plain {
            s.to_string()
        } else {
            format!("'{}'", s.replace('\'', "''"))
        }
    } else {
        shell_quote(s)
    }
}

/// Quotes `s` for display in a POSIX shell command line.
pub fn shell_quote(s: &str) -> String {
    let plain = !s.is_empty()
        && s.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(
                    c,
                    '/' | '.' | '_' | '-' | '=' | ':' | ',' | '+' | '@' | '\\'
                )
        });
    if plain {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// What a registration step did.
#[derive(Debug, Default)]
pub struct Outcome {
    /// Registrations made.
    pub registered: Vec<Registration>,
    /// Facts for the report.
    pub notes: Vec<String>,
    /// Commands the user has to run because the installer could not.
    pub manual: Vec<String>,
    /// Servers that ended up registered under a name the installer manages.
    pub servers_done: Vec<String>,
    /// Backups made while registering, as `(original, backup)`.
    pub backups: Vec<(PathBuf, PathBuf)>,
    /// A registration that failed after earlier ones succeeded; those stay recorded.
    pub failure: Option<Error>,
}

/// The subagent default from `subagent-prefs.md`; a blank field means "not set".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubagentPrefs {
    /// Default subagent model.
    pub model: String,
    /// Default subagent reasoning effort.
    pub effort: String,
}

/// The changes one file edit made, plus refusals that left parts of it untouched.
#[derive(Debug, Default)]
pub struct EditOutcome {
    /// Keys that changed.
    pub changes: Vec<Change>,
    /// Edits that were refused, each naming what to fix.
    pub refusals: Vec<Error>,
    /// Records of earlier edits that were undone (because the value is now unset).
    pub undone: Vec<Vec<String>>,
}

/// Runs a harness CLI and returns its output.
pub fn run_cli(
    env: &Env,
    program: &str,
    args: &[String],
    extra_env: &[(String, String)],
    remove_env: &[&str],
) -> Result<Output> {
    let mut cmd = env.command(program);
    cmd.args(args);
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    for k in remove_env {
        cmd.env_remove(k);
    }
    cmd.output().map_err(|e| {
        Error::command(format!("cannot run `{program}`: {e}"))
            .with_fix(format!("check that `{program}` is installed and on PATH"))
    })
}

/// The stderr of a failed command, trimmed to a readable size.
pub fn stderr_text(out: &Output) -> String {
    let text = String::from_utf8_lossy(&out.stderr);
    let text = text.trim();
    let mut s: String = text.chars().take(600).collect();
    if text.chars().count() > 600 {
        s.push_str("...");
    }
    s
}

/// The binary path of `server` in `bin_dir`.
pub fn server_binary(env: &Env, bin_dir: &Path, server: &str) -> PathBuf {
    crate::binaries::binary_path(env, bin_dir, server)
}

#[cfg(test)]
pub(crate) fn test_ctx(harness: Harness, home: &str, default_home: &str) -> Ctx {
    Ctx {
        harness,
        home: PathBuf::from(home),
        default_home: PathBuf::from(default_home),
        bin_dir: PathBuf::from("/bin"),
        roots: Vec::new(),
        servers: vec!["aside".into(), "dispatch".into(), "palette".into()],
        version: "13.0.0".into(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn arg_quote_follows_the_platform_shell() {
        assert_eq!(
            super::arg_quote("x86_64-unknown-linux-gnu", "it's"),
            "'it'\\''s'"
        );
        assert_eq!(
            super::arg_quote("x86_64-pc-windows-msvc", "C:\\a b"),
            "'C:\\a b'"
        );
        assert_eq!(
            super::arg_quote("x86_64-pc-windows-msvc", "C:\\O'Brien"),
            "'C:\\O''Brien'"
        );
        assert_eq!(
            super::arg_quote("x86_64-pc-windows-msvc", "C:\\plain"),
            "C:\\plain"
        );
    }

    #[test]
    fn env_prefix_follows_the_platform_shell() {
        assert_eq!(
            super::env_prefix("aarch64-apple-darwin", "V", "/a b"),
            "V='/a b' "
        );
        assert_eq!(
            super::env_prefix("x86_64-pc-windows-msvc", "V", "C:\\a's"),
            "$env:V='C:\\a''s'; "
        );
    }

    use super::*;

    fn get<'a>(env: &'a [(String, String)], k: &str) -> Option<&'a str> {
        env.iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn claude_environment_per_server() {
        let mut ctx = test_ctx(Harness::Claude, "/h/.claude", "/h/.claude");
        assert_eq!(
            server_env(&ctx, "aside"),
            vec![("ASIDE_HARNESS".to_string(), "claude".to_string())]
        );
        let d = server_env(&ctx, "dispatch");
        assert_eq!(get(&d, "SLATE_AGENT_STATE_HOME"), Some("/h/.claude"));
        assert_eq!(get(&d, "DISPATCH_EXTRA_ROOTS"), None);
        assert!(server_env(&ctx, "palette").is_empty());
        let sep = if cfg!(windows) { ";" } else { ":" };
        ctx.roots = vec!["/w/a".into(), "/w/b".into()];
        assert_eq!(
            get(&server_env(&ctx, "dispatch"), "DISPATCH_EXTRA_ROOTS"),
            Some(format!("/w/a{sep}/w/b").as_str())
        );
        assert_eq!(
            get(&server_env(&ctx, "palette"), "PALETTE_EXTRA_ROOTS"),
            Some(format!("/w/a{sep}/w/b").as_str())
        );
    }

    #[test]
    fn codex_and_kimi_keep_state_under_the_kit_folder() {
        let ctx = test_ctx(Harness::Codex, "/h/.codex", "/h/.codex");
        let d = server_env(&ctx, "dispatch");
        assert_eq!(
            Path::new(get(&d, "SLATE_AGENT_STATE_HOME").unwrap()),
            Path::new("/h/.codex").join("slate-agent-kit")
        );
        assert_eq!(get(&d, "KIMI_CODE_HOME"), None);
    }

    #[test]
    fn kimi_passes_its_home_only_when_it_is_not_the_default() {
        let ctx = test_ctx(Harness::Kimi, "/h/.kimi-code", "/h/.kimi-code");
        assert_eq!(get(&server_env(&ctx, "aside"), "KIMI_CODE_HOME"), None);
        assert_eq!(get(&server_env(&ctx, "dispatch"), "KIMI_CODE_HOME"), None);
        let ctx = test_ctx(Harness::Kimi, "/x/kimi", "/h/.kimi-code");
        assert_eq!(
            get(&server_env(&ctx, "aside"), "KIMI_CODE_HOME"),
            Some("/x/kimi")
        );
        assert_eq!(
            get(&server_env(&ctx, "dispatch"), "KIMI_CODE_HOME"),
            Some("/x/kimi")
        );
        assert_eq!(get(&server_env(&ctx, "palette"), "KIMI_CODE_HOME"), None);
    }

    #[test]
    fn quoting_for_display() {
        assert_eq!(shell_quote("/a/b-c_d.e"), "/a/b-c_d.e");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }
}
