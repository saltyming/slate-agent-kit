//! What one attempt runs: the backend, its sandbox, isolation and output mode,
//! and the [`RunSpec`] a server builds per attempt.
//!
//! These are plain values; the server chooses them (its policy) and the crate
//! applies them. Argv construction from a `RunSpec` lives in `argv.rs`, the
//! capture and failure-text policies in `capture.rs` and `failure.rs`.

use std::path::PathBuf;

use crate::capture::CapturePolicy;
use crate::failure::FailureTextPolicy;

/// A backend CLI spawned as a process with the prompt on stdin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Backend {
    /// OpenAI's `codex` CLI.
    Codex,
    /// Anthropic's `claude` CLI (Claude Code).
    Claude,
}

impl Backend {
    /// The binary name looked up on `PATH`; also the backend's stable name in
    /// records and tool output.
    pub fn binary(&self) -> &'static str {
        match self {
            Backend::Codex => "codex",
            Backend::Claude => "claude",
        }
    }

    /// The backend's stable lowercase name (`codex`, `claude`), as recorded
    /// in `RunRecord::backend`; the same as [`Backend::binary`].
    pub fn as_str(&self) -> &'static str {
        self.binary()
    }

    /// Every process backend, in a stable order.
    pub fn all() -> &'static [Backend] {
        &[Backend::Codex, Backend::Claude]
    }
}

/// What the backend may do to the machine. Codex receives it as `-s`; Claude
/// as a permission mode. The ceiling that refuses `DangerFullAccess` belongs
/// to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sandbox {
    /// Reads only (codex `-s read-only`; claude `--permission-mode plan`).
    ReadOnly,
    /// Edits inside the working directory (codex `-s workspace-write`; claude
    /// `--permission-mode acceptEdits --allowedTools Bash`).
    WorkspaceWrite,
    /// No sandbox (codex `-s danger-full-access`; claude
    /// `--dangerously-skip-permissions`).
    DangerFullAccess,
}

impl Sandbox {
    /// The codex `-s` value: `read-only`, `workspace-write` or
    /// `danger-full-access`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Sandbox::ReadOnly => "read-only",
            Sandbox::WorkspaceWrite => "workspace-write",
            Sandbox::DangerFullAccess => "danger-full-access",
        }
    }

    /// Parse one of the three `as_str` values; anything else is `None`.
    pub fn parse(s: &str) -> Option<Sandbox> {
        match s {
            "read-only" => Some(Sandbox::ReadOnly),
            "workspace-write" => Some(Sandbox::WorkspaceWrite),
            "danger-full-access" => Some(Sandbox::DangerFullAccess),
            _ => None,
        }
    }
}

/// How a spawned backend is kept from reaching back into the server that
/// spawned it. The two codex variants apply to `Backend::Codex` only; the two
/// claude variants to `Backend::Claude` only. A variant of the other backend
/// adds no isolation flag (a claude run then gets the `Permissions` mapping).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Isolation {
    /// Codex `exec --ignore-user-config`: no user config, so no MCP server at
    /// all.
    IgnoreUserConfig,
    /// Codex `-c mcp_servers.<name>.enabled=false` before `exec`: that one MCP
    /// server disabled, every other server kept.
    DisableMcpServer(String),
    /// Claude `--safe-mode --no-session-persistence --permission-mode plan
    /// --tools <list>`, the list joined with `,`; the sandbox adds nothing.
    SafeMode {
        /// The built-in tools exposed, e.g. `Read`, `Grep`, `Glob`, `WebFetch`.
        tools: Vec<String>,
    },
    /// Claude: the sandbox's permission-mode mapping alone.
    Permissions,
}

/// Which output format flags the backend receives, and so what the record can
/// read from stdout. A mode that does not apply to the backend (`Text` or
/// `Json` for codex, `JsonStream` for claude) adds no flag and behaves as
/// `Default`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// No format flag; the backend's final message on stdout.
    Default,
    /// Claude `--input-format text --output-format text`.
    Text,
    /// Claude `--output-format json`: one result object, with usage.
    Json,
    /// Codex `exec --json`: a JSONL event stream, with usage.
    JsonStream,
}

/// A server's re-entry marker: the environment variable every spawned process
/// carries at the server's depth plus one, and the depth at or above which the
/// server refuses a request (see `reentry`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reentry {
    /// The environment variable, e.g. `ASIDE_REENTRY_DEPTH`.
    pub name: String,
    /// The refusal ceiling; `1` allows no nesting.
    pub ceiling: u32,
}

/// Whether a run is placed under the parent-death guard, so its process tree
/// dies with the server that started it.
///
/// On Linux and macOS the guard is the `agent-guard` executable
/// (`guard::locate`); on Windows it is a kill-on-close Job Object attached
/// inside the server process, and no executable is looked up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardMode {
    /// The run must be guarded: without `agent-guard` (or, on Windows, when
    /// the Job Object cannot be attached) the attempt is `Outcome::Spawn` and
    /// no backend is left running.
    Required,
    /// Guard the run when possible, else run it unguarded; `RunEvent::Started`
    /// says which.
    Preferred,
    /// Never guard; `agent-guard` is not looked up. Cancellation still ends
    /// the whole tree: through the process group on Unix, and on Windows
    /// through a Job Object that is not kill-on-close (so the tree does not
    /// die with the server).
    Off,
}

/// One attempt: everything `run` needs to spawn, capture and record it.
#[derive(Debug, Clone)]
pub struct RunSpec {
    /// The backend CLI.
    pub backend: Backend,
    /// The prompt, written to the process's stdin, which is then closed.
    pub prompt: String,
    /// The process directory; for codex also passed as `-C`. `None` inherits
    /// the server's directory and adds no `-C`.
    pub working_dir: Option<PathBuf>,
    /// The sandbox.
    pub sandbox: Sandbox,
    /// The anti-recursion isolation.
    pub isolation: Isolation,
    /// The output format flags.
    pub output_mode: OutputMode,
    /// How much of stdout and stderr is kept.
    pub capture: CapturePolicy,
    /// What the classifier sees when the attempt fails.
    pub failure_text: FailureTextPolicy,
    /// `-m` / `--model`; `None` uses the backend's default.
    pub model: Option<String>,
    /// Codex `-c model_reasoning_effort=`; claude `--effort`.
    pub reasoning_effort: Option<String>,
    /// Continue this backend session (codex `exec resume <id>`; claude
    /// `--resume <id> --fork-session`).
    pub resume_session: Option<String>,
    /// Claude only: pin the new session id with `--session-id`.
    pub pin_session: Option<String>,
    /// Codex only: `--skip-git-repo-check`.
    pub skip_git_repo_check: bool,
    /// Extra environment variables for the process.
    pub env: Vec<(String, String)>,
    /// The server's re-entry marker, stamped on the process.
    pub reentry: Reentry,
    /// Whether the run is placed under the parent-death guard.
    pub guard: GuardMode,
    /// The backend's `--version` as the server probed it with `version`;
    /// carried into `RunEvent::Started` and the record. `None` when not probed.
    pub backend_version: Option<String>,
}
