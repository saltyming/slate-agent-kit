//! The process environment as the installer sees it, and the harness identity.
//!
//! Owns an injectable snapshot of environment variables, the user's home
//! folder and the extra variables passed to child processes, so library code
//! never reads process-global state and tests can run in parallel. Also owns
//! [`Harness`], the per-harness names and default locations. It does not run
//! commands beyond building them.
//!
//! Main entry points: [`Env::from_process`], [`Env::for_home`], [`Env::command`],
//! [`Env::which`] and [`Harness`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// One of the three harnesses a kit can target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Harness {
    /// Claude Code.
    Claude,
    /// Codex.
    Codex,
    /// Kimi Code.
    Kimi,
}

impl Harness {
    /// Every harness, in a fixed order.
    pub const ALL: [Harness; 3] = [Harness::Claude, Harness::Codex, Harness::Kimi];

    /// Parses `claude`, `codex` or `kimi`.
    pub fn parse(s: &str) -> Option<Harness> {
        match s {
            "claude" => Some(Harness::Claude),
            "codex" => Some(Harness::Codex),
            "kimi" => Some(Harness::Kimi),
            _ => None,
        }
    }

    /// The name used in descriptors and in `ASIDE_HARNESS`.
    pub fn name(self) -> &'static str {
        match self {
            Harness::Claude => "claude",
            Harness::Codex => "codex",
            Harness::Kimi => "kimi",
        }
    }

    /// Human-readable product name.
    pub fn product(self) -> &'static str {
        match self {
            Harness::Claude => "Claude Code",
            Harness::Codex => "Codex",
            Harness::Kimi => "Kimi Code",
        }
    }

    /// Environment variable that relocates the harness home.
    pub fn home_var(self) -> &'static str {
        match self {
            Harness::Claude => "CLAUDE_CONFIG_DIR",
            Harness::Codex => "CODEX_HOME",
            Harness::Kimi => "KIMI_CODE_HOME",
        }
    }

    /// Folder name of the default home under the user's home folder.
    pub fn default_dir_name(self) -> &'static str {
        match self {
            Harness::Claude => ".claude",
            Harness::Codex => ".codex",
            Harness::Kimi => ".kimi-code",
        }
    }

    /// File name of the primary instruction file in the home.
    pub fn primary_file(self) -> &'static str {
        match self {
            Harness::Claude => "CLAUDE.md",
            Harness::Codex | Harness::Kimi => "AGENTS.md",
        }
    }

    /// Environment variable naming the harness CLI, when the harness has one.
    pub fn cli_var(self) -> Option<&'static str> {
        match self {
            Harness::Claude => Some("CLAUDE_BIN"),
            Harness::Codex => Some("CODEX_BIN"),
            Harness::Kimi => None,
        }
    }

    /// Default command name of the harness CLI, when the harness has one.
    pub fn cli_default(self) -> Option<&'static str> {
        match self {
            Harness::Claude => Some("claude"),
            Harness::Codex => Some("codex"),
            Harness::Kimi => None,
        }
    }
}

/// Snapshot of the environment the installer runs in.
#[derive(Debug, Clone, Default)]
pub struct Env {
    vars: BTreeMap<String, String>,
    /// The user's home folder.
    pub home: PathBuf,
    /// Extra variables set on every child process (tests use this for stand-in CLIs).
    pub child_env: BTreeMap<String, String>,
}

impl Env {
    /// Builds the snapshot from the running process.
    pub fn from_process() -> Env {
        let vars: BTreeMap<String, String> = std::env::vars().collect();
        let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let home = vars
            .get(home_var)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(std::env::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        Env {
            vars,
            home,
            child_env: BTreeMap::new(),
        }
    }

    /// Builds an environment with an explicit home folder and no other variables.
    pub fn for_home(home: impl Into<PathBuf>) -> Env {
        Env {
            vars: BTreeMap::new(),
            home: home.into(),
            child_env: BTreeMap::new(),
        }
    }

    /// Sets a variable in the snapshot.
    pub fn set_var(&mut self, key: &str, value: impl Into<String>) {
        self.vars.insert(key.to_string(), value.into());
    }

    /// Returns a variable when it is set and not empty.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars
            .get(key)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }

