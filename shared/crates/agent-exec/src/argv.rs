//! Argv construction: the one place a `RunSpec` becomes a backend command
//! line.
//!
//! Entry point: [`command`], which returns the ready `Command` (arguments,
//! directory, extra environment, re-entry marker; stdio is configured by
//! `run`) and the argv vector recorded for audit. The prompt is never on argv:
//! it travels on stdin, which avoids OS argv-length and quoting limits. A
//! server never appends to what it gets back.
//!
//! Codex, in order: `-C <dir>` when set; `-s <sandbox>`; `-a never`;
//! `-m <model>`; `-c model_reasoning_effort=<effort>`;
//! `-c mcp_servers.<name>.enabled=false` for `DisableMcpServer`;
//! `-c mcp_servers.<server>.command="<command>"` (when set) and
//! `-c mcp_servers.<server>.env.<var>="<value>"` and
//! `-c mcp_servers.<server>.tools.<tool>.approval_mode="approve"` per approved
//! tool for `ServerEnv`; `exec`;
//! `--ignore-user-config` for `IgnoreUserConfig`; `resume <session>`;
//! `--skip-git-repo-check`; `--json` for `JsonStream`.
//!
//! Claude, in order: `-p`; the isolation and sandbox flags; the output-mode
//! format flags; `--model`; `--effort`; `--resume <session> --fork-session`;
//! `--session-id <id>`.
//!
//! agy, in order: `--dangerously-skip-permissions --input-format stream-json
//! --output-format stream-json --print-timeout 0`; `--sandbox` for
//! workspace-write; `--model`; `--effort` unless the model slug already ends
//! in an effort; `--conversation <session>` when resuming. The output mode and
//! the isolation add nothing; the read-only level is refused by `run` before
//! any spawn.

use tokio::process::Command;

use crate::reentry;
use crate::spec::{Backend, Isolation, OutputMode, RunSpec, Sandbox};

/// Build the backend command for `spec` and the argv it runs (binary name
/// first). The `Command` has its arguments, its directory (`working_dir`), the
/// extra environment and the re-entry marker set.
pub fn command(spec: &RunSpec) -> (Command, Vec<String>) {
    let args = match spec.backend {
        Backend::Codex => codex_args(spec),
        Backend::Claude => claude_args(spec),
        Backend::Agy => agy_args(spec),
    };
    let binary = spec.backend.binary();
    let mut cmd = Command::new(binary);
    cmd.args(&args);
    if let Some(dir) = &spec.working_dir {
        cmd.current_dir(dir);
    }
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    reentry::stamp(&mut cmd, &spec.reentry.name);
    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push(binary.to_string());
    argv.extend(args);
    (cmd, argv)
}

