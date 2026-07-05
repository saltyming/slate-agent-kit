//! Backend CLI adapters.
//!
//! Each backend has one function that:
//!  1. Composes the prompt (context / transcript / question).
//!  2. Spawns the CLI via `tokio::process::Command` (kill_on_drop).
//!  3. Captures stdout as the backend's reply.
//!
//! There is intentionally no wall-clock timeout — advisor calls can
//! legitimately take minutes on complex prompts (the built-in `advisor()`
//! has no timeout either). The caller interrupts if they want to abort.
//!
//! Exact invocation flags confirmed from the user's local `--help` output at
//! plan time (2026-04-14). The argv template per backend is localised in
//! `build_command` so future CLI syntax drift is a single-line change.

use std::process::Stdio;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

const MAX_CAPTURED_STDERR: usize = 2 * 1024;
const MAX_CAPTURED_STDOUT: usize = 50 * 1024;

/// Env var marking how deep we are inside aside-spawned backends.
///
/// A top-level harness call has it unset (depth 0). Every backend aside spawns
/// carries the incremented value, and the child inherits it down its whole
/// process tree — including any `aside` MCP server the child itself boots from
/// its own config. The tool layer refuses a call whose depth is already at the
/// ceiling, so a spawned backend can never recursively re-enter aside. This is a
/// defense-in-depth net; the primary guarantee is that each backend is spawned
/// with no MCP servers at all (codex `--ignore-user-config`, claude `--safe-mode`,
/// copilot's tool whitelist).
pub const REENTRY_DEPTH_ENV: &str = "ASIDE_REENTRY_DEPTH";

/// Depth at or above which a call is refused. `1` = no nesting: a top-level call
/// (depth 0) proceeds; anything aside itself spawned (depth ≥ 1) is refused.
pub const REENTRY_CEILING: u32 = 1;

/// Parse a re-entry depth from the raw env value. Fail-closed: unset/empty is a
/// legitimate top-level call (0); a present-but-malformed value is treated as
/// past the ceiling (`u32::MAX`) so a corrupt marker refuses rather than
/// silently permitting recursion.
fn parse_reentry_depth(raw: Option<&str>) -> u32 {
    match raw {
        None => 0,
        Some(s) if s.trim().is_empty() => 0,
        Some(s) => s.trim().parse::<u32>().unwrap_or(u32::MAX),
    }
}

/// Current re-entry depth, read from the environment. Uses `var_os` so a
/// present-but-non-Unicode value fails closed (`u32::MAX`) instead of being
/// misread as unset (which `env::var().ok()` would do).
pub fn reentry_depth() -> u32 {
    depth_from_env(std::env::var_os(REENTRY_DEPTH_ENV).as_deref())
}

/// Pure core of `reentry_depth`, split out for testing: unset → 0; valid Unicode
/// → `parse_reentry_depth`; present-but-non-Unicode (malformed) → `u32::MAX`.
fn depth_from_env(raw: Option<&std::ffi::OsStr>) -> u32 {
    match raw {
        None => 0,
        Some(v) => match v.to_str() {
            Some(s) => parse_reentry_depth(Some(s)),
            None => u32::MAX,
        },
    }
}

/// Stamp the next depth (current + 1, saturating) on a child command so a
/// backend aside spawns — and anything it in turn spawns — inherits the marker.
fn stamp_reentry_depth(cmd: &mut Command) {
    cmd.env(
        REENTRY_DEPTH_ENV,
        reentry_depth().saturating_add(1).to_string(),
    );
}

/// Which CLI we're talking to. Each variant maps to a concrete command builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Codex,
    Copilot,
    Claude,
}

impl Backend {
    pub fn binary(&self) -> &'static str {
        match self {
            Backend::Codex => "codex",
            Backend::Copilot => "copilot",
            Backend::Claude => "claude",
        }
    }

    pub fn all() -> &'static [Backend] {
        &[Backend::Codex, Backend::Copilot, Backend::Claude]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptTransport {
    Argv,
    Stdin,
}

