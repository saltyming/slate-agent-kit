//! What one attempt runs: the backend, its sandbox, isolation and output mode,
//! and the [`RunSpec`] a server builds per attempt.
//!
//! These are plain values; the server chooses them (its policy) and the crate
//! applies them. What follows from the backend alone — how the prompt is
//! encoded on stdin ([`InputEncoding`]), how the outcome is decided
//! ([`ResultNormaliser`]) and which sandbox levels it refuses — is a method of
//! [`Backend`], not a choice of the server. Argv construction from a `RunSpec`
//! lives in `argv.rs`, the capture and failure-text policies in `capture.rs`
//! and `failure.rs`, the agy stream in `agy.rs`.

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
    /// Google's Antigravity CLI, `agy`: executions only, never a
    /// consultation; the prompt travels as a stream-json `user` event and the
    /// outcome is read from the stream's `result` event.
    Agy,
}

impl Backend {
    /// The binary name looked up on `PATH`; also the backend's stable name in
    /// records and tool output.
    pub fn binary(&self) -> &'static str {
        match self {
            Backend::Codex => "codex",
            Backend::Claude => "claude",
            Backend::Agy => "agy",
        }
    }

    /// The backend's stable lowercase name (`codex`, `claude`, `agy`), as
    /// recorded in `RunRecord::backend`; the same as [`Backend::binary`].
    pub fn as_str(&self) -> &'static str {
        self.binary()
    }

    /// Every process backend, in a stable order.
    pub fn all() -> &'static [Backend] {
        &[Backend::Codex, Backend::Claude, Backend::Agy]
    }

    /// How the prompt is written to the backend's stdin.
    pub fn input_encoding(&self) -> InputEncoding {
        match self {
            Backend::Codex | Backend::Claude => InputEncoding::Verbatim,
            Backend::Agy => InputEncoding::AgyStreamJson,
        }
    }

    /// How the outcome of a run of this backend is decided from its exit
    /// status and output.
    pub fn result_normaliser(&self) -> ResultNormaliser {
        match self {
            Backend::Codex | Backend::Claude => ResultNormaliser::ExitStatus,
            Backend::Agy => ResultNormaliser::AgyStreamJson,
        }
    }

    /// Why this backend cannot run under `sandbox`, or `None` when it can.
    /// agy refuses [`Sandbox::ReadOnly`]: a headless agy run is possible only
    /// with `--dangerously-skip-permissions` (it denies every unapproved tool
    /// otherwise), and nothing then holds it to reads. `run` returns the text
    /// as `Outcome::Spawn` without spawning; a server refuses the request
    /// with it before that.
    pub fn sandbox_refusal(&self, sandbox: Sandbox) -> Option<&'static str> {
        match (self, sandbox) {
            (Backend::Agy, Sandbox::ReadOnly) => Some(
                "agy cannot run an execution read-only: headless agy runs only under \
                 --dangerously-skip-permissions, which nothing then holds to reads; use \
                 workspace-write (agy --sandbox) or danger-full-access",
            ),
            _ => None,
        }
    }
}

/// How a run's prompt reaches the backend: the bytes written to stdin, which
/// is then closed. The prompt never travels on argv.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEncoding {
    /// The prompt's UTF-8 bytes as they are (codex, claude).
    Verbatim,
    /// One stream-json line, `{"event":"user","message":{"role":"user",
    /// "content":<prompt as a JSON string>}}`, and a newline (agy
    /// `--input-format stream-json`).
    AgyStreamJson,
}

impl InputEncoding {
    /// The bytes written to stdin for `prompt`.
    pub fn encode<'a>(&self, prompt: &'a str) -> std::borrow::Cow<'a, [u8]> {
        match self {
            InputEncoding::Verbatim => std::borrow::Cow::Borrowed(prompt.as_bytes()),
            InputEncoding::AgyStreamJson => {
                std::borrow::Cow::Owned(crate::agy::user_event_line(prompt).into_bytes())
            }
        }
    }
}

