//! Migration of prefs files written by claude-agent-kit 12.x and the codex and
//! kimi kits 0.7.x to the current format.
//!
//! Owns detection of the old format and the mapping of old values to the new
//! settings: the old `Auto-call policy`, `Execution policy` and `Approval mode`
//! become a `level`, and every value that has a new setting carries over. The
//! old file's `Notes` and `Repository overrides` sections are kept verbatim, and
//! every other old section the template has no heading for and the migration
//! does not read is appended verbatim at the end. Old per-backend values of a
//! backend the schema lacks are reported in one warning.
//! It does not decide whether to migrate; the caller asks the user.
//!
//! Main entry points: [`is_legacy_format`] and [`migrate`].

use super::doc::PrefsDoc;
use super::schema::{self, ValidationCtx};
use std::collections::BTreeMap;

/// Sections copied verbatim from the old file into the migrated one.
pub const PRESERVED_SECTIONS: [&str; 2] = ["Notes", "Repository overrides"];

/// Old-layout sections a migration reads or drops, per file; they are not appended.
///
/// The aside and dispatch list lines are read wherever they are, but the old
/// layout keeps them under the per-backend and `Default backend / model /
/// effort` sections named here; `Default granularity` has no new setting and is
/// dropped.
pub fn consumed_sections(file: &str) -> &'static [&'static str] {
    match file {
        "aside" => &[
            "Preferred third-party advisor",
            "Default models (per backend)",
            "Default reasoning effort (per backend)",
            "Default model fallback chain (per backend)",
            "Auto-call policy",
        ],
        "dispatch" => &[
            "Execution policy",
            "Approval mode",
            "Default granularity",
            "Default backend / model / effort",
        ],
        _ => &[],
    }
}

