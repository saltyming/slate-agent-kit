//! The failure text of an outcome: what the classifier (`errkind::classify`)
//! and the user see when an attempt fails.
//!
//! Entry points: [`FailureTextPolicy`] (with the [`FailureTextPolicy::aside`]
//! and [`FailureTextPolicy::dispatch`] presets) and [`failure_text`]. stderr is
//! always part of the text; stdout only for the backends the policy lists,
//! because a backend's stdout can hold a partial answer that mentions, say, a
//! rate limit and would read as one.

use crate::run::Outcome;
use crate::spec::Backend;

/// Which backends contribute stdout to the failure text, and how much.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureTextPolicy {
    /// Backends whose stdout joins the text. Claude prints its discriminating
    /// error (e.g. an unknown model) on stdout and exits non-zero with only
    /// warnings on stderr.
    pub stdout_for: Vec<Backend>,
    /// Keep only the last this many characters of stdout; `None` keeps all the
    /// record holds (the capture policy's failure cap applies).
    pub stdout_tail: Option<usize>,
}

impl FailureTextPolicy {
    /// aside's policy: claude's stdout, as much as the failure capture kept.
    pub fn aside() -> Self {
        FailureTextPolicy {
            stdout_for: vec![Backend::Claude],
            stdout_tail: None,
        }
    }

    /// dispatch's policy: claude's stdout, its last 2,000 characters.
    pub fn dispatch() -> Self {
        FailureTextPolicy {
            stdout_for: vec![Backend::Claude],
            stdout_tail: Some(2000),
        }
    }
}

/// The text to classify for a failed outcome of `backend`, or `None` for a
/// success, a cancellation or a missing binary (none of which is retried).
///
/// `backend` is the backend's stable name (`Backend::as_str`, or a runner's
/// own such as `opencode`, which is in no `stdout_for` list). A record that
/// exited non-zero gives `exit_code=<code> stderr=<stderr>`, followed by
/// ` stdout=<stdout>` when `backend` is in `policy.stdout_for`. A `Spawn` or
/// `WaitFailed` outcome gives its message.
pub fn failure_text(
    policy: &FailureTextPolicy,
    backend: &str,
    outcome: &Outcome,
) -> Option<String> {
    match outcome {
        Outcome::Record(r) if !r.success => {
            let mut text = format!("exit_code={:?} stderr={}", r.exit_code, r.stderr);
            if policy.stdout_for.iter().any(|b| b.as_str() == backend) {
                text.push_str(" stdout=");
                match policy.stdout_tail {
                    Some(n) => {
                        let total = r.stdout.chars().count();
                        text.extend(r.stdout.chars().skip(total.saturating_sub(n)));
                    }
                    None => text.push_str(&r.stdout),
                }
            }
            Some(text)
        }
        Outcome::Spawn(msg) | Outcome::WaitFailed(msg) => Some(msg.clone()),
        Outcome::Record(_) | Outcome::NotFound { .. } | Outcome::Cancelled => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errkind::{BackendErrorKind, classify};
    use crate::run::RunRecord;

    fn ft(policy: &FailureTextPolicy, backend: Backend, o: &Outcome) -> Option<String> {
        failure_text(policy, backend.as_str(), o)
    }

    fn failed(backend: Backend, stdout: &str, stderr: &str) -> Outcome {
        Outcome::Record(RunRecord {
            backend: backend.binary().into(),
            exit_code: Some(1),
            success: false,
            stdout: stdout.into(),
            stderr: stderr.into(),
            ..RunRecord::default()
        })
    }

    #[test]
    fn claude_stdout_joins_the_text_and_classifies() {
        let bad_model = "There's an issue with the selected model (x). It may not exist or \
             you may not have access to it.";
        let o = failed(Backend::Claude, bad_model, "Warning: Advisor disabled");
        for policy in [FailureTextPolicy::aside(), FailureTextPolicy::dispatch()] {
            let text = ft(&policy, Backend::Claude, &o).unwrap();
            assert_eq!(
                text,
                format!("exit_code=Some(1) stderr=Warning: Advisor disabled stdout={bad_model}")
            );
            assert_eq!(classify(&text), BackendErrorKind::ModelUnavailable);
        }
    }

    #[test]
    fn codex_stdout_mentioning_a_rate_limit_is_not_retry_worthy() {
        let o = failed(
            Backend::Codex,
            "Here is how to handle a 429 Too Many Requests rate limit error...",
            "error: sandbox denied the write",
        );
        for policy in [FailureTextPolicy::aside(), FailureTextPolicy::dispatch()] {
            let text = ft(&policy, Backend::Codex, &o).unwrap();
            assert!(!text.contains("stdout="), "{text}");
            assert!(!classify(&text).is_retry_worthy(), "{text}");
        }
    }

    #[test]
    fn dispatch_keeps_the_last_2000_characters_of_stdout() {
        let stdout = format!("{}{}", "é".repeat(3000), "rate limit exceeded");
        let o = failed(Backend::Claude, &stdout, "");
        let text = ft(&FailureTextPolicy::dispatch(), Backend::Claude, &o).unwrap();
        let kept = text.split_once(" stdout=").unwrap().1;
        assert_eq!(kept.chars().count(), 2000);
        assert!(kept.ends_with("rate limit exceeded"));

        let all = ft(&FailureTextPolicy::aside(), Backend::Claude, &o).unwrap();
        assert_eq!(all.split_once(" stdout=").unwrap().1, stdout);
    }

    #[test]
    fn spawn_and_wait_failures_carry_their_message_and_the_rest_carry_none() {
        let policy = FailureTextPolicy::aside();
        assert_eq!(
            failure_text(
                &policy,
                "codex",
                &Outcome::Spawn("spawn codex failed: x".into())
            )
            .as_deref(),
            Some("spawn codex failed: x")
        );
        assert_eq!(
            failure_text(
                &policy,
                "codex",
                &Outcome::WaitFailed("wait failed: y".into())
            )
            .as_deref(),
            Some("wait failed: y")
        );
        assert_eq!(failure_text(&policy, "codex", &Outcome::Cancelled), None);
        assert_eq!(
            failure_text(
                &policy,
                "codex",
                &Outcome::NotFound {
                    binary: "codex".into(),
                    hint: "install".into()
                }
            ),
            None
        );
        let ok = Outcome::Record(RunRecord {
            success: true,
            exit_code: Some(0),
            ..RunRecord::default()
        });
        assert_eq!(failure_text(&policy, "claude", &ok), None);
    }

    #[test]
    fn a_runner_outside_stdout_for_contributes_stderr_only() {
        // dispatch's opencode runner reports through the same record type.
        let o = Outcome::Record(RunRecord {
            backend: "opencode".into(),
            exit_code: Some(1),
            stdout: "rate limit mentioned in an answer".into(),
            stderr: "boom".into(),
            ..RunRecord::default()
        });
        let text = failure_text(&FailureTextPolicy::dispatch(), "opencode", &o).unwrap();
        assert_eq!(text, "exit_code=Some(1) stderr=boom");
    }
}
