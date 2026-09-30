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
//! `-c mcp_servers.<name>.enabled=false` for `DisableMcpServer`; `exec`;
//! `--ignore-user-config` for `IgnoreUserConfig`; `resume <session>`;
//! `--skip-git-repo-check`; `--json` for `JsonStream`.
//!
//! Claude, in order: `-p`; the isolation and sandbox flags; the output-mode
//! format flags; `--model`; `--effort`; `--resume <session> --fork-session`;
//! `--session-id <id>`.

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
    if let Isolation::DisableMcpServer(name) = &spec.isolation {
        args.push("-c".into());
        args.push(format!("mcp_servers.{name}.enabled=false"));
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::capture::CapturePolicy;
    use crate::failure::FailureTextPolicy;
    use crate::spec::{GuardMode, Reentry};

    /// aside's claude attempt: safe mode, read-only tools, explicit text formats.
    fn aside_claude(model: Option<&str>, effort: Option<&str>) -> RunSpec {
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
                name: "ASIDE_REENTRY_DEPTH".into(),
                ceiling: 1,
            },
            guard: GuardMode::Required,
            backend_version: None,
        }
    }

    /// aside's codex attempt: read-only, no user config, prompt on stdin.
    fn aside_codex(model: Option<&str>, effort: Option<&str>) -> RunSpec {
        RunSpec {
            backend: Backend::Codex,
            isolation: Isolation::IgnoreUserConfig,
            output_mode: OutputMode::Default,
            ..aside_claude(model, effort)
        }
    }

    /// dispatch's codex/claude attempt: working root, the dispatch MCP server
    /// disabled for codex, the sandbox mapping for claude, no format flags.
    fn dispatch_spec(backend: Backend, sandbox: Sandbox) -> RunSpec {
        RunSpec {
            backend,
            prompt: "rendered spec".into(),
            working_dir: Some(PathBuf::from("/w")),
            sandbox,
            isolation: match backend {
                Backend::Codex => Isolation::DisableMcpServer("dispatch".into()),
                Backend::Claude => Isolation::Permissions,
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
    fn aside_claude_argv_is_exact() {
        assert_eq!(
            argv(&aside_claude(Some("sonnet"), Some("high"))),
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
            argv(&aside_claude(None, None)),
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
    fn aside_codex_argv_is_exact_without_positional_prompt() {
        let spec = aside_codex(Some("gpt-5.5"), Some("high"));
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
            argv(&aside_codex(None, None)),
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
        // aside stays enabled so dispatch → aside remains allowed.
        assert!(!argv(&spec).join(" ").contains("mcp_servers.aside"));
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

        let mut codex = aside_codex(None, None);
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

        // aside: no working_dir → no -C and no process directory
        let (cmd, _) = command(&aside_codex(None, None));
        assert_eq!(cmd.as_std().get_current_dir(), None);
    }
}