fn codex_args(spec: &RunSpec) -> Vec<String> {
    //   -C <dir>:     working root, set explicitly (and recorded in argv) to match
    //                 the process directory, so a rollout's cwd is unambiguous.
    //   -s <sandbox>: read-only allows reads only; workspace-write lets codex edit
    //                 files under the working root; danger-full-access drops it.
    //   -a never:     non-interactive — never pause for an approval prompt.
    //   -c ...:       top-level TOML config overrides, so they precede `exec`.
    //   mcp_servers.<name>.enabled=false: disable one MCP server in the spawned
    //                 codex. codex does NOT pass its process env to the MCP
    //                 servers it boots, so the re-entry marker never reaches a
    //                 codex-booted server; this override is the fail-closed guard.
    //   mcp_servers.<server>.env.<var>="<value>": keep the server and set one
    //                 variable in its environment table instead, so the
    //                 re-entry marker does reach the codex-booted server. The
    //                 value is a TOML string; codex rejects an integer there.
    //   mcp_servers.<server>.command="<command>": set alongside, so the entry
    //                 has a transport even when config.toml does not register
    //                 the server (an env-only entry fails codex's config load
    //                 with "invalid transport").
    //   mcp_servers.<server>.tools.<tool>.approval_mode="approve": under
    //                 `-a never` an MCP call needing approval is refused, so
    //                 these name the only tools of that server the backend can
    //                 call, whatever config.toml says.
    //   exec:         non-interactive subcommand. With no positional PROMPT, codex
    //                 reads the instructions from stdin.
    //   --ignore-user-config: do not load ~/.codex/config.toml, so the spawned
    //                 codex carries no MCP server at all. Auth still resolves from
    //                 CODEX_HOME. An `exec` flag, so it comes after `exec`.
    //   resume <sid>: continue a prior session, preserving its context.
    //   --skip-git-repo-check: permit a working root that is not a git repo.
    //   --json:       JSONL events on stdout, including per-turn token usage.
    let mut args: Vec<String> = Vec::new();
    if let Some(dir) = &spec.working_dir {
        args.push("-C".into());
        args.push(dir.to_string_lossy().into_owned());
    }
    args.push("-s".into());
    args.push(spec.sandbox.as_str().into());
    args.push("-a".into());
    args.push("never".into());
    if let Some(m) = &spec.model {
        args.push("-m".into());
        args.push(m.clone());
    }
    if let Some(eff) = &spec.reasoning_effort {
        args.push("-c".into());
        args.push(format!("model_reasoning_effort={eff}"));
    }
    match &spec.isolation {
        Isolation::DisableMcpServer(name) => {
            args.push("-c".into());
            args.push(format!("mcp_servers.{name}.enabled=false"));
        }
        Isolation::ServerEnv {
            server,
            var,
            value,
            command,
            approve_tools,
        } => {
            if let Some(command) = command {
                args.push("-c".into());
                args.push(format!(
                    "mcp_servers.{server}.command={}",
                    toml_string(command)
                ));
            }
            args.push("-c".into());
            args.push(format!(
                "mcp_servers.{server}.env.{var}={}",
                toml_string(value)
            ));
            for tool in approve_tools {
                args.push("-c".into());
                args.push(format!(
                    "mcp_servers.{server}.tools.{tool}.approval_mode={}",
                    toml_string("approve")
                ));
            }
        }
        _ => {}
    }
    args.push("exec".into());
    if spec.isolation == Isolation::IgnoreUserConfig {
        args.push("--ignore-user-config".into());
    }
    if let Some(sid) = &spec.resume_session {
        args.push("resume".into());
        args.push(sid.clone());
    }
    if spec.skip_git_repo_check {
        args.push("--skip-git-repo-check".into());
    }
    if spec.output_mode == OutputMode::JsonStream {
        args.push("--json".into());
    }
    args
}

