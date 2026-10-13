//! The coding-agent backends dispatch delegates to, and dispatch's spawn policy
//! for an execution by the two that run as processes.
//!
//! An execution runs the backend **write-capable**: codex executes in
//! `-s workspace-write` and may modify files in the target directory. codex and
//! claude are started through the `agent-exec` crate, which owns lookup, argv,
//! spawning under the parent-death guard, capture and the fallback loop; this
//! module only chooses the [`agent_exec::RunSpec`] values of an execution
//! attempt ([`run_spec`]); a consultation's are in `consult.rs`. opencode is
//! driven over HTTP by `opencode.rs`.
//!
//! Anti-recursion: every process dispatch spawns carries [`REENTRY_ENV`] at
//! this server's depth plus one ([`next_depth`]). A dispatch server at depth
//! [`REENTRY_CEILING`] or more refuses `dispatch_submit` and `dispatch_steer`
//! and accepts `dispatch_consult`, so a backend may consult but never start
//! another execution. **claude** and **opencode** pass their process env to
//! the MCP servers they boot, so the inherited marker reaches a booted
//! dispatch. **codex** launches MCP servers with a clean env, so an execution's
//! codex receives the marker on its command line as the environment of the
//! `dispatch` server (`Isolation::ServerEnv`,
//! `-c mcp_servers.dispatch.env.DISPATCH_REENTRY_DEPTH="<depth>"`); the server
//! stays enabled so the backend can consult. The same override also sets
//! `mcp_servers.dispatch.command` to this running binary, because codex
//! refuses to load a configuration whose `dispatch` entry has an `env` key but
//! no transport, which is what the env override alone creates when the user's
//! `config.toml` does not register dispatch; with a registered entry, the
//! command override merges with its other keys. The codex backend runs with
//! `-a never`, under which codex refuses an MCP call that needs approval, so the
//! override also pre-approves the tools a backend may use
//! ([`BACKEND_APPROVED_TOOLS`]: `dispatch_consult` and the read tools) with
//! `-c mcp_servers.dispatch.tools.<tool>.approval_mode="approve"`.
//! `dispatch_submit` and `dispatch_steer` stay unapproved, so codex refuses them
//! before they reach the server; the server's per-kind re-entry refusal is the
//! backstop. A consultation backend has no MCP server at all (codex
//! `--ignore-user-config`, claude safe mode).

use std::path::Path;

use agent_exec::{
    CapturePolicy, FailureTextPolicy, GuardMode, Isolation, OutputMode, Reentry, RunSpec, Sandbox,
};

/// Env var marking that this dispatch server is running inside a
/// dispatch-spawned backend. A top-level call has it unset (depth 0).
pub(crate) const REENTRY_ENV: &str = "DISPATCH_REENTRY_DEPTH";

/// Depth at or above which a submit/steer is refused. `1` = no nesting.
pub(crate) const REENTRY_CEILING: u32 = 1;

/// The dispatch tools a codex execution backend may call, pre-approved on its
/// command line in this order: the consultation and the read tools. Executions
/// (`dispatch_submit`, `dispatch_steer`) are left out.
pub(crate) const BACKEND_APPROVED_TOOLS: [&str; 5] = [
    "dispatch_consult",
    "dispatch_status",
    "dispatch_list",
    "dispatch_logs",
    "dispatch_backends",
];

/// The depth every process this server spawns carries: the server's own
/// depth plus one, saturating.
pub(crate) fn next_depth() -> u32 {
    agent_exec::reentry::depth(REENTRY_ENV).saturating_add(1)
}

/// dispatch's re-entry marker, as stamped on every spawned process.
pub(crate) fn reentry() -> Reentry {
    Reentry {
        name: REENTRY_ENV.to_string(),
        ceiling: REENTRY_CEILING,
    }
}

/// Which coding-agent CLI we delegate to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Codex,
    Opencode,
    Claude,
}

