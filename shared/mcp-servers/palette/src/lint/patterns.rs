//! The heuristic patterns and budgets of the lint rules, in one place so they can be
//! tuned without touching rule logic.
//!
//! Owns P009 status markers, P010 development-stage patterns, the P011 budgets and the
//! P005 "direct use" keyword. Does not run any rule.

use std::sync::LazyLock;

use regex::Regex;

use crate::util::re;

/// P011: lines a state document may have.
pub const STATE_MAX_LINES: usize = 300;
/// P011: lines one state entry may have.
pub const STATE_ENTRY_MAX_LINES: usize = 3;
/// P011: lines a phase or deliverable file may have.
pub const PHASE_MAX_LINES: usize = 120;
/// P011: lines a backlog item body may have.
pub const ITEM_BODY_MAX_LINES: usize = 4;

/// P005: a `Depends` entry that is also reachable through another entry needs this
/// word (any case) in its parenthetical to count as stating a direct use.
pub const DIRECT_USE_KEYWORD: &str = "direct";

/// P009: a checkbox.
pub static CHECKBOX: LazyLock<Regex> = LazyLock::new(|| re(r"\[[ xX]\]"));

/// P009: a done/pending marker: a bracketed or parenthesized word, a status glyph, or
/// a leading `done:`/`pending -` label.
pub static STATUS_MARKER: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?i)(?:[(\[]\s*(?:done|pending)\s*[)\]]|[✅☑☐✔✓⬜]|^\s*(?:[-*]\s+)?(?:done|pending)\s*(?::|—|–|-)\s)",
    )
});

const MODEL_NAMES: &str =
    r"(?:opus|sonnet|haiku|fable|gpt-[\w.-]+|codex|gemini|kimi|claude|o[134](?:-mini)?)";
const COUNT_WORDS: &str = r"(?:one|two|three|four|five|six|seven|eight|nine|ten|\d+)";
const DATE: &str = r"\d{4}-\d{2}-\d{2}";
const PROGRESS_VERBS: &str = r"(?:started|landed|completed|results|dispatched|merged)";

/// P010 patterns with the message each reports.
pub static STAGE_PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        (
            // A date followed (within two words) by a progress verb.
            re(&format!(
                r"(?i)\b{DATE}\b[\s:,—–()-]*(?:\w+[\s,]+){{0,2}}{PROGRESS_VERBS}\b"
            )),
            "a dated result or progress sentence",
        ),
        (
            re(
                r"(?i)\b(?:wf_[a-z0-9-]{6,}|[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})\b",
            ),
            "a run or dispatch identifier",
        ),
        (
            re(r"(?i)\b(?:run|dispatch|task|job|plan)[_ -]?id\b\s*[:=]?\s*\S+"),
            "a run or dispatch identifier",
        ),
        (
            re(&format!(
                r"(?i)\b(?:{COUNT_WORDS}\s*[x×]?\s*{MODEL_NAMES}\b|{MODEL_NAMES}\s*[x×]\s*{COUNT_WORDS}\b|{MODEL_NAMES}\s+(?:lane|batch)\b|(?:lane|batch)\s*(?:on|of|for|[:=])?\s*{MODEL_NAMES}\b)"
            )),
            "a model-routing instruction",
        ),
    ]
});

#[cfg(test)]
mod tests {
    use super::*;

    fn hits(s: &str) -> Vec<&'static str> {
        STAGE_PATTERNS
            .iter()
            .filter(|(r, _)| r.is_match(s))
            .map(|(_, m)| *m)
            .collect()
    }

    #[test]
    fn dated_progress() {
        assert!(!hits("2026-09-29 landed on main").is_empty());
        assert!(!hits("On 2026-09-29: results were recorded").is_empty());
        assert!(!hits("2026-09-29 phase 1 started").is_empty());
        assert!(hits("Source: user, 2026-09-29. Target: docs.").is_empty());
        assert!(hits("Accepted: 2026-09-29, Hamin Sung").is_empty());
    }

    #[test]
    fn run_identifiers() {
        assert!(!hits("run wf_abc123def").is_empty());
        assert!(!hits("task 3f2a9c1e-1234-4bcd-8ef0-0123456789ab finished").is_empty());
        assert!(!hits("dispatch id: 7").is_empty());
        assert!(hits("the run loop").is_empty());
    }

    #[test]
    fn model_routing() {
        assert!(!hits("use 3 sonnet subagents").is_empty());
        assert!(!hits("sonnet x3").is_empty());
        assert!(!hits("opus lane").is_empty());
        assert!(!hits("batch on haiku").is_empty());
        assert!(!hits("haiku batch").is_empty());
        assert!(hits("the model decides").is_empty());
    }

    #[test]
    fn status_markers() {
        assert!(STATUS_MARKER.is_match("B-1 (done)"));
        assert!(STATUS_MARKER.is_match("- pending: write it"));
        assert!(STATUS_MARKER.is_match("✅ shipped"));
        assert!(!STATUS_MARKER.is_match("the work was done by hand"));
        assert!(CHECKBOX.is_match("- [ ] task"));
        assert!(CHECKBOX.is_match("- [x] task"));
    }
}
