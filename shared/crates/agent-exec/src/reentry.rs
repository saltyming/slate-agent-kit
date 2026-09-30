//! Re-entry depth markers: keep a spawned backend from recursing into the
//! server that spawned it.
//!
//! A server names an environment variable (aside `ASIDE_REENTRY_DEPTH`,
//! dispatch `DISPATCH_REENTRY_DEPTH`) and a ceiling. A top-level call has the
//! variable unset (depth 0). Every process the server spawns — the backend and
//! the `--version` probe — carries the server's depth plus one ([`stamp`]),
//! and the child inherits it down its whole process tree, including any copy
//! of the server the backend boots from its own configuration. That copy sees
//! the marker and refuses the request ([`refused`]). This is defense in depth;
//! the primary guarantee is the backend's isolation flags.
//!
//! Parsing fails closed: unset or empty is depth 0, while a malformed or
//! non-Unicode value is past every ceiling (`u32::MAX`), so a corrupt marker
//! refuses rather than silently permitting recursion.

use tokio::process::Command;

/// The server's current depth for marker `name`, read from the environment.
/// Uses `var_os` so a present-but-non-Unicode value fails closed instead of
/// being misread as unset (which `env::var().ok()` would do).
pub fn depth(name: &str) -> u32 {
    depth_from_env(std::env::var_os(name).as_deref())
}

/// Stamp the next depth (current + 1, saturating) of marker `name` on a child
/// command, so the child — and anything it in turn spawns — inherits it.
pub fn stamp(command: &mut Command, name: &str) {
    command.env(name, depth(name).saturating_add(1).to_string());
}

/// Whether a request must be refused: the server's own depth for marker
/// `name` is at or above `ceiling`.
pub fn refused(name: &str, ceiling: u32) -> bool {
    depth(name) >= ceiling
}

/// Parse a re-entry depth from the raw env value: unset/empty/blank is 0; a
/// present-but-malformed value is `u32::MAX`.
fn parse_depth(raw: Option<&str>) -> u32 {
    match raw {
        None => 0,
        Some(s) if s.trim().is_empty() => 0,
        Some(s) => s.trim().parse::<u32>().unwrap_or(u32::MAX),
    }
}

/// Pure core of `depth`, split out for testing: unset → 0; valid Unicode →
/// `parse_depth`; present-but-non-Unicode (malformed) → `u32::MAX`.
fn depth_from_env(raw: Option<&std::ffi::OsStr>) -> u32 {
    match raw {
        None => 0,
        Some(v) => match v.to_str() {
            Some(s) => parse_depth(Some(s)),
            None => u32::MAX,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_depth_fails_closed() {
        assert_eq!(parse_depth(None), 0, "unset is top-level");
        assert_eq!(parse_depth(Some("")), 0, "empty is top-level");
        assert_eq!(parse_depth(Some("  ")), 0, "blank is top-level");
        assert_eq!(parse_depth(Some("0")), 0);
        assert_eq!(parse_depth(Some("1")), 1);
        assert_eq!(parse_depth(Some(" 2 ")), 2);
        // malformed markers fail closed (>= any ceiling) rather than reading as 0.
        assert_eq!(parse_depth(Some("abc")), u32::MAX);
        assert_eq!(parse_depth(Some("-1")), u32::MAX);
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
    fn unset_marker_is_depth_zero_and_not_refused() {
        // A name no environment sets: depth 0, allowed under ceiling 1,
        // refused under ceiling 0.
        let name = "AGENT_EXEC_TEST_UNSET_REENTRY_MARKER";
        assert_eq!(depth(name), 0);
        assert!(!refused(name, 1));
        assert!(refused(name, 0));
    }

    #[test]
    fn stamped_child_carries_next_depth() {
        let name = "AGENT_EXEC_TEST_STAMP_MARKER";
        let mut cmd = Command::new("true");
        stamp(&mut cmd, name);
        let value = cmd
            .as_std()
            .get_envs()
            .find(|(k, _)| *k == std::ffi::OsStr::new(name))
            .and_then(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()));
        assert_eq!(value.as_deref(), Some("1"), "child must carry {name}=1");
    }
}