/// `value` as a TOML basic string: double-quoted, with `\`, `"` and control
/// characters escaped.
fn toml_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn claude_args(spec: &RunSpec) -> Vec<String> {
    //   -p:                       print the response and exit (non-interactive).
    //   SafeMode:                 --safe-mode disables project/user customizations,
    //                             hooks, plugins, MCP servers and CLAUDE.md discovery;
    //                             --no-session-persistence writes no session to disk;
    //                             --permission-mode plan is read-only; --tools exposes
    //                             only the listed built-in tools.
    //   Permissions, by sandbox (Claude Code has no OS-level sandbox):
    //     read-only          → --permission-mode plan
    //     workspace-write    → --permission-mode acceptEdits + Bash allowlisted:
    //                          edits inside the working dir auto-accepted, edits
    //                          outside it prompt → auto-denied headless
    //     danger-full-access → --dangerously-skip-permissions
    //   --resume <sid> --fork-session --session-id <new>: a resumed run continues
    //                             the parent conversation under a new pinned id
    //                             (--session-id alongside --resume requires
    //                             --fork-session).
    let mut args: Vec<String> = vec!["-p".into()];
    match &spec.isolation {
        Isolation::SafeMode { tools } => {
            args.push("--safe-mode".into());
            args.push("--no-session-persistence".into());
            args.push("--permission-mode".into());
            args.push("plan".into());
            args.push("--tools".into());
            args.push(tools.join(","));
        }
        _ => match spec.sandbox {
            Sandbox::ReadOnly => {
                args.push("--permission-mode".into());
                args.push("plan".into());
            }
            Sandbox::WorkspaceWrite => {
                args.push("--permission-mode".into());
                args.push("acceptEdits".into());
                args.push("--allowedTools".into());
                args.push("Bash".into());
            }
            Sandbox::DangerFullAccess => {
                args.push("--dangerously-skip-permissions".into());
            }
        },
    }
    match spec.output_mode {
        OutputMode::Text => {
            args.push("--input-format".into());
            args.push("text".into());
            args.push("--output-format".into());
            args.push("text".into());
        }
        OutputMode::Json => {
            args.push("--output-format".into());
            args.push("json".into());
        }
        OutputMode::Default | OutputMode::JsonStream => {}
    }
    if let Some(m) = &spec.model {
        args.push("--model".into());
        args.push(m.clone());
    }
    if let Some(eff) = &spec.reasoning_effort {
        args.push("--effort".into());
        args.push(eff.clone());
    }
    if let Some(sid) = &spec.resume_session {
        args.push("--resume".into());
        args.push(sid.clone());
        args.push("--fork-session".into());
    }
    if let Some(pin) = &spec.pin_session {
        args.push("--session-id".into());
        args.push(pin.clone());
    }
    args
}

fn agy_args(spec: &RunSpec) -> Vec<String> {
    //   --dangerously-skip-permissions: headless agy cannot ask for a
    //                 permission and auto-denies every unapproved tool, so an
    //                 execution runs with all tools approved.
    //   --input-format stream-json: the prompt arrives on stdin as one `user`
    //                 event (never on argv: a long prompt passes the Windows
    //                 command-line limit); requires stream-json output.
    //   --output-format stream-json: NDJSON events (init, step_update,
    //                 result), read by the agy result normaliser.
    //   --print-timeout 0: wait for the turn however long it takes.
    //   --sandbox:    agy's terminal sandbox, for workspace-write. ReadOnly is
    //                 refused before the spawn (`Backend::sandbox_refusal`); its
    //                 argv here is the most restrictive one, never run.
    //   --effort <e>: left out when the model slug already names an effort
    //                 (`claude-sonnet-5-5-low`), which agy takes as is.
    //   --conversation <id>: resume that conversation with the new prompt.
    let mut args: Vec<String> = [
        "--dangerously-skip-permissions",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--print-timeout",
        "0",
    ]
    .iter()
    .map(|a| a.to_string())
    .collect();
    if spec.sandbox != Sandbox::DangerFullAccess {
        args.push("--sandbox".into());
    }
    if let Some(m) = &spec.model {
        args.push("--model".into());
        args.push(m.clone());
    }
    if let Some(eff) = &spec.reasoning_effort
        && !spec.model.as_deref().is_some_and(slug_names_effort)
    {
        args.push("--effort".into());
        args.push(eff.clone());
    }
    if let Some(id) = &spec.resume_session {
        args.push("--conversation".into());
        args.push(id.clone());
    }
    args
}