struct BuiltCommand {
    cmd: Command,
    argv: Vec<String>,
    prompt_transport: PromptTransport,
}

/// Structured outcome of a backend call. Returned to the tool layer which
/// converts it into `CallToolResult`.
pub enum InvokeOutcome {
    Ok {
        stdout: String,
        truncated: bool,
    },
    NotFound {
        binary: &'static str,
        hint: String,
    },
    Failed {
        code: Option<i32>,
        stderr: String,
        /// Captured stdout on the failure path. Some CLIs (notably `claude -p`)
        /// exit non-zero but print the *discriminating* error — e.g. an
        /// unknown/inaccessible model — to stdout, leaving stderr with only
        /// incidental warnings. The classifier and the user-facing error both
        /// need stdout, or a fallback-worthy failure would be misclassified as
        /// `Other` and the model_fallback chain would never advance.
        stdout: String,
    },
    Spawn(String),
    /// Client cancelled the request (MCP CancelledNotification). The child
    /// process was killed as part of the select arm's future drop path
    /// (kill_on_drop=true on the Command).
    Cancelled,
}

/// Ask a backend a question.
///
/// There is intentionally no wall-clock timeout — advisor-style CLIs can
/// legitimately take minutes, and the built-in `advisor()` has no timeout
/// either. Cancellation is driven by the MCP request's `CancellationToken`:
/// when the client sends a CancelledNotification (or the harness tears down
/// the session) rmcp calls `ct.cancel()`, this function's `tokio::select!`
/// abandons the `wait_with_output` future, the child is dropped, and
/// `kill_on_drop(true)` sends SIGKILL. The subprocess does NOT outlive a
/// cancelled request.
///
/// The only case not covered is "client stays connected, child wedges,
/// no cancellation" — the user walked away. That's by design.
pub async fn invoke(
    backend: Backend,
    prompt: &str,
    model: Option<&str>,
    reasoning_effort: Option<&str>,
    ct: &CancellationToken,
) -> InvokeOutcome {
    if which(backend.binary()).is_none() {
        return InvokeOutcome::NotFound {
            binary: backend.binary(),
            hint: install_hint(backend),
        };
    }

    let BuiltCommand {
        mut cmd,
        argv: _argv,
        prompt_transport,
    } = build_command(backend, prompt, model, reasoning_effort);
    match prompt_transport {
        PromptTransport::Argv => {
            cmd.stdin(Stdio::null());
        }
        PromptTransport::Stdin => {
            cmd.stdin(Stdio::piped());
        }
    }
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    // Mark the child (and its whole process tree) as aside-spawned so a nested
    // aside call from within the backend is refused before it can recurse.
    stamp_reentry_depth(&mut cmd);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return InvokeOutcome::Spawn(format!("spawn {} failed: {}", backend.binary(), e));
        }
    };

    if prompt_transport == PromptTransport::Stdin {
        let Some(mut stdin) = child.stdin.take() else {
            return InvokeOutcome::Spawn(format!("{} stdin pipe unavailable", backend.binary()));
        };
        let write_prompt = async {
            stdin.write_all(prompt.as_bytes()).await?;
            stdin.shutdown().await
        };
        tokio::select! {
            biased;
            _ = ct.cancelled() => {
                return InvokeOutcome::Cancelled;
            }
            res = write_prompt => {
                if let Err(e) = res {
                    return InvokeOutcome::Spawn(format!("write prompt to {} failed: {}", backend.binary(), e));
                }
            }
        }
    }

    let output = tokio::select! {
        biased;
        _ = ct.cancelled() => {
            // child is owned by the wait future; dropping this select arm
            // drops the future, dropping the child, triggering kill_on_drop.
            return InvokeOutcome::Cancelled;
        }
        res = child.wait_with_output() => match res {
            Ok(o) => o,
            Err(e) => return InvokeOutcome::Spawn(format!("wait failed: {}", e)),
        }
    };

    if !output.status.success() {
        let mut stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if stderr.len() > MAX_CAPTURED_STDERR {
            let keep = &stderr[stderr.len() - MAX_CAPTURED_STDERR..];
            stderr = format!(
                "[stderr truncated to last {} bytes]\n{}",
                MAX_CAPTURED_STDERR, keep
            );
        }
        let mut stdout = String::from_utf8_lossy(&output.stdout).to_string();
        if stdout.len() > MAX_CAPTURED_STDERR {
            // char-safe head slice (the discriminating error is at the top for
            // the CLIs that print errors to stdout); avoids a mid-codepoint panic.
            let head: String = stdout.chars().take(MAX_CAPTURED_STDERR).collect();
            stdout = format!("{head}\n[stdout truncated]");
        }
        return InvokeOutcome::Failed {
            code: output.status.code(),
            stderr,
            stdout,
        };
    }

    let mut stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let mut truncated = false;
    if stdout.len() > MAX_CAPTURED_STDOUT {
        let keep: String = stdout.chars().take(MAX_CAPTURED_STDOUT).collect();
        let orig_len = stdout.len();
        stdout = format!(
            "{}\n\n[response truncated after {} bytes; original was {} bytes]",
            keep, MAX_CAPTURED_STDOUT, orig_len
        );
        truncated = true;
    }

    InvokeOutcome::Ok { stdout, truncated }
}

