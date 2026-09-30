//! Migration of prefs files written by claude-agent-kit 12.x and the codex and
//! kimi kits 0.7.x to the current format.
//!
//! Owns detection of the old format and the mapping of old values to the new
//! settings: the old `Auto-call policy`, `Execution policy` and `Approval mode`
//! become a `level`, and every value that has a new setting carries over. The
//! old file's `Notes` and `Repository overrides` sections are kept verbatim.
//! It does not decide whether to migrate; the caller asks the user.
//!
//! Main entry points: [`is_legacy_format`] and [`migrate`].

use super::doc::PrefsDoc;
use super::schema::{self, ValidationCtx};
use std::collections::BTreeMap;

/// Sections copied verbatim from the old file into the migrated one.
pub const PRESERVED_SECTIONS: [&str; 2] = ["Notes", "Repository overrides"];

/// True when an aside or dispatch file still has the pre-`level` layout.
///
/// Files with an unchanged layout (git, comment) cannot be recognised by content;
/// the caller treats them as old when an old-format manifest lists them.
pub fn is_legacy_format(file: &str, doc: &PrefsDoc) -> bool {
    match file {
        "aside" => {
            (doc.has_heading("Auto-call policy")
                || doc.has_heading("Preferred third-party advisor"))
                && !doc.has_heading("Level")
        }
        "dispatch" => doc.has_heading("Execution policy") && !doc.has_heading("Level"),
        _ => false,
    }
}

/// The values a migration carries over, keyed by `<key>` within the file.
type Values = BTreeMap<String, String>;

fn aside_values(old: &PrefsDoc, warnings: &mut Vec<String>) -> Values {
    let mut out = Values::new();
    let policy = old.get("Auto-call policy");
    let advisor = old.inline_value("Preferred third-party advisor");
    report_unreadable(old, "aside", "Auto-call policy", policy.as_ref(), warnings);
    report_unreadable(
        old,
        "aside",
        "Preferred third-party advisor",
        advisor.as_ref(),
        warnings,
    );
    let mut level = match policy.as_deref() {
        Some("conservative") | Some("preference-only") => Some("on-request"),
        Some("proactive") => Some("auto"),
        Some(other) => {
            warnings.push(format!(
                "aside: unknown Auto-call policy `{other}`; level left at the default"
            ));
            None
        }
        None => None,
    };
    match advisor.as_deref() {
        Some(b) if schema::ASIDE_BACKENDS.contains(&b) => {
            out.insert("backend".into(), b.into());
        }
        Some("none") => {
            out.insert("backend".into(), "codex".into());
            level = Some("on-request");
        }
        Some("") | None => {}
        Some(other) => warnings.push(format!(
            "aside: unknown preferred advisor `{other}`; backend left at the default"
        )),
    }
    if let Some(l) = level {
        out.insert("level".into(), l.into());
    }
    for b in schema::ASIDE_BACKENDS {
        for (label, key) in [
            ("default model", "model"),
            ("default reasoning effort", "effort"),
            ("default model fallback", "fallback"),
        ] {
            read_list_line(
                old,
                "aside",
                &format!("{b} {label}"),
                &format!("{b}.{key}"),
                &mut out,
                warnings,
            );
        }
    }
    out
}

/// Reads one old list item into `out`, or reports that the line exists but could not be read.
fn read_list_line(
    old: &PrefsDoc,
    file: &str,
    label: &str,
    key: &str,
    out: &mut Values,
    warnings: &mut Vec<String>,
) {
    match old.list_value(label) {
        Some(v) => {
            out.insert(key.to_string(), v);
        }
        None if old.has_list_line(label) => warnings.push(format!(
            "{file}: the old line `- {label}:` could not be read, so {file}.{key} was not carried over"
        )),
        None => {}
    }
}