/// Whether an agy model slug ends in an effort level (`-low`, `-medium`,
/// `-high`, `-xhigh`, `-max`), as the `agy models` list names them.
fn slug_names_effort(model: &str) -> bool {
    model
        .rsplit_once('-')
        .is_some_and(|(_, last)| matches!(last, "low" | "medium" | "high" | "xhigh" | "max"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::capture::CapturePolicy;
    use crate::failure::FailureTextPolicy;
    use crate::spec::{GuardMode, Reentry};

    /// A consultation's claude attempt: safe mode, read-only tools, explicit
    /// text formats.
    fn consult_claude(model: Option<&str>, effort: Option<&str>) -> RunSpec {
        RunSpec {
            backend: Backend::Claude,
            prompt: "prompt body".into(),
            working_dir: None,
            sandbox: Sandbox::ReadOnly,
            isolation: Isolation::SafeMode {
                tools: vec![
                    "Read".into(),
                    "Grep".into(),
                    "Glob".into(),
                    "WebFetch".into(),
                ],
            },
            output_mode: OutputMode::Text,
            capture: CapturePolicy::aside(),
            failure_text: FailureTextPolicy::aside(),
            model: model.map(str::to_string),
            reasoning_effort: effort.map(str::to_string),
            resume_session: None,
            pin_session: None,
            skip_git_repo_check: false,
            env: Vec::new(),
            reentry: Reentry {
                name: "DISPATCH_REENTRY_DEPTH".into(),
                ceiling: 1,
            },
            guard: GuardMode::Required,
            backend_version: None,
        }
    }

    /// A consultation's codex attempt: read-only, no user config, prompt on
    /// stdin.
    fn consult_codex(model: Option<&str>, effort: Option<&str>) -> RunSpec {
        RunSpec {
            backend: Backend::Codex,
            isolation: Isolation::IgnoreUserConfig,
            output_mode: OutputMode::Default,
            ..consult_claude(model, effort)
        }
    }

    /// An execution's codex/claude attempt: working root, the dispatch MCP
    /// server disabled for codex, the sandbox mapping for claude, no format
    /// flags.
    fn dispatch_spec(backend: Backend, sandbox: Sandbox) -> RunSpec {
        RunSpec {
            backend,
            prompt: "rendered spec".into(),
            working_dir: Some(PathBuf::from("/w")),
            sandbox,
            isolation: match backend {
                Backend::Codex => Isolation::DisableMcpServer("dispatch".into()),
                Backend::Claude | Backend::Agy => Isolation::Permissions,
            },
            output_mode: OutputMode::Default,
            capture: CapturePolicy::dispatch(),
            failure_text: FailureTextPolicy::dispatch(),
            model: None,
            reasoning_effort: None,
            resume_session: None,
            pin_session: None,
            skip_git_repo_check: false,
            env: Vec::new(),
            reentry: Reentry {
                name: "DISPATCH_REENTRY_DEPTH".into(),
                ceiling: 1,
            },
            guard: GuardMode::Required,
            backend_version: None,
        }
    }

    fn argv(spec: &RunSpec) -> Vec<String> {
        command(spec).1
    }

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn consult_claude_argv_is_exact() {
        assert_eq!(
            argv(&consult_claude(Some("sonnet"), Some("high"))),
            v(&[
                "claude",
                "-p",
                "--safe-mode",
                "--no-session-persistence",
                "--permission-mode",
                "plan",
                "--tools",
                "Read,Grep,Glob,WebFetch",
                "--input-format",
                "text",
                "--output-format",
                "text",
                "--model",
                "sonnet",
                "--effort",
                "high",
            ])
        );
        assert_eq!(
            argv(&consult_claude(None, None)),
            v(&[
                "claude",
                "-p",
                "--safe-mode",
                "--no-session-persistence",
                "--permission-mode",
                "plan",
                "--tools",
                "Read,Grep,Glob,WebFetch",
                "--input-format",
                "text",
                "--output-format",
                "text",
            ])
        );
    }

    #[test]
    fn consult_codex_argv_is_exact_without_positional_prompt() {
        let spec = consult_codex(Some("gpt-5.5"), Some("high"));
        let got = argv(&spec);
        assert_eq!(
            got,
            v(&[
                "codex",
                "-s",
                "read-only",
                "-a",
                "never",
                "-m",
                "gpt-5.5",
                "-c",
                "model_reasoning_effort=high",
                "exec",
                "--ignore-user-config",
            ])
        );
        assert!(!got.iter().any(|a| a.contains("prompt body")));
        assert_eq!(
            argv(&consult_codex(None, None)),
            v(&[
                "codex",
                "-s",
                "read-only",
                "-a",
                "never",
                "exec",
                "--ignore-user-config"
            ])
        );
    }

    #[test]
    fn dispatch_codex_argv_is_exact() {
        let mut spec = dispatch_spec(Backend::Codex, Sandbox::WorkspaceWrite);
        let w = PathBuf::from("/w").to_string_lossy().into_owned();
        assert_eq!(
            argv(&spec),
            v(&[
                "codex",
                "-C",
                &w,
                "-s",
                "workspace-write",
                "-a",
                "never",
                "-c",
                "mcp_servers.dispatch.enabled=false",
                "exec",
            ])
        );

        spec.model = Some("gpt-5.5".into());
        spec.reasoning_effort = Some("medium".into());
        spec.resume_session = Some("sid-1".into());
        spec.skip_git_repo_check = true;
        spec.sandbox = Sandbox::ReadOnly;
        assert_eq!(
            argv(&spec),
            v(&[
                "codex",
                "-C",
                &w,
                "-s",
                "read-only",
                "-a",
                "never",
                "-m",
                "gpt-5.5",
                "-c",
                "model_reasoning_effort=medium",
                "-c",
                "mcp_servers.dispatch.enabled=false",
                "exec",
                "resume",
                "sid-1",
                "--skip-git-repo-check",
            ])
        );

        spec.sandbox = Sandbox::DangerFullAccess;
        spec.resume_session = None;
        spec.skip_git_repo_check = false;
        spec.model = None;
        spec.reasoning_effort = None;
        assert_eq!(
            argv(&spec),
            v(&[
                "codex",
                "-C",
                &w,
                "-s",
                "danger-full-access",
                "-a",
                "never",
                "-c",
                "mcp_servers.dispatch.enabled=false",
                "exec",
            ])
        );
        // Only the named server is disabled; no other server is touched.
        assert_eq!(
            argv(&spec)
                .iter()
                .filter(|a| a.starts_with("mcp_servers."))
                .count(),
            1
        );
    }

    #[test]
    fn server_env_sets_one_variable_of_one_server_before_exec() {
        let mut spec = dispatch_spec(Backend::Codex, Sandbox::WorkspaceWrite);
        spec.isolation = Isolation::ServerEnv {
            server: "dispatch".into(),
            var: "DISPATCH_REENTRY_DEPTH".into(),
            value: "1".into(),
            command: Some(r"C:\bin\dispatch".into()),
            approve_tools: vec!["dispatch_consult".into(), "dispatch_status".into()],
        };
        spec.model = Some("gpt-5.5".into());
        spec.resume_session = Some("sid-1".into());
        let w = PathBuf::from("/w").to_string_lossy().into_owned();
        assert_eq!(
            argv(&spec),
            v(&[
                "codex",
                "-C",
                &w,
                "-s",
                "workspace-write",
                "-a",
                "never",
                "-m",
                "gpt-5.5",
                "-c",
                r#"mcp_servers.dispatch.command="C:\\bin\\dispatch""#,
                "-c",
                "mcp_servers.dispatch.env.DISPATCH_REENTRY_DEPTH=\"1\"",
                "-c",
                "mcp_servers.dispatch.tools.dispatch_consult.approval_mode=\"approve\"",
                "-c",
                "mcp_servers.dispatch.tools.dispatch_status.approval_mode=\"approve\"",
                "exec",
                "resume",
                "sid-1",
            ])
        );
        assert!(!argv(&spec).iter().any(|a| a.ends_with(".enabled=false")));

        // Without a command or approvals, the configured ones stay: only the
        // env key.
        if let Isolation::ServerEnv {
            command,
            approve_tools,
            ..
        } = &mut spec.isolation
        {
            *command = None;
            approve_tools.clear();
        }
        let got = argv(&spec);
        assert!(!got.iter().any(|a| a.contains(".command=")));
        assert!(!got.iter().any(|a| a.contains(".approval_mode=")));
        assert!(got.contains(&"mcp_servers.dispatch.env.DISPATCH_REENTRY_DEPTH=\"1\"".to_string()));

        // A claude run takes no codex isolation flag.
        spec.backend = Backend::Claude;
        spec.resume_session = None;
        spec.model = None;
        assert_eq!(
            argv(&spec),
            v(&[
                "claude",
                "-p",
                "--permission-mode",
                "acceptEdits",
                "--allowedTools",
                "Bash"
            ])
        );
    }

    #[test]
    fn toml_string_quotes_and_escapes() {
        assert_eq!(toml_string("1"), "\"1\"");
        assert_eq!(toml_string(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(toml_string("x\ny"), "\"x\\ny\"");
    }

    #[test]
    fn dispatch_claude_argv_maps_sandbox_and_pins_session() {
        let mut spec = dispatch_spec(Backend::Claude, Sandbox::WorkspaceWrite);
        spec.model = Some("haiku".into());
        spec.reasoning_effort = Some("high".into());
        spec.pin_session = Some("uuid-1".into());
        assert_eq!(
            argv(&spec),
            v(&[
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

        spec.sandbox = Sandbox::ReadOnly;
        spec.model = None;
        spec.reasoning_effort = None;
        spec.pin_session = Some("u".into());
        assert_eq!(
            argv(&spec),
            v(&[
                "claude",
                "-p",
                "--permission-mode",
                "plan",
                "--session-id",
                "u"
            ])
        );

        spec.sandbox = Sandbox::DangerFullAccess;
        assert_eq!(
            argv(&spec),
            v(&[
                "claude",
                "-p",
                "--dangerously-skip-permissions",
                "--session-id",
                "u"
            ])
        );

        // steer: resume the parent session under a new pinned id
        spec.sandbox = Sandbox::WorkspaceWrite;
        spec.resume_session = Some("old-sid".into());
        spec.pin_session = Some("new-sid".into());
        assert_eq!(
            argv(&spec),
            v(&[
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
    }

    #[test]
    fn output_modes_add_their_flags_only_to_their_backend() {
        let mut claude = dispatch_spec(Backend::Claude, Sandbox::ReadOnly);
        claude.output_mode = OutputMode::Json;
        assert_eq!(
            argv(&claude),
            v(&[
                "claude",
                "-p",
                "--permission-mode",
                "plan",
                "--output-format",
                "json"
            ])
        );
        claude.output_mode = OutputMode::JsonStream;
        assert_eq!(
            argv(&claude),
            v(&["claude", "-p", "--permission-mode", "plan"])
        );

        let mut codex = consult_codex(None, None);
        codex.output_mode = OutputMode::JsonStream;
        codex.resume_session = Some("s".into());
        assert_eq!(
            argv(&codex),
            v(&[
                "codex",
                "-s",
                "read-only",
                "-a",
                "never",
                "exec",
                "--ignore-user-config",
                "resume",
                "s",
                "--json",
            ])
        );
        codex.output_mode = OutputMode::Text;
        assert!(
            !argv(&codex)
                .iter()
                .any(|a| a.starts_with("--") && a != "--ignore-user-config")
        );
    }

    #[test]
    fn command_matches_argv_and_carries_dir_env_and_marker() {
        let mut spec = dispatch_spec(Backend::Codex, Sandbox::WorkspaceWrite);
        spec.env = vec![("EXTRA_VAR".into(), "1".into())];
        let (cmd, argv) = command(&spec);
        let std = cmd.as_std();
        assert_eq!(std.get_program(), "codex");
        let args: Vec<String> = std
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, argv[1..].to_vec());
        assert_eq!(std.get_current_dir(), Some(PathBuf::from("/w").as_path()));
        let envs: Vec<(String, Option<String>)> = std
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert!(envs.contains(&("EXTRA_VAR".into(), Some("1".into()))));
        assert!(
            envs.iter()
                .any(|(k, v)| k == "DISPATCH_REENTRY_DEPTH" && v.is_some())
        );

        // A consultation: no working_dir → no -C and no process directory
        let (cmd, _) = command(&consult_codex(None, None));
        assert_eq!(cmd.as_std().get_current_dir(), None);
    }
    const AGY_BASE: [&str; 8] = [
        "agy",
        "--dangerously-skip-permissions",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--print-timeout",
        "0",
    ];

    fn agy(rest: &[&str]) -> Vec<String> {
        v(&AGY_BASE.iter().chain(rest).copied().collect::<Vec<_>>())
    }

    #[test]
    fn agy_argv_per_sandbox_level() {
        let mut spec = dispatch_spec(Backend::Agy, Sandbox::WorkspaceWrite);
        assert_eq!(argv(&spec), agy(&["--sandbox"]));
        spec.sandbox = Sandbox::DangerFullAccess;
        assert_eq!(argv(&spec), agy(&[]));
        // Never spawned (`run` refuses it), and the most restrictive if it were.
        spec.sandbox = Sandbox::ReadOnly;
        assert_eq!(argv(&spec), agy(&["--sandbox"]));
        assert!(Backend::Agy.sandbox_refusal(Sandbox::ReadOnly).is_some());
        assert_eq!(Backend::Agy.sandbox_refusal(Sandbox::WorkspaceWrite), None);
        assert_eq!(
            Backend::Agy.sandbox_refusal(Sandbox::DangerFullAccess),
            None
        );
        for b in [Backend::Codex, Backend::Claude] {
            assert_eq!(b.sandbox_refusal(Sandbox::ReadOnly), None);
        }
        // The output mode and every isolation variant add nothing.
        spec.sandbox = Sandbox::WorkspaceWrite;
        spec.output_mode = OutputMode::Json;
        spec.isolation = Isolation::DisableMcpServer("dispatch".into());
        assert_eq!(argv(&spec), agy(&["--sandbox"]));
        // No positional prompt: it travels on stdin.
        assert!(!argv(&spec).iter().any(|a| a.contains("rendered spec")));
        // The process directory is the working directory.
        let (cmd, _) = command(&spec);
        assert_eq!(
            cmd.as_std().get_current_dir(),
            Some(PathBuf::from("/w").as_path())
        );
    }

    #[test]
    fn agy_argv_model_effort_and_steer() {
        let mut spec = dispatch_spec(Backend::Agy, Sandbox::WorkspaceWrite);
        spec.model = Some("claude-sonnet-5-5".into());
        spec.reasoning_effort = Some("low".into());
        assert_eq!(
            argv(&spec),
            agy(&[
                "--sandbox",
                "--model",
                "claude-sonnet-5-5",
                "--effort",
                "low"
            ])
        );
        // A slug that carries its effort is passed as is, with no --effort.
        for slug in [
            "claude-sonnet-5-5-low",
            "gpt-oss-120b-medium",
            "gemini-3.8-flash-high",
        ] {
            spec.model = Some(slug.into());
            assert_eq!(argv(&spec), agy(&["--sandbox", "--model", slug]), "{slug}");
        }
        spec.model = None;
        assert_eq!(argv(&spec), agy(&["--sandbox", "--effort", "low"]));

        // steer: resume the stored conversation; the new instruction goes on
        // stdin.
        spec.reasoning_effort = None;
        spec.resume_session = Some("e195bb9c-conv".into());
        spec.pin_session = Some("ignored".into());
        spec.skip_git_repo_check = true;
        assert_eq!(
            argv(&spec),
            agy(&["--sandbox", "--conversation", "e195bb9c-conv"])
        );
    }

    #[test]
    fn effort_suffixes_are_recognised() {
        for yes in ["a-low", "a-medium", "a-high", "a-xhigh", "a-max"] {
            assert!(slug_names_effort(yes), "{yes}");
        }
        for no in ["gemini-3.8-flash", "claude-opus-5-5", "low", "a-lowest", ""] {
            assert!(!slug_names_effort(no), "{no}");
        }
    }
}