/// Build the `Command` for a backend. Prompt and flags inlined; stdio is
/// configured by the caller in `invoke`.
fn build_command(
    backend: Backend,
    prompt: &str,
    model: Option<&str>,
    reasoning_effort: Option<&str>,
) -> BuiltCommand {
    match backend {
        Backend::Codex => {
            // codex -s read-only -a never [-m MODEL] [-c model_reasoning_effort=EFF]
            //       exec --ignore-user-config "<PROMPT>"
            //   -s read-only: sandbox blocks file writes / shell side effects but ALLOWS reads,
            //                 so codex can open files the caller references by path.
            //   -a never:     skip approval prompts (non-interactive)
            //   -c ...:       TOML config override for reasoning effort
            //   exec:         non-interactive subcommand; prompt is the positional arg
            //   --ignore-user-config: do NOT load ~/.codex/config.toml for this run, so the
            //                 spawned codex carries NO MCP servers (neither aside nor dispatch)
            //                 and cannot recurse back into aside. Auth still resolves from
            //                 CODEX_HOME (auth.json is separate). Flag is an `exec` subcommand
            //                 flag, so it MUST come after `exec` (codex 0.142.x).
            let mut cmd = Command::new("codex");
            cmd.arg("-s").arg("read-only");
            cmd.arg("-a").arg("never");
            if let Some(m) = model {
                cmd.arg("-m").arg(m);
            }
            if let Some(eff) = reasoning_effort {
                cmd.arg("-c").arg(format!("model_reasoning_effort={}", eff));
            }
            cmd.arg("exec");
            cmd.arg("--ignore-user-config");
            cmd.arg(prompt);
            BuiltCommand {
                argv: vec!["codex".into()],
                cmd,
                prompt_transport: PromptTransport::Argv,
            }
        }
        Backend::Copilot => {
            // copilot -p "<PROMPT>" --allow-all-tools --available-tools=view,rg,glob,web_fetch
            //         -s --no-color [--model MODEL] [--effort EFF]
            //   -p:                  non-interactive prompt via argv
            //   --allow-all-tools:   required for non-interactive mode per help (auto-approve
            //                        whatever is in --available-tools; no approval prompts)
            //   --available-tools=…: read-only tool whitelist so copilot can inspect files the
            //                        caller references by path:
            //                          view      — read file contents
            //                          rg        — ripgrep across the workspace
            //                          glob      — file path pattern match
            //                          web_fetch — fetch URL bodies for docs / spec lookups
            //                        Intentionally excludes bash / write_bash / read_bash / task
            //                        / skill / sql / store_memory / report_intent, which would
            //                        let copilot exec shells or mutate state — aside is Q&A only.
            //   -s:                  silent (stdout contains only the response)
            //   --no-color:          strip ANSI for clean capture
            let mut cmd = Command::new("copilot");
            cmd.arg("-p").arg(prompt);
            cmd.arg("--allow-all-tools");
            cmd.arg("--available-tools=view,rg,glob,web_fetch");
            cmd.arg("-s");
            cmd.arg("--no-color");
            if let Some(m) = model {
                cmd.arg("--model").arg(m);
            }
            if let Some(eff) = reasoning_effort {
                cmd.arg("--effort").arg(eff);
            }
            BuiltCommand {
                argv: vec!["copilot".into()],
                cmd,
                prompt_transport: PromptTransport::Argv,
            }
        }
        Backend::Claude => {
            // claude -p --safe-mode --no-session-persistence --permission-mode plan
            //        --tools Read,Grep,Glob,WebFetch --input-format text --output-format text
            //        [--model MODEL] [--effort EFF]   (prompt on stdin)
            //   -p:                       print response and exit (non-interactive)
            //   --safe-mode:              disable project/user customizations, hooks, plugins,
            //                             MCP servers, and CLAUDE.md discovery for a predictable
            //                             advisor subprocess.
            //   --no-session-persistence: avoid writing an aside-only Claude session to disk.
            //   --permission-mode plan:   read-only planning mode; no edits or state-changing
            //                             tool approvals.
            //   --tools …:                expose only built-in read/search/fetch tools.
            //   prompt on stdin:          avoids OS argv-length and quoting limits for long
            //                             redacted transcripts.
            let mut args: Vec<String> = vec![
                "-p".into(),
                "--safe-mode".into(),
                "--no-session-persistence".into(),
                "--permission-mode".into(),
                "plan".into(),
                "--tools".into(),
                "Read,Grep,Glob,WebFetch".into(),
                "--input-format".into(),
                "text".into(),
                "--output-format".into(),
                "text".into(),
            ];
            if let Some(m) = model {
                args.push("--model".into());
                args.push(m.into());
            }
            if let Some(eff) = reasoning_effort {
                args.push("--effort".into());
                args.push(eff.into());
            }

            let mut cmd = Command::new("claude");
            cmd.args(&args);
            let mut argv = vec!["claude".into()];
            argv.extend(args);
            BuiltCommand {
                cmd,
                argv,
                prompt_transport: PromptTransport::Stdin,
            }
        }
    }
}