impl Backend {
    pub fn binary(&self) -> &'static str {
        match self {
            Backend::Codex => "codex",
            Backend::Opencode => "opencode",
            Backend::Claude => "claude",
        }
    }

    /// Stable string used in params, the DB `backend` column, and tool output.
    pub fn as_str(&self) -> &'static str {
        self.binary()
    }

    /// Parse the `backend` param. Empty/absent defaults to Codex.
    pub fn parse(s: &str) -> Option<Backend> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "codex" => Some(Backend::Codex),
            "opencode" => Some(Backend::Opencode),
            "claude" => Some(Backend::Claude),
            _ => None,
        }
    }

    pub fn all() -> &'static [Backend] {
        &[Backend::Codex, Backend::Opencode, Backend::Claude]
    }

    /// The `agent-exec` process backend, or `None` for opencode (driven over
    /// HTTP by `opencode.rs`).
    pub fn process(&self) -> Option<agent_exec::Backend> {
        match self {
            Backend::Codex => Some(agent_exec::Backend::Codex),
            Backend::Claude => Some(agent_exec::Backend::Claude),
            Backend::Opencode => None,
        }
    }
}

/// Ask the backend CLI for its `--version` string, with the re-entry marker
/// stamped. Returns `None` if missing.
pub async fn version(backend: Backend) -> Option<String> {
    match backend.process() {
        Some(b) => agent_exec::version(b, &reentry()).await,
        None => crate::opencode::version(&reentry()).await,
    }
}

/// What one codex or claude attempt of a job runs with, besides the backend.
pub struct AttemptSpec<'a> {
    /// The canonical working directory: the process directory and codex `-C`.
    pub working_dir: &'a Path,
    /// The job's sandbox string (`read-only`, `workspace-write`,
    /// `danger-full-access`); the danger ceiling was applied at submit.
    pub sandbox: &'a str,
    /// The attempt's model; `None` is the backend default.
    pub model: Option<&'a str>,
    /// Codex `model_reasoning_effort`; claude `--effort`.
    pub reasoning_effort: Option<&'a str>,
    /// Codex `--skip-git-repo-check`.
    pub skip_git_repo_check: bool,
    /// Continue this backend session (`dispatch_steer`): codex
    /// `exec resume <id>`, claude `--resume <id> --fork-session`. The
    /// accumulated conversation context is preserved.
    pub resume_session: Option<&'a str>,
    /// Claude only: the attempt's pinned session id (`--session-id`), so its
    /// session log path under `~/.claude/projects/` is known in advance.
    pub pin_session: Option<&'a str>,
    /// The backend's `--version`, probed at boot, recorded with the run.
    pub backend_version: Option<&'a str>,
}

