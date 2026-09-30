//! aside's execution policy: the `RunSpec` of one advisor attempt and the
//! re-entry marker it carries.
//!
//! Both backends run read-only and isolated from the caller's MCP servers:
//! codex under `-s read-only` with `exec --ignore-user-config` (no user config,
//! so no MCP server at all), claude in safe mode with `--permission-mode plan`
//! and only the built-in read/search/fetch tools. The prompt travels on stdin.
//! Spawning, capture and the fallback loop belong to `agent_exec`; this module
//! only chooses what an attempt runs.

use agent_exec::{
    Backend, CapturePolicy, FailureTextPolicy, GuardMode, Isolation, OutputMode, Reentry, RunSpec,
    Sandbox,
};

/// The environment variable marking how deep a process is inside
/// aside-spawned backends. A top-level harness call has it unset.
pub const REENTRY_MARKER: &str = "ASIDE_REENTRY_DEPTH";

/// Depth at or above which a call is refused. `1` allows no nesting: a
/// top-level call (depth 0) proceeds; anything aside itself spawned is refused.
pub const REENTRY_CEILING: u32 = 1;

/// The built-in claude tools an advisor run may use.
const CLAUDE_TOOLS: [&str; 4] = ["Read", "Grep", "Glob", "WebFetch"];

/// aside's re-entry marker, stamped on every process it spawns.
pub fn reentry() -> Reentry {
    Reentry {
        name: REENTRY_MARKER.to_string(),
        ceiling: REENTRY_CEILING,
    }
}

/// The `RunSpec` of one advisor attempt of `backend` with `prompt`, run with
/// `model` and `reasoning_effort` (`None` leaves each to the backend's
/// default). The backend runs in the server's directory, under the guard when
/// `agent-guard` is found.
pub fn attempt(
    backend: Backend,
    prompt: &str,
    model: Option<&str>,
    reasoning_effort: Option<&str>,
) -> RunSpec {
    let (isolation, output_mode) = match backend {
        Backend::Codex => (Isolation::IgnoreUserConfig, OutputMode::Default),
        Backend::Claude => (
            Isolation::SafeMode {
                tools: CLAUDE_TOOLS.iter().map(|t| t.to_string()).collect(),
            },
            OutputMode::Text,
        ),
    };
    RunSpec {
        backend,
        prompt: prompt.to_string(),
        working_dir: None,
        sandbox: Sandbox::ReadOnly,
        isolation,
        output_mode,
        capture: CapturePolicy::aside(),
        failure_text: FailureTextPolicy::aside(),
        model: model.map(str::to_string),
        reasoning_effort: reasoning_effort.map(str::to_string),
        resume_session: None,
        pin_session: None,
        skip_git_repo_check: false,
        env: Vec::new(),
        reentry: reentry(),
        guard: GuardMode::Preferred,
        backend_version: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(spec: &RunSpec) -> Vec<String> {
        agent_exec::command(spec).1
    }

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn claude_command_is_read_only_with_model_and_effort() {
        let spec = attempt(Backend::Claude, "prompt body", Some("sonnet"), Some("high"));
        assert_eq!(
            argv(&spec),
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
    }

    #[test]
    fn claude_command_without_model_or_effort() {
        let spec = attempt(Backend::Claude, "prompt body", None, None);
        assert_eq!(
            argv(&spec),
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
    fn codex_disables_user_config_after_exec() {
        let spec = attempt(Backend::Codex, "prompt body", Some("gpt-5.5"), Some("high"));
        assert_eq!(
            argv(&spec),
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
    }

    #[test]
    fn codex_command_without_model_or_effort() {
        let spec = attempt(Backend::Codex, "prompt body", None, None);
        assert_eq!(
            argv(&spec),
            v(&[
                "codex",
                "-s",
                "read-only",
                "-a",
                "never",
                "exec",
                "--ignore-user-config",
            ])
        );
    }

    #[test]
    fn no_backend_takes_the_prompt_as_a_positional_argument() {
        for backend in Backend::all() {
            let spec = attempt(*backend, "prompt body", Some("m"), Some("high"));
            let (cmd, argv) = agent_exec::command(&spec);
            assert!(
                !argv.iter().any(|a| a.contains("prompt body")),
                "{backend:?}: {argv:?}"
            );
            assert!(
                !cmd.as_std()
                    .get_args()
                    .any(|a| a.to_string_lossy().contains("prompt body")),
                "{backend:?}: prompt must travel on stdin"
            );
            assert_eq!(spec.prompt, "prompt body");
        }
    }

    #[test]
    fn attempts_carry_the_aside_policy() {
        for backend in Backend::all() {
            let spec = attempt(*backend, "p", None, None);
            assert_eq!(spec.sandbox, Sandbox::ReadOnly);
            assert_eq!(spec.working_dir, None);
            assert_eq!(spec.capture, CapturePolicy::aside());
            assert_eq!(spec.failure_text, FailureTextPolicy::aside());
            assert_eq!(spec.guard, GuardMode::Preferred);
            assert!(spec.env.is_empty());
            assert_eq!(
                spec.reentry,
                Reentry {
                    name: "ASIDE_REENTRY_DEPTH".into(),
                    ceiling: 1
                }
            );
        }
    }

    #[test]
    fn spawned_command_carries_the_reentry_marker() {
        let spec = attempt(Backend::Codex, "p", None, None);
        let (cmd, _) = agent_exec::command(&spec);
        let marked = cmd.as_std().get_envs().any(|(k, v)| {
            k == std::ffi::OsStr::new(REENTRY_MARKER) && v.is_some_and(|v| !v.is_empty())
        });
        assert!(marked, "child command must carry {REENTRY_MARKER}");
    }
}