/// Old aside list-item labels after the backend name, with the key each one maps to.
const ASIDE_LIST_LABELS: [(&str, &str); 3] = [
    ("default model", "model"),
    ("default reasoning effort", "effort"),
    ("default model fallback", "fallback"),
];

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
    let mut advisor_warning = None;
    match advisor.as_deref() {
        Some(b) if schema::ASIDE_BACKENDS.contains(&b) => {
            out.insert("backend".into(), b.into());
        }
        Some("none") => {
            out.insert("backend".into(), "codex".into());
            level = Some("on-request");
        }
        Some("") | None => {}
        Some(other) => {
            advisor_warning = Some(warnings.len());
            warnings.push(format!(
                "aside: unknown preferred advisor `{other}`; backend left at the default"
            ));
        }
    }
    let unknown = unknown_backends_with_values(old);
    if !unknown.is_empty() {
        let clause = format!(
            "values for {} (not a backend of this kit) were not carried over and are left in the backup of the old file",
            unknown
                .iter()
                .map(|b| format!("`{b}`"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        // One line per file: join the advisor warning when there is one.
        match advisor_warning {
            Some(i) => warnings[i] = format!("{}; {clause}", warnings[i]),
            None => warnings.push(format!("aside: {clause}")),
        }
    }
    if let Some(l) = level {
        out.insert("level".into(), l.into());
    }
    for b in schema::ASIDE_BACKENDS {
        for (label, key) in ASIDE_LIST_LABELS {
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

/// Backends named by old aside list items that the schema lacks and whose value is not blank, in file order.
///
/// A value that cannot be read counts as not blank.
fn unknown_backends_with_values(old: &PrefsDoc) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for label in old.list_labels() {
        let backend = ASIDE_LIST_LABELS.iter().find_map(|(suffix, _)| {
            label
                .trim_end()
                .strip_suffix(suffix)?
                .strip_suffix(' ')
                .filter(|b| !b.is_empty() && !b.contains(char::is_whitespace))
        });
        let Some(backend) = backend else {
            continue;
        };
        let blank = old.list_value(label).is_some_and(|v| v.trim().is_empty());
        if !schema::ASIDE_BACKENDS.contains(&backend) && !blank && !out.iter().any(|b| b == backend)
        {
            out.push(backend.to_string());
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
/// are reported), keeps the old `Notes` and `Repository overrides` verbatim, and
/// appends verbatim every other old section that the template has no heading
/// for and [`consumed_sections`] does not name.
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
    let template_headings: Vec<String> = new.headings().map(str::to_string).collect();
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
    let consumed = consumed_sections(file);
    new.append_sections_from(&old, |h| {
        !template_headings.iter().any(|t| t == h)
            && !consumed.contains(&h)
            && !PRESERVED_SECTIONS.contains(&h)
    });
    Migrated {
        text: new.render(),
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::Harness;

    const ASIDE_TEMPLATE: &str = "<!-- k-custom:aside-prefs -->\n# Aside Preferences\n\n## Level\n\n**suggest**\n\n## Backend\n\n**codex**\n\n## Codex model\n\n****\n\n## Codex reasoning effort\n\n****\n\n## Codex model fallback\n\n****\n\n## Claude model\n\n****\n\n## Claude reasoning effort\n\n****\n\n## Claude model fallback\n\n****\n\n## Notes\n\nnew default notes\n\n## Repository overrides\n\nnew default overrides\n";

    const OLD_ASIDE: &str = "<!-- claude-agent-kit-custom:aside-prefs -->\n# Aside Preferences\n\n## Preferred third-party advisor\n\nDefault backend when Claude Code decides to ask a cross-family advisor: **codex**\n\nValid values: `none` | `codex`\n\n## Default models (per backend)\n\n- codex default model: **gpt-6-astra**\n- legacy default model: ****\n- claude default model: **opus**\n\n## Default reasoning effort (per backend)\n\n- codex default reasoning effort: **high**   (`low`)\n- legacy default reasoning effort: ****\n- claude default reasoning effort: **max**\n\n## Default model fallback chain (per backend)\n\n- codex default model fallback: **gpt-6-sol(high)**   (comma-separated)\n- legacy default model fallback: ****\n- claude default model fallback: ****\n\n## Auto-call policy\n\n**proactive**\n\n## Notes\n\nMy own note.\n";

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

    #[test]
    fn list_lines_of_a_backend_the_schema_lacks_are_not_carried_over() {
        let old = "## Preferred third-party advisor\n\nDefault backend when X decides to ask: **legacy**\n\n## Default models (per backend)\n\n- codex default model: **model-a**\n- legacy default model: **model-l**\n\n## Default reasoning effort (per backend)\n\n- legacy default reasoning effort: **high**\n\n## Default model fallback chain (per backend)\n\n- legacy default model fallback: model-m   (comma-separated; blank = none)\n\n## Auto-call policy\n\n**proactive**\n";
        let m = migrate("aside", old, ASIDE_TEMPLATE, &ctx());
        // The advisor value and the list values are reported in one line.
        assert_eq!(m.warnings.len(), 1, "{:?}", m.warnings);
        assert!(
            m.warnings[0].contains("unknown preferred advisor `legacy`")
                && m.warnings[0].contains("values for `legacy`")
                && m.warnings[0].contains("backup"),
            "{:?}",
            m.warnings
        );
        let d = PrefsDoc::parse(&m.text);
        assert_eq!(d.get("Level").as_deref(), Some("auto"));
        assert_eq!(d.get("Backend").as_deref(), Some("codex"));
        assert_eq!(d.get("Codex model").as_deref(), Some("model-a"));
        assert!(!m.text.contains("model-l") && !m.text.contains("model-m"));
    }

    /// An old aside file with the codex advisor and one list line per key for `legacy`.
    fn old_aside_with_legacy_lines(model: &str, effort: &str, fallback: &str) -> String {
        format!(
            "## Preferred third-party advisor\n\nDefault backend when X decides to ask: **codex**\n\n## Default models (per backend)\n\n- codex default model: **model-a**\n- legacy default model: {model}\n\n## Default reasoning effort (per backend)\n\n- legacy default reasoning effort: {effort}   (`low` / `medium`)\n\n## Default model fallback chain (per backend)\n\n- legacy default model fallback: {fallback}   (comma-separated; blank = none)\n\n## Auto-call policy\n\n**proactive**\n"
        )
    }

    #[test]
    fn non_blank_values_of_a_backend_the_schema_lacks_are_reported_in_one_line() {
        let old = old_aside_with_legacy_lines("**model-l**", "**high**", "model-m");
        let m = migrate("aside", &old, ASIDE_TEMPLATE, &ctx());
        assert_eq!(m.warnings.len(), 1, "{:?}", m.warnings);
        assert!(
            m.warnings[0].starts_with("aside: values for `legacy` ")
                && m.warnings[0].contains("backup"),
            "{:?}",
            m.warnings
        );
        let d = PrefsDoc::parse(&m.text);
        assert_eq!(d.get("Backend").as_deref(), Some("codex"));
        assert_eq!(d.get("Codex model").as_deref(), Some("model-a"));
        assert!(!m.text.contains("model-l") && !m.text.contains("model-m"));
    }

    #[test]
    fn blank_values_of_a_backend_the_schema_lacks_are_not_reported() {
        let bold = old_aside_with_legacy_lines("****", "****", "****");
        // A bare blank value has no trailing hint, since the hint would be read as the value.
        let bare = old_aside_with_legacy_lines("", "", "")
            .replace("   (`low` / `medium`)", "")
            .replace("   (comma-separated; blank = none)", "");
        for old in [bold, bare] {
            let m = migrate("aside", &old, ASIDE_TEMPLATE, &ctx());
            assert!(m.warnings.is_empty(), "{old}: {:?}", m.warnings);
        }
    }

    #[test]
    fn an_old_section_the_template_lacks_is_kept_verbatim_at_the_end() {
        let own = "## My own section\n\nfirst line\n  second line **bold**\n";
        let other = "## Another one\n\nkept too\n";
        // The unknown sections sit between consumed ones and before Notes in the old file.
        let lf = OLD_ASIDE.replace(
            "## Auto-call policy\n",
            &format!("{own}\n{other}\n## Auto-call policy\n"),
        );
        for (old, eol) in [(lf.clone(), "\n"), (lf.replace('\n', "\r\n"), "\r\n")] {
            let m = migrate("aside", &old, ASIDE_TEMPLATE, &ctx());
            assert!(m.warnings.is_empty(), "{:?}", m.warnings);
            // Each section keeps its body up to the next old heading, trailing blank line included.
            let tail = format!("{own}\n{other}\n").replace('\n', eol);
            assert!(m.text.ends_with(&tail), "{eol:?}: {}", m.text);
            // Consumed old sections are not appended, and Notes stays at the template's heading.
            for consumed in consumed_sections("aside") {
                assert!(!m.text.contains(consumed), "{consumed}: {}", m.text);
            }
            let notes = m.text.find("## Notes").unwrap();
            assert!(notes < m.text.find("## My own section").unwrap());
            assert!(
                m.text
                    .contains(&"## Notes\n\nMy own note.\n".replace('\n', eol))
            );
        }
    }

    #[test]
    fn notes_as_the_last_old_section_stay_separated_from_the_next_heading() {
        let base = OLD_ASIDE
            .strip_suffix("My own note.\n")
            .expect("OLD_ASIDE ends with its Notes body");
        for eol in ["\n", "\r\n"] {
            // (a) no final newline, (b) a final newline, (c) a trailing blank line.
            for ending in ["", "\n", "\n\n"] {
                let old = format!("{base}My own note.{ending}").replace('\n', eol);
                let m = migrate("aside", &old, ASIDE_TEMPLATE, &ctx());
                let boundary = "\n## Notes\n\nMy own note.\n\n## Repository overrides\n\nnew default overrides\n"
                    .replace('\n', eol);
                assert!(
                    m.text.ends_with(&boundary),
                    "{ending:?} {eol:?}: {:?}",
                    m.text
                );
            }
        }
    }

    #[test]
    fn an_unterminated_last_section_is_appended_as_it_was() {
        let old = "## Execution policy\n\n**proactive**\n\n## Approval mode\n\n**auto**\n\n## Mine\n\nno newline";
        let m = migrate("dispatch", old, DISPATCH_TEMPLATE, &ctx());
        assert!(m.text.ends_with("\n\n## Mine\n\nno newline"), "{}", m.text);
        assert!(!m.text.contains("Execution policy") && !m.text.contains("Approval mode"));
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
        let old = "## Preferred third-party advisor\n\nDefault backend when X decides to ask: **codex**\n\n## Default models (per backend)\n\n- codex default model: **model-a**\n\n## Default reasoning effort (per backend)\n\n- codex default reasoning effort: **high**   (`low` / `medium`)\n\n## Default model fallback chain (per backend)\n\n- codex default model fallback: model-b(high), model-c   (comma-separated; blank = none)\n- legacy default model fallback: ****   (comma-separated; blank = none)\n- claude default model fallback: model-d  (comma-separated; blank = none)\n\n## Auto-call policy\n\n**proactive**\n";
        let m = migrate("aside", old, ASIDE_TEMPLATE, &ctx());
        assert!(m.warnings.is_empty(), "{:?}", m.warnings);
        let d = PrefsDoc::parse(&m.text);
        assert_eq!(
            d.get("Codex model fallback").as_deref(),
            Some("model-b(high), model-c")
        );
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