/// dispatch's `RunSpec` for one codex or claude execution attempt: codex with
/// the re-entry marker set in the `dispatch` MCP server's environment, claude
/// with the sandbox's permission mapping
/// (read-only → plan; workspace-write → acceptEdits + Bash; danger-full-access →
/// skip permissions), no output-format flags, dispatch's capture and
/// failure-text policies, and the guard required. The prompt travels on stdin.
///
/// A sandbox string outside the three known values runs as workspace-write, the
/// default dispatch applies at submit.
pub fn run_spec(backend: agent_exec::Backend, a: &AttemptSpec<'_>, prompt: String) -> RunSpec {
    let isolation = match backend {
        agent_exec::Backend::Codex => Isolation::ServerEnv {
            server: "dispatch".to_string(),
            var: REENTRY_ENV.to_string(),
            value: next_depth().to_string(),
            command: Some(crate::current_exe_path()),
            approve_tools: BACKEND_APPROVED_TOOLS.map(str::to_string).to_vec(),
        },
        agent_exec::Backend::Claude => Isolation::Permissions,
    };
    RunSpec {
        backend,
        prompt,
        working_dir: Some(a.working_dir.to_path_buf()),
        sandbox: Sandbox::parse(a.sandbox).unwrap_or(Sandbox::WorkspaceWrite),
        isolation,
        output_mode: OutputMode::Default,
        capture: CapturePolicy::dispatch(),
        failure_text: FailureTextPolicy::dispatch(),
        model: a.model.map(str::to_string),
        reasoning_effort: a.reasoning_effort.map(str::to_string),
        resume_session: a.resume_session.map(str::to_string),
        pin_session: match backend {
            agent_exec::Backend::Claude => a.pin_session.map(str::to_string),
            agent_exec::Backend::Codex => None,
        },
        skip_git_repo_check: a.skip_git_repo_check,
        env: Vec::new(),
        reentry: reentry(),
        guard: GuardMode::Required,
        backend_version: a.backend_version.map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_backends() {
        assert_eq!(Backend::parse(""), Some(Backend::Codex));
        assert_eq!(Backend::parse("codex"), Some(Backend::Codex));
        assert_eq!(Backend::parse("opencode"), Some(Backend::Opencode));
        assert_eq!(Backend::parse("claude"), Some(Backend::Claude));
        assert_eq!(Backend::parse("missing"), None);
        assert!(Backend::all().contains(&Backend::Codex));
        assert!(Backend::all().contains(&Backend::Opencode));
        assert!(Backend::all().contains(&Backend::Claude));
    }

    fn attempt<'a>(
        sandbox: &'a str,
        model: Option<&'a str>,
        effort: Option<&'a str>,
        resume: Option<&'a str>,
        pin: Option<&'a str>,
        skip_git: bool,
    ) -> AttemptSpec<'a> {
        AttemptSpec {
            working_dir: Path::new("/w"),
            sandbox,
            model,
            reasoning_effort: effort,
            skip_git_repo_check: skip_git,
            resume_session: resume,
            pin_session: pin,
            backend_version: Some("1.0"),
        }
    }

    /// The argv `agent-exec` builds for a spec, binary first.
    fn argv(backend: agent_exec::Backend, a: &AttemptSpec<'_>) -> Vec<String> {
        agent_exec::command(&run_spec(backend, a, "prompt".into())).1
    }

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// `-C` carries the working dir as rendered by the platform; compare it
    /// through the same conversion rather than a separator-dependent literal.
    fn dir() -> String {
        Path::new("/w").to_string_lossy().into_owned()
    }

    /// The `-c` override that carries the marker into a codex-booted dispatch.
    fn marker_override() -> String {
        format!(
            "mcp_servers.dispatch.env.DISPATCH_REENTRY_DEPTH=\"{}\"",
            next_depth()
        )
    }

    /// The `-c` override that names this binary as the dispatch server's
    /// command, quoted as a TOML string.
    fn command_override() -> String {
        let exe = crate::current_exe_path()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        format!("mcp_servers.dispatch.command=\"{exe}\"")
    }

    #[test]
    fn codex_argv_passes_the_marker_to_the_dispatch_server_before_exec() {
        let bare = argv(
            agent_exec::Backend::Codex,
            &attempt("workspace-write", None, None, None, None, false),
        );
        assert_eq!(
            bare,
            strs(&[
                "codex",
                "-C",
                &dir(),
                "-s",
                "workspace-write",
                "-a",
                "never",
                "-c",
                &command_override(),
                "-c",
                &marker_override(),
                "-c",
                "mcp_servers.dispatch.tools.dispatch_consult.approval_mode=\"approve\"",
                "-c",
                "mcp_servers.dispatch.tools.dispatch_status.approval_mode=\"approve\"",
                "-c",
                "mcp_servers.dispatch.tools.dispatch_list.approval_mode=\"approve\"",
                "-c",
                "mcp_servers.dispatch.tools.dispatch_logs.approval_mode=\"approve\"",
                "-c",
                "mcp_servers.dispatch.tools.dispatch_backends.approval_mode=\"approve\"",
                "exec",
            ])
        );
        // Executions stay unapproved, so codex refuses them under `-a never`.
        for tool in ["dispatch_submit", "dispatch_steer"] {
            assert!(!bare.iter().any(|a| a.contains(&format!(".tools.{tool}."))));
        }
        // The dispatch server stays enabled so the backend can consult.
        assert!(!bare.iter().any(|a| a.ends_with(".enabled=false")));

        let full = argv(
            agent_exec::Backend::Codex,
            &attempt(
                "read-only",
                Some("gpt-x"),
                Some("high"),
                Some("sid-1"),
                // A pin is claude-only: codex never receives it.
                Some("ignored"),
                true,
            ),
        );
        assert_eq!(
            full,
            strs(&[
                "codex",
                "-C",
                &dir(),
                "-s",
                "read-only",
                "-a",
                "never",
                "-m",
                "gpt-x",
                "-c",
                "model_reasoning_effort=high",
                "-c",
                &command_override(),
                "-c",
                &marker_override(),
                "-c",
                "mcp_servers.dispatch.tools.dispatch_consult.approval_mode=\"approve\"",
                "-c",
                "mcp_servers.dispatch.tools.dispatch_status.approval_mode=\"approve\"",
                "-c",
                "mcp_servers.dispatch.tools.dispatch_list.approval_mode=\"approve\"",
                "-c",
                "mcp_servers.dispatch.tools.dispatch_logs.approval_mode=\"approve\"",
                "-c",
                "mcp_servers.dispatch.tools.dispatch_backends.approval_mode=\"approve\"",
                "exec",
                "resume",
                "sid-1",
                "--skip-git-repo-check",
            ])
        );
    }

    #[test]
    fn claude_argv_maps_sandbox_and_pins_session() {
        let ww = argv(
            agent_exec::Backend::Claude,
            &attempt(
                "workspace-write",
                Some("haiku"),
                Some("high"),
                None,
                Some("uuid-1"),
                false,
            ),
        );
        assert_eq!(
            ww,
            strs(&[
                "claude",
                "-p",
                "--permission-mode",
                "acceptEdits",
                "--allowedTools",
                "Bash",
                "--model",
                "haiku",
                "--effort",
                "high",
                "--session-id",
                "uuid-1",
            ])
        );

        let ro = argv(
            agent_exec::Backend::Claude,
            &attempt("read-only", None, None, None, Some("u"), false),
        );
        assert_eq!(
            ro,
            strs(&[
                "claude",
                "-p",
                "--permission-mode",
                "plan",
                "--session-id",
                "u"
            ])
        );

        let danger = argv(
            agent_exec::Backend::Claude,
            &attempt("danger-full-access", None, None, None, Some("u"), false),
        );
        assert_eq!(
            danger,
            strs(&[
                "claude",
                "-p",
                "--dangerously-skip-permissions",
                "--session-id",
                "u"
            ])
        );

        let steer = argv(
            agent_exec::Backend::Claude,
            &attempt(
                "workspace-write",
                None,
                None,
                Some("old-sid"),
                Some("new-sid"),
                true,
            ),
        );
        assert_eq!(
            steer,
            strs(&[
                "claude",
                "-p",
                "--permission-mode",
                "acceptEdits",
                "--allowedTools",
                "Bash",
                "--resume",
                "old-sid",
                "--fork-session",
                "--session-id",
                "new-sid",
            ])
        );
        for a in [&ww, &ro, &danger, &steer] {
            assert!(
                !a.iter()
                    .any(|x| x == "--input-format" || x == "--output-format")
            );
            assert!(!a.iter().any(|x| x == "--skip-git-repo-check"));
        }
    }

    #[test]
    fn run_spec_carries_dispatch_policy() {
        let s = run_spec(
            agent_exec::Backend::Codex,
            &attempt("danger-full-access", None, None, None, None, false),
            "p".into(),
        );
        assert_eq!(s.sandbox, Sandbox::DangerFullAccess);
        assert_eq!(s.output_mode, OutputMode::Default);
        assert_eq!(s.capture, CapturePolicy::dispatch());
        assert_eq!(s.failure_text, FailureTextPolicy::dispatch());
        assert_eq!(s.guard, GuardMode::Required);
        assert_eq!(s.reentry, reentry());
        assert_eq!(s.reentry.name, "DISPATCH_REENTRY_DEPTH");
        assert_eq!(s.reentry.ceiling, 1);
        assert_eq!(s.working_dir.as_deref(), Some(Path::new("/w")));
        assert_eq!(s.backend_version.as_deref(), Some("1.0"));
        assert_eq!(s.prompt, "p");

        let odd = run_spec(
            agent_exec::Backend::Claude,
            &attempt("unknown", None, None, None, None, false),
            "p".into(),
        );
        assert_eq!(odd.sandbox, Sandbox::WorkspaceWrite);
        assert_eq!(odd.isolation, Isolation::Permissions);

        assert_eq!(
            s.isolation,
            Isolation::ServerEnv {
                server: "dispatch".into(),
                var: "DISPATCH_REENTRY_DEPTH".into(),
                value: next_depth().to_string(),
                command: Some(crate::current_exe_path()),
                approve_tools: BACKEND_APPROVED_TOOLS.map(str::to_string).to_vec(),
            }
        );
        assert_eq!(next_depth(), agent_exec::reentry::depth(REENTRY_ENV) + 1);
    }
}