/// Reports a heading whose value line exists in the old file but could not be read.
fn report_unreadable(
    old: &PrefsDoc,
    file: &str,
    heading: &str,
    value: Option<&String>,
    warnings: &mut Vec<String>,
) {
    if value.is_none() && old.has_heading(heading) {
        warnings.push(format!(
            "{file}: the old value under `## {heading}` could not be read and was not carried over"
        ));
    }
}

fn dispatch_values(old: &PrefsDoc, warnings: &mut Vec<String>) -> Values {
    let mut out = Values::new();
    let approval = old.get("Approval mode");
    let exec_policy = old.get("Execution policy");
    report_unreadable(
        old,
        "dispatch",
        "Execution policy",
        exec_policy.as_ref(),
        warnings,
    );
    report_unreadable(
        old,
        "dispatch",
        "Approval mode",
        approval.as_ref(),
        warnings,
    );
    match exec_policy.as_deref() {
        Some("conservative") | Some("preference-only") => {
            out.insert("level".into(), "on-request".into());
        }
        Some("proactive") => {
            let level = if approval.as_deref() == Some("auto") {
                "auto"
            } else {
                "suggest"
            };
            out.insert("level".into(), level.into());
        }
        Some(other) => warnings.push(format!(
            "dispatch: unknown Execution policy `{other}`; level left at the default"
        )),
        None => {}
    }
    for (label, key) in [
        ("default backend", "backend"),
        ("default model", "model"),
        ("default reasoning effort", "effort"),
        ("default model fallback", "fallback"),
    ] {
        read_list_line(old, "dispatch", label, key, &mut out, warnings);
    }
    out
}

fn unchanged_values(file: &str, old: &PrefsDoc) -> Values {
    schema::for_file(file)
        .filter_map(|s| old.get(s.heading).map(|v| (s.key.to_string(), v)))
        .collect()
}

/// Result of a migration.
#[derive(Debug, Clone)]
pub struct Migrated {
    /// The migrated file text.
    pub text: String,
    /// Old values that could not be carried over, with the reason.
    pub warnings: Vec<String>,
}