fn install_hint(backend: Backend) -> String {
    match backend {
        Backend::Codex => {
            "install codex CLI (`npm i -g @openai/codex`; see https://github.com/openai/codex)"
                .to_string()
        }
        Backend::Copilot => {
            "install copilot CLI (see https://docs.github.com/copilot/how-tos/copilot-cli)"
                .to_string()
        }
        Backend::Claude => "install Claude Code CLI (`npm i -g @anthropic-ai/claude-code`; see \
             https://claude.com/claude-code)"
            .to_string(),
    }
}

/// Minimal PATH lookup — returns Some(path) if the binary is executable.
pub fn which(binary: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(binary);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let exe = dir.join(format!("{}.exe", binary));
            if exe.is_file() {
                return Some(exe);
            }
        }
    }
    None
}

/// Ask the backend CLI for its `--version` string. Returns `None` if missing.
pub async fn version(backend: Backend) -> Option<String> {
    let _ = which(backend.binary())?;
    let output = Command::new(backend.binary())
        .arg("--version")
        .env(
            REENTRY_DEPTH_ENV,
            reentry_depth().saturating_add(1).to_string(),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .ok()?;
    let out = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !out.is_empty() {
        return Some(out);
    }
    let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if err.is_empty() { None } else { Some(err) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_backends_include_claude() {
        assert_eq!(
            Backend::all(),
            &[Backend::Codex, Backend::Copilot, Backend::Claude]
        );
    }

    #[test]
    fn claude_command_is_read_only_and_uses_stdin_prompt() {
        let built = build_command(Backend::Claude, "prompt body", Some("sonnet"), Some("high"));
        let joined = built.argv.join(" ");
        assert!(joined.starts_with("claude -p"));
        assert!(joined.contains("--safe-mode"));
        assert!(joined.contains("--no-session-persistence"));
        assert!(joined.contains("--permission-mode plan"));
        assert!(joined.contains("--tools Read,Grep,Glob,WebFetch"));
        assert!(joined.contains("--input-format text"));
        assert!(joined.contains("--output-format text"));
        assert!(joined.contains("--model sonnet"));
        assert!(joined.contains("--effort high"));
        assert!(!joined.contains("prompt body"));
        assert_eq!(built.prompt_transport, PromptTransport::Stdin);
    }

    #[test]
    fn codex_and_copilot_keep_argv_prompt_transport() {
        let codex = build_command(Backend::Codex, "prompt body", None, None);
        let copilot = build_command(Backend::Copilot, "prompt body", None, None);
        assert_eq!(codex.prompt_transport, PromptTransport::Argv);
        assert_eq!(copilot.prompt_transport, PromptTransport::Argv);
    }

    #[test]
    fn codex_disables_user_config_after_exec() {
        let built = build_command(Backend::Codex, "prompt body", Some("gpt-5.5"), Some("high"));
        let args: Vec<String> = built
            .cmd
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let exec_pos = args.iter().position(|a| a == "exec").expect("exec present");
        let ignore_pos = args
            .iter()
            .position(|a| a == "--ignore-user-config")
            .expect("--ignore-user-config present");
        assert!(
            ignore_pos > exec_pos,
            "--ignore-user-config must come after `exec`: {args:?}"
        );
    }

    #[test]
    fn parse_reentry_depth_fails_closed() {
        assert_eq!(parse_reentry_depth(None), 0, "unset is top-level");
        assert_eq!(parse_reentry_depth(Some("")), 0, "empty is top-level");
        assert_eq!(parse_reentry_depth(Some("  ")), 0, "blank is top-level");
        assert_eq!(parse_reentry_depth(Some("0")), 0);
        assert_eq!(parse_reentry_depth(Some("1")), 1);
        assert_eq!(parse_reentry_depth(Some(" 2 ")), 2);
        // malformed markers fail closed (>= ceiling) rather than reading as 0.
        assert_eq!(parse_reentry_depth(Some("abc")), u32::MAX);
        assert_eq!(parse_reentry_depth(Some("-1")), u32::MAX);
        assert!(parse_reentry_depth(Some("abc")) >= REENTRY_CEILING);
    }

    #[test]
    fn depth_from_env_fails_closed_on_non_unicode() {
        use std::ffi::OsStr;
        assert_eq!(depth_from_env(None), 0, "unset is top-level");
        assert_eq!(depth_from_env(Some(OsStr::new(""))), 0);
        assert_eq!(depth_from_env(Some(OsStr::new("1"))), 1);
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let bad = OsStr::from_bytes(&[0xff, 0xfe]); // invalid UTF-8
            assert_eq!(
                depth_from_env(Some(bad)),
                u32::MAX,
                "present-but-non-Unicode marker must fail closed"
            );
        }
    }

    #[test]
    fn stamped_child_carries_reentry_marker() {
        let mut cmd = Command::new("true");
        stamp_reentry_depth(&mut cmd);
        let marked = cmd.as_std().get_envs().any(|(k, v)| {
            k == std::ffi::OsStr::new(REENTRY_DEPTH_ENV) && v.is_some_and(|v| !v.is_empty())
        });
        assert!(marked, "child command must carry {REENTRY_DEPTH_ENV}");
    }
}