    /// The default home of `harness`, ignoring its override variable.
    pub fn default_home(&self, harness: Harness) -> PathBuf {
        self.home.join(harness.default_dir_name())
    }

    /// The home of `harness`: its override variable, else the default folder.
    pub fn harness_home(&self, harness: Harness) -> PathBuf {
        match self.get(harness.home_var()) {
            Some(v) => PathBuf::from(v),
            None => self.default_home(harness),
        }
    }

    /// The default binary folder, `~/.local/bin`.
    pub fn default_bin_dir(&self) -> PathBuf {
        self.home.join(".local").join("bin")
    }

    /// The program used to run the harness CLI (`CLAUDE_BIN`, `CODEX_BIN` or the default name).
    pub fn cli_program(&self, harness: Harness) -> Option<String> {
        let default = harness.cli_default()?;
        let var = harness.cli_var()?;
        Some(self.get(var).unwrap_or(default).to_string())
    }

    /// The release target the installer downloads binaries for.
    pub fn platform(&self) -> String {
        self.get("SLATE_PLATFORM")
            .unwrap_or(env!("SLATE_SETUP_TARGET"))
            .to_string()
    }

    /// Creates a command with no standard input and the configured child variables.
    pub fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut cmd = Command::new(program);
        cmd.stdin(Stdio::null());
        cmd.envs(&self.child_env);
        cmd
    }

    /// Finds `program` on `PATH` (or checks it directly when it contains a separator).
    pub fn which(&self, program: &str) -> Option<PathBuf> {
        let candidate = Path::new(program);
        if candidate.components().count() > 1 || candidate.is_absolute() {
            return executable_variants(candidate, self)
                .into_iter()
                .find(|p| p.is_file());
        }
        let path = self.get("PATH")?;
        for dir in std::env::split_paths(path) {
            for full in executable_variants(&dir.join(program), self) {
                if full.is_file() {
                    return Some(full);
                }
            }
        }
        None
    }
}

fn executable_variants(base: &Path, env: &Env) -> Vec<PathBuf> {
    let mut out = vec![base.to_path_buf()];
    if cfg!(windows) {
        let exts = env.get("PATHEXT").unwrap_or(".COM;.EXE;.BAT;.CMD");
        for ext in exts.split(';').filter(|e| !e.is_empty()) {
            let mut name = base.as_os_str().to_os_string();
            name.push(ext);
            out.push(PathBuf::from(name));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_home_prefers_override_variable() {
        let mut env = Env::for_home("/h");
        assert_eq!(
            env.harness_home(Harness::Codex),
            Path::new("/h").join(".codex")
        );
        env.set_var("CODEX_HOME", "/elsewhere");
        assert_eq!(
            env.harness_home(Harness::Codex),
            PathBuf::from("/elsewhere")
        );
        assert_eq!(
            env.default_home(Harness::Codex),
            Path::new("/h").join(".codex")
        );
    }

    #[test]
    fn empty_variables_count_as_unset() {
        let mut env = Env::for_home("/h");
        env.set_var("KIMI_CODE_HOME", "");
        assert_eq!(
            env.harness_home(Harness::Kimi),
            Path::new("/h").join(".kimi-code")
        );
    }

    #[test]
    fn cli_program_uses_override() {
        let mut env = Env::for_home("/h");
        assert_eq!(env.cli_program(Harness::Claude).as_deref(), Some("claude"));
        env.set_var("CLAUDE_BIN", "/x/fake");
        assert_eq!(env.cli_program(Harness::Claude).as_deref(), Some("/x/fake"));
        assert_eq!(env.cli_program(Harness::Kimi), None);
    }

    #[test]
    fn which_finds_files_on_path() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir
            .path()
            .join(if cfg!(windows) { "tool.exe" } else { "tool" });
        std::fs::write(&exe, "").unwrap();
        let mut env = Env::for_home("/h");
        env.set_var("PATH", dir.path().to_string_lossy());
        assert_eq!(env.which("tool"), Some(exe));
        assert_eq!(env.which("absent"), None);
    }
}