/// How the outcome of a run is decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultNormaliser {
    /// Success is exit status 0; `final_text` is the retained stdout (or, for
    /// a structured output mode, the message parsed from it); usage comes
    /// from the structured output modes (codex, claude).
    ExitStatus,
    /// stdout is an NDJSON event stream read as it is drained (agy
    /// `--output-format stream-json`): the `init` event's `conversation_id`
    /// is the session, reported at once as `RunEvent::Session`; `step_update`
    /// text deltas are the retained capture; the `result` event (kept whole
    /// up to 8 MB) decides success, `final_text`, the failure detail and the
    /// usage (`UsageSource::AgyStreamJson`). Success needs exit 0 and a well-formed `result` with `status`
    /// `SUCCESS`, no `error`, and not `denied_actions` with an empty
    /// `response`.
    AgyStreamJson,
}

/// What the backend may do to the machine. Codex receives it as `-s`; Claude
/// as a permission mode; agy as `--sandbox` or nothing, refusing `ReadOnly`
/// ([`Backend::sandbox_refusal`]). The ceiling that refuses `DangerFullAccess`
/// belongs to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sandbox {
    /// Reads only (codex `-s read-only`; claude `--permission-mode plan`;
    /// refused for agy).
    ReadOnly,
    /// Edits inside the working directory (codex `-s workspace-write`; claude
    /// `--permission-mode acceptEdits --allowedTools Bash`; agy `--sandbox`).
    WorkspaceWrite,
    /// No sandbox (codex `-s danger-full-access`; claude
    /// `--dangerously-skip-permissions`; agy without `--sandbox`).
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
/// spawned it. The three codex variants apply to `Backend::Codex` only; the two
/// claude variants to `Backend::Claude` only. A variant of the other backend
/// adds no isolation flag (a claude run then gets the `Permissions` mapping).
/// agy takes no isolation flag at all: it passes its process environment to
/// the MCP servers it starts, so the re-entry marker reaches them, and a
/// server gives it `Permissions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Isolation {
    /// Codex `exec --ignore-user-config`: no user config, so no MCP server at
    /// all.
    IgnoreUserConfig,
    /// Codex `-c mcp_servers.<name>.enabled=false` before `exec`: that one MCP
    /// server disabled, every other server kept.
    DisableMcpServer(String),
    /// Codex `-c mcp_servers.<server>.command="<command>"` (when `command` is
    /// set) and `-c mcp_servers.<server>.env.<var>="<value>"` before `exec`:
    /// every MCP server kept, and the server `server` started with `var` set
    /// to `value` in its environment. Codex starts its MCP servers with a
    /// clean environment, so this is how a re-entry marker reaches a
    /// codex-booted copy of the spawning server. The values are passed as TOML
    /// strings (codex rejects an integer there). An override of `env` alone
    /// creates a server entry with no transport when the user's configuration
    /// does not register `server`, and codex then refuses to load its
    /// configuration; setting `command` makes the entry complete either way,
    /// and merges with a registered entry's other keys. Each tool in
    /// `approve_tools` then gets `-c mcp_servers.<server>.tools.<tool>.
    /// approval_mode="approve"`: under `-a never` codex refuses an MCP tool
    /// call that needs approval, so only the listed tools can be called, and
    /// a tool left out is refused by codex before it reaches the server.
    ServerEnv {
        /// The MCP server's name in the codex configuration, e.g. `dispatch`.
        server: String,
        /// The environment variable, e.g. `DISPATCH_REENTRY_DEPTH`.
        var: String,
        /// The variable's value, e.g. `1`.
        value: String,
        /// The server's executable, e.g. the absolute path of the running
        /// server; `None` leaves the configured command in place.
        command: Option<String>,
        /// Tools of `server` pre-approved on the command line, rendered in
        /// this order after the env key.
        approve_tools: Vec<String>,
    },
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
    /// The environment variable, e.g. `DISPATCH_REENTRY_DEPTH`.
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
    /// The prompt, written to the process's stdin in the backend's
    /// [`InputEncoding`], after which stdin is closed.
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
    /// Codex `-c model_reasoning_effort=`; claude and agy `--effort` (agy:
    /// not passed when the model slug already ends in an effort).
    pub reasoning_effort: Option<String>,
    /// Continue this backend session (codex `exec resume <id>`; claude
    /// `--resume <id> --fork-session`; agy `--conversation <id>`).
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