/// Builds the migrated text of `file` from the old text and the current template.
///
/// The new file starts from the template, takes every old value that has a new
/// setting (values the new schema rejects fall back to the template's value and
/// are reported), and keeps the old `Notes` and `Repository overrides` verbatim.
pub fn migrate(file: &str, old_text: &str, template_text: &str, ctx: &ValidationCtx) -> Migrated {
    let old = PrefsDoc::parse(old_text);
    let mut warnings = Vec::new();
    let values = match file {
        "aside" => aside_values(&old, &mut warnings),
        "dispatch" => dispatch_values(&old, &mut warnings),
        _ => unchanged_values(file, &old),
    };
    let mut new = PrefsDoc::parse(template_text);
    new.set_eol(old.eol());
    for setting in schema::for_file(file) {
        let Some(raw) = values.get(setting.key) else {
            continue;
        };
        match schema::validate(setting, ctx, raw) {
            Ok(v) => {
                if let Err(e) = new.set(setting.heading, &v) {
                    warnings.push(format!("{file}.{}: {e}", setting.key));
                }
            }
            Err(reason) => warnings.push(format!(
                "{file}.{}: old value `{raw}` was not carried over ({reason})",
                setting.key
            )),
        }
    }
    for section in PRESERVED_SECTIONS {
        new.copy_section_from(&old, section);
    }
    Migrated {
        text: new.render(),
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::Harness;

    const ASIDE_TEMPLATE: &str = "<!-- k-custom:aside-prefs -->\n# Aside Preferences\n\n## Level\n\n**suggest**\n\n## Backend\n\n**codex**\n\n## Codex model\n\n****\n\n## Codex reasoning effort\n\n****\n\n## Codex model fallback\n\n****\n\n## Copilot model\n\n****\n\n## Copilot reasoning effort\n\n****\n\n## Copilot model fallback\n\n****\n\n## Claude model\n\n****\n\n## Claude reasoning effort\n\n****\n\n## Claude model fallback\n\n****\n\n## Notes\n\nnew default notes\n\n## Repository overrides\n\nnew default overrides\n";

    const OLD_ASIDE: &str = "<!-- claude-agent-kit-custom:aside-prefs -->\n# Aside Preferences\n\n## Preferred third-party advisor\n\nDefault backend when Claude Code decides to ask a cross-family advisor: **codex**\n\nValid values: `none` | `codex`\n\n## Default models (per backend)\n\n- codex default model: **gpt-6-astra**\n- copilot default model: ****\n- claude default model: **opus**\n\n## Default reasoning effort (per backend)\n\n- codex default reasoning effort: **high**   (`low`)\n- copilot default reasoning effort: ****\n- claude default reasoning effort: **max**\n\n## Default model fallback chain (per backend)\n\n- codex default model fallback: **gpt-6-sol(high)**   (comma-separated)\n- copilot default model fallback: ****\n- claude default model fallback: ****\n\n## Auto-call policy\n\n**proactive**\n\n## Notes\n\nMy own note.\n";

    fn ctx() -> ValidationCtx {
        ValidationCtx::new(Harness::Claude)
    }

    #[test]
    fn detects_old_layouts() {
        assert!(is_legacy_format("aside", &PrefsDoc::parse(OLD_ASIDE)));
        assert!(!is_legacy_format("aside", &PrefsDoc::parse(ASIDE_TEMPLATE)));
        assert!(!is_legacy_format("git", &PrefsDoc::parse(OLD_ASIDE)));
    }

    #[test]
    fn aside_values_and_level_are_mapped() {
        let m = migrate("aside", OLD_ASIDE, ASIDE_TEMPLATE, &ctx());
        assert!(m.warnings.is_empty(), "{:?}", m.warnings);
        let d = PrefsDoc::parse(&m.text);
        assert_eq!(d.get("Level").as_deref(), Some("auto"));
        assert_eq!(d.get("Backend").as_deref(), Some("codex"));
        assert_eq!(d.get("Codex model").as_deref(), Some("gpt-6-astra"));
        assert_eq!(d.get("Codex reasoning effort").as_deref(), Some("high"));
        assert_eq!(
            d.get("Codex model fallback").as_deref(),
            Some("gpt-6-sol(high)")
        );
        assert_eq!(d.get("Claude model").as_deref(), Some("opus"));
        assert_eq!(d.get("Claude reasoning effort").as_deref(), Some("max"));
        assert_eq!(d.get("Copilot model").as_deref(), Some(""));
        // Notes come from the old file; the section the old file lacks stays from the template.
        assert!(m.text.contains("## Notes\n\nMy own note.\n"));
        assert!(
            m.text
                .contains("## Repository overrides\n\nnew default overrides\n")
        );
        assert!(m.text.starts_with("<!-- k-custom:aside-prefs -->\n"));
    }

    #[test]
    fn aside_policy_and_advisor_table() {
        for (policy, advisor, level, backend) in [
            ("conservative", "codex", "on-request", "codex"),
            ("preference-only", "claude", "on-request", "claude"),
            ("proactive", "copilot", "auto", "copilot"),
            ("proactive", "none", "on-request", "codex"),
            ("conservative", "none", "on-request", "codex"),
        ] {
            let old = format!(
                "## Preferred third-party advisor\n\nDefault backend when Claude Code decides to ask a cross-family advisor: **{advisor}**\n\n## Auto-call policy\n\n**{policy}**\n"
            );
            let m = migrate("aside", &old, ASIDE_TEMPLATE, &ctx());
            let d = PrefsDoc::parse(&m.text);
            assert_eq!(d.get("Level").as_deref(), Some(level), "{policy}/{advisor}");
            assert_eq!(
                d.get("Backend").as_deref(),
                Some(backend),
                "{policy}/{advisor}"
            );
        }
    }

    const DISPATCH_TEMPLATE: &str = "<!-- k-custom:dispatch-prefs -->\n# Dispatch\n\n## Level\n\n**suggest**\n\n## Backend\n\n**codex**\n\n## Model\n\n****\n\n## Reasoning effort\n\n****\n\n## Model fallback\n\n****\n\n## Notes\n\nnew\n";

    fn old_dispatch(policy: &str, approval: &str) -> String {
        format!(
            "<!-- k-custom:dispatch-prefs -->\n## Execution policy\n\n**{policy}**\n\n## Approval mode\n\n**{approval}**\n\n## Default granularity\n\n**batch**\n\n## Default backend / model / effort\n\n- default backend: **opencode**\n- default model: **gpt-6-sol**\n- default reasoning effort: **medium**\n- default model fallback: ****\n\n## Notes\n\nkeep me\n"
        )
    }

    #[test]
    fn dispatch_level_table() {
        for (policy, approval, level) in [
            ("conservative", "ask", "on-request"),
            ("conservative", "auto", "on-request"),
            ("preference-only", "auto", "on-request"),
            ("proactive", "ask", "suggest"),
            ("proactive", "auto", "auto"),
        ] {
            let m = migrate(
                "dispatch",
                &old_dispatch(policy, approval),
                DISPATCH_TEMPLATE,
                &ctx(),
            );
            let d = PrefsDoc::parse(&m.text);
            assert_eq!(
                d.get("Level").as_deref(),
                Some(level),
                "{policy}/{approval}"
            );
            assert_eq!(d.get("Backend").as_deref(), Some("opencode"));
            assert_eq!(d.get("Model").as_deref(), Some("gpt-6-sol"));
            assert_eq!(d.get("Reasoning effort").as_deref(), Some("medium"));
            assert!(!m.text.contains("granularity"), "granularity is dropped");
            assert!(m.text.contains("## Notes\n\nkeep me\n"));
        }
    }

    #[test]
    fn git_and_comment_values_carry_over_unchanged() {
        let template = "<!-- k-custom:git-prefs -->\n# Git\n\n## Commit signing\n\n**unset**\n\n## Model attribution\n\n**unset**\n\n## Commit message format\n\n**unset**\n\n## PR body format\n\n**unset**\n\n## Branch naming\n\n**unset**\n\n## Repository overrides\n\nnew\n\n## Notes\n\nnew\n";
        let old = "<!-- k-custom:git-prefs -->\n# Git\n\nold intro\n\n## Commit signing\n\n**no-gpg-sign**\n\n## Model attribution\n\n**off**\n\n## Commit message format\n\n**my own <type>: format**\n\n## PR body format\n\n**unset**\n\n## Branch naming\n\n**descriptive**\n\n## Repository overrides\n\n- /r: keep\n\n## Notes\n\nmine\n";
        let m = migrate("git", old, template, &ctx());
        assert!(m.warnings.is_empty());
        let d = PrefsDoc::parse(&m.text);
        assert_eq!(d.get("Commit signing").as_deref(), Some("no-gpg-sign"));
        assert_eq!(d.get("Model attribution").as_deref(), Some("off"));
        assert_eq!(
            d.get("Commit message format").as_deref(),
            Some("my own <type>: format")
        );
        assert_eq!(d.get("PR body format").as_deref(), Some("unset"));
        assert_eq!(d.get("Branch naming").as_deref(), Some("descriptive"));
        assert!(m.text.contains("## Repository overrides\n\n- /r: keep\n"));
        assert!(m.text.contains("## Notes\n\nmine\n"));
        assert!(
            !m.text.contains("old intro"),
            "free text outside preserved sections comes from the template"
        );
    }

    #[test]
    fn unbolded_fallback_lines_survive_migration() {
        let old = "## Preferred third-party advisor\n\nDefault backend when X decides to ask: **codex**\n\n## Default models (per backend)\n\n- codex default model: **model-a**\n\n## Default reasoning effort (per backend)\n\n- codex default reasoning effort: **high**   (`low` / `medium`)\n\n## Default model fallback chain (per backend)\n\n- codex default model fallback: model-b(high), model-c   (comma-separated; blank = none)\n- copilot default model fallback: ****   (comma-separated; blank = none)\n- claude default model fallback: model-d  (comma-separated; blank = none)\n\n## Auto-call policy\n\n**proactive**\n";
        let m = migrate("aside", old, ASIDE_TEMPLATE, &ctx());
        assert!(m.warnings.is_empty(), "{:?}", m.warnings);
        let d = PrefsDoc::parse(&m.text);
        assert_eq!(
            d.get("Codex model fallback").as_deref(),
            Some("model-b(high), model-c")
        );
        assert_eq!(d.get("Copilot model fallback").as_deref(), Some(""));
        assert_eq!(d.get("Claude model fallback").as_deref(), Some("model-d"));
        assert_eq!(d.get("Codex reasoning effort").as_deref(), Some("high"));
    }

    #[test]
    fn unbolded_dispatch_list_lines_survive_migration() {
        let old = "## Execution policy\n\n**proactive**\n\n## Approval mode\n\n**ask**\n\n## Default backend / model / effort\n\n- default backend: **codex**   (codex / opencode / claude)\n- default model: **model-d**\n- default reasoning effort: **medium**   (low / medium)\n- default model fallback: model-e, model-f   (comma-separated; blank = none)\n";
        let m = migrate("dispatch", old, DISPATCH_TEMPLATE, &ctx());
        assert!(m.warnings.is_empty(), "{:?}", m.warnings);
        let d = PrefsDoc::parse(&m.text);
        assert_eq!(d.get("Model fallback").as_deref(), Some("model-e, model-f"));
        assert_eq!(d.get("Level").as_deref(), Some("suggest"));
    }

    #[test]
    fn a_value_that_cannot_be_read_is_reported_not_dropped() {
        let old = "## Preferred third-party advisor\n\nDefault backend: codex\n\n## Default model fallback chain (per backend)\n\n- codex default model fallback: **unterminated\n\n## Auto-call policy\n\nproactive without bold\n";
        let m = migrate("aside", old, ASIDE_TEMPLATE, &ctx());
        assert_eq!(m.warnings.len(), 3, "{:?}", m.warnings);
        assert!(
            m.warnings
                .iter()
                .any(|w| w.contains("codex default model fallback"))
        );
        assert!(m.warnings.iter().any(|w| w.contains("Auto-call policy")));
        assert!(
            m.warnings
                .iter()
                .any(|w| w.contains("Preferred third-party advisor"))
        );
    }

    #[test]
    fn invalid_old_values_fall_back_and_are_reported() {
        let old = "## Preferred third-party advisor\n\nDefault backend when X decides to ask: **codex**\n\n## Default reasoning effort (per backend)\n\n- codex default reasoning effort: **turbo**\n\n## Auto-call policy\n\n**whatever**\n";
        let m = migrate("aside", old, ASIDE_TEMPLATE, &ctx());
        assert_eq!(m.warnings.len(), 2, "{:?}", m.warnings);
        let d = PrefsDoc::parse(&m.text);
        assert_eq!(d.get("Codex reasoning effort").as_deref(), Some(""));
        assert_eq!(d.get("Level").as_deref(), Some("suggest"));
    }

    #[test]
    fn crlf_files_stay_crlf() {
        let old = OLD_ASIDE.replace('\n', "\r\n");
        let m = migrate("aside", &old, ASIDE_TEMPLATE, &ctx());
        assert_eq!(m.text.matches('\n').count(), m.text.matches("\r\n").count());
        assert!(m.text.contains("## Notes\r\n\r\nMy own note.\r\n"));
    }
}
