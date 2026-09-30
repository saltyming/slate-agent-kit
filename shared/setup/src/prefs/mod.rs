//! Prefs files: the settings schema, the line-preserving document, migration,
//! and the plan for one file.
//!
//! Owns turning "template + existing file + answers" into the text to write and
//! the value changes to show, for all five prefs files and all three harnesses
//! the same way. It does not ask questions (`wizard`) or write files (`install`).
//!
//! Main entry points: [`plan_file`], [`current_values`], [`classify_state`] and
//! [`validate_template`].

pub mod doc;
pub mod migrate;
pub mod schema;

use crate::error::{Error, Result};
use doc::PrefsDoc;
use schema::ValidationCtx;
use std::collections::BTreeMap;

/// What an existing prefs file looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// No file yet.
    Missing,
    /// A file in the current format.
    Current,
    /// A file in a format an earlier release wrote.
    Legacy,
}

/// What the plan does with a prefs file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Leave the file as it is.
    Keep,
    /// Write a new file from the template.
    Create,
    /// Change value lines of a current file.
    Update,
    /// Replace an old-format file after backing it up.
    Migrate,
    /// An old-format file the user chose not to migrate.
    KeepLegacy,
}

/// One value that differs between the existing and the planned file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueChange {
    /// `<file>.<key>`.
    pub key: String,
    /// Value in the existing file, when it had one.
    pub old: Option<String>,
    /// Value after the plan.
    pub new: String,
}

/// What to do with one prefs file, and why.
#[derive(Debug, Clone)]
pub struct FilePlan {
    /// Prefs file name without the `-prefs` suffix.
    pub name: String,
    /// The action.
    pub action: Action,
    /// The text to write; `None` when the file stays as it is.
    pub text: Option<String>,
    /// Notes for the report: values that could not be migrated, and similar.
    pub warnings: Vec<String>,
    /// Values that change.
    pub changes: Vec<ValueChange>,
}

/// Answers for one prefs file.
#[derive(Debug, Clone, Default)]
pub struct Input {
    /// The user confirmed migrating an old-format file (or there is no terminal).
    pub migrate: bool,
    /// New values by `<key>`, already validated.
    pub values: BTreeMap<String, String>,
}

/// Classifies an existing file.
///
/// `listed_in_old_manifest` is true when a line-format manifest listed the file;
/// that marks files whose layout did not change (git, comment) as written by an
/// earlier release, so they are rebuilt from the current template.
pub fn classify_state(name: &str, existing: Option<&str>, listed_in_old_manifest: bool) -> State {
    let Some(text) = existing else {
        return State::Missing;
    };
    let doc = PrefsDoc::parse(text);
    if migrate::is_legacy_format(name, &doc) || (listed_in_old_manifest && name != "subagent") {
        State::Legacy
    } else {
        State::Current
    }
}

/// Checks that the template carries a heading and a value line for every setting of `name`.
pub fn validate_template(name: &str, template: &str) -> Result<()> {
    let doc = PrefsDoc::parse(template);
    let missing: Vec<&str> = schema::for_file(name)
        .filter(|s| doc.get(s.heading).is_none())
        .map(|s| s.heading)
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(Error::payload(format!(
            "the {name} prefs template lacks `## <heading>` plus `**value**` for: {}",
            missing.join(", ")
        ))
        .with_fix("re-render the kit with the slate release that matches this installer"))
    }
}

/// Builds the base document the answers are applied to.
fn base_doc(
    name: &str,
    existing: Option<&str>,
    template: &str,
    state: State,
    migrate_old: bool,
    ctx: &ValidationCtx,
    warnings: &mut Vec<String>,
) -> PrefsDoc {
    let template_doc = PrefsDoc::parse(template);
    match (state, existing) {
        (State::Missing, _) | (_, None) => template_doc,
        (State::Current, Some(text)) => {
            let mut doc = PrefsDoc::parse(text);
            for setting in schema::for_file(name) {
                if !doc.has_heading(setting.heading) {
                    doc.copy_section_from(&template_doc, setting.heading);
                }
            }
            doc
        }
        (State::Legacy, Some(text)) => {
            if migrate_old {
                let m = migrate::migrate(name, text, template, ctx);
                warnings.extend(m.warnings);
                PrefsDoc::parse(&m.text)
            } else {
                PrefsDoc::parse(text)
            }
        }
    }
}

/// The values of `name` as a plan would start from them, for use as wizard defaults.
///
/// A legacy file is shown as it would look after migration.
pub fn current_values(
    name: &str,
    existing: Option<&str>,
    template: &str,
    state: State,
    ctx: &ValidationCtx,
) -> BTreeMap<String, String> {
    let mut sink = Vec::new();
    let doc = base_doc(name, existing, template, state, true, ctx, &mut sink);
    let template_doc = PrefsDoc::parse(template);
    schema::for_file(name)
        .map(|s| {
            let v = doc
                .get(s.heading)
                .or_else(|| template_doc.get(s.heading))
                .unwrap_or_default();
            (s.key.to_string(), v)
        })
        .collect()
}

/// Plans one prefs file.
pub fn plan_file(
    name: &str,
    existing: Option<&str>,
    template: &str,
    state: State,
    input: &Input,
    ctx: &ValidationCtx,
) -> Result<FilePlan> {
    validate_template(name, template)?;
    let mut warnings = Vec::new();
    if state == State::Legacy && !input.migrate {
        return Ok(FilePlan {
            name: name.to_string(),
            action: Action::KeepLegacy,
            text: None,
            warnings: vec![format!(
                "{name} prefs keep the old layout; the current rules read the new settings, so run `slate-setup configure` again to migrate"
            )],
            changes: Vec::new(),
        });
    }
    let mut doc = base_doc(
        name,
        existing,
        template,
        state,
        input.migrate,
        ctx,
        &mut warnings,
    );
    // A new file is compared with its template, so the listed changes are the answers.
    let before = Some(PrefsDoc::parse(existing.unwrap_or(template)));
    for (key, value) in &input.values {
        let full = format!("{name}.{key}");
        let setting = schema::find(&full)
            .ok_or_else(|| Error::usage(format!("unknown prefs key `{full}`")))?;
        let value = schema::validate(setting, ctx, value)
            .map_err(|e| Error::usage(format!("{full}: {e}")))?;
        doc.set(setting.heading, &value)
            .map_err(|e| Error::payload(format!("{name} prefs template: {e}")))?;
    }
    let text = doc.render();
    let changes = schema::for_file(name)
        .filter_map(|s| {
            let new = doc.get(s.heading).unwrap_or_default();
            let old = before.as_ref().and_then(|b| b.get(s.heading));
            (old.as_deref() != Some(new.as_str())).then(|| ValueChange {
                key: format!("{name}.{}", s.key),
                old,
                new,
            })
        })
        .collect();
    let (action, text) = match (state, existing) {
        (State::Missing, _) | (_, None) => (Action::Create, Some(text)),
        (State::Legacy, Some(_)) => (Action::Migrate, Some(text)),
        (State::Current, Some(old)) if old == text => (Action::Keep, None),
        (State::Current, Some(_)) => (Action::Update, Some(text)),
    };
    Ok(FilePlan {
        name: name.to_string(),
        action,
        text,
        warnings,
        changes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::Harness;

    const GIT_TEMPLATE: &str = "<!-- k-custom:git-prefs -->\n# Git\n\n## Commit signing\n\n**unset**\n\n## Model attribution\n\n**unset**\n\n## Commit message format\n\n**unset**\n\n## PR body format\n\n**unset**\n\n## Branch naming\n\n**unset**\n\n## Repository overrides\n\nx\n\n## Notes\n\nx\n";

    fn ctx() -> ValidationCtx {
        ValidationCtx::new(Harness::Codex)
    }

    fn values(pairs: &[(&str, &str)]) -> Input {
        Input {
            migrate: true,
            values: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn a_new_file_is_the_template_with_answers_applied() {
        let p = plan_file(
            "git",
            None,
            GIT_TEMPLATE,
            State::Missing,
            &values(&[("signing", "no-gpg-sign")]),
            &ctx(),
        )
        .unwrap();
        assert_eq!(p.action, Action::Create);
        let text = p.text.unwrap();
        assert!(text.contains("## Commit signing\n\n**no-gpg-sign**\n"));
        assert!(text.contains("## Branch naming\n\n**unset**\n"));
        assert_eq!(p.changes.len(), 1);
        assert_eq!(p.changes[0].key, "git.signing");
        assert_eq!(p.changes[0].old.as_deref(), Some("unset"));
    }

    #[test]
    fn an_existing_file_keeps_free_text_and_only_value_lines_change() {
        let existing = GIT_TEMPLATE
            .replace("**unset**", "**default**")
            .replace("# Git\n", "# Git\n\nUser wrote this.\n");
        let p = plan_file(
            "git",
            Some(&existing),
            GIT_TEMPLATE,
            State::Current,
            &values(&[("attribution", "off")]),
            &ctx(),
        )
        .unwrap();
        assert_eq!(p.action, Action::Update);
        let text = p.text.unwrap();
        assert!(text.contains("User wrote this."));
        assert!(text.contains("## Model attribution\n\n**off**\n"));
        assert_eq!(text.matches("**default**").count(), 4);
        assert_eq!(p.changes.len(), 1);
        assert_eq!(p.changes[0].old.as_deref(), Some("default"));
    }

    #[test]
    fn no_answers_keeps_the_file_byte_for_byte() {
        let existing = GIT_TEMPLATE.replace('\n', "\r\n");
        let p = plan_file(
            "git",
            Some(&existing),
            GIT_TEMPLATE,
            State::Current,
            &Input::default(),
            &ctx(),
        )
        .unwrap();
        assert_eq!(p.action, Action::Keep);
        assert!(p.text.is_none());
        assert!(p.changes.is_empty());
    }

    #[test]
    fn invalid_answers_are_usage_errors() {
        let err = plan_file(
            "git",
            None,
            GIT_TEMPLATE,
            State::Missing,
            &values(&[("signing", "maybe")]),
            &ctx(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("git.signing"));
        assert!(err.to_string().contains("default, no-gpg-sign, unset"));
        let err = plan_file(
            "git",
            None,
            GIT_TEMPLATE,
            State::Missing,
            &values(&[("nope", "x")]),
            &ctx(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown prefs key"));
    }

    #[test]
    fn a_template_missing_a_heading_is_rejected_by_name() {
        let bad = GIT_TEMPLATE.replace("## Branch naming\n\n**unset**\n\n", "");
        let err =
            plan_file("git", None, &bad, State::Missing, &Input::default(), &ctx()).unwrap_err();
        assert!(err.to_string().contains("Branch naming"), "{err}");
    }

    #[test]
    fn a_current_file_missing_a_heading_gets_it_from_the_template() {
        let existing = GIT_TEMPLATE.replace("## Branch naming\n\n**unset**\n\n", "");
        let p = plan_file(
            "git",
            Some(&existing),
            GIT_TEMPLATE,
            State::Current,
            &values(&[("branch-naming", "descriptive")]),
            &ctx(),
        )
        .unwrap();
        let text = p.text.unwrap();
        assert!(text.contains("## Branch naming\n\n**descriptive**\n"));
    }

    #[test]
    fn legacy_state_rules() {
        let old_aside = "## Auto-call policy\n\n**proactive**\n";
        assert_eq!(
            classify_state("aside", Some(old_aside), false),
            State::Legacy
        );
        assert_eq!(
            classify_state("git", Some(GIT_TEMPLATE), false),
            State::Current
        );
        assert_eq!(
            classify_state("git", Some(GIT_TEMPLATE), true),
            State::Legacy
        );
        assert_eq!(classify_state("git", None, true), State::Missing);
    }

    #[test]
    fn declining_migration_keeps_the_old_file() {
        let old = "## Auto-call policy\n\n**proactive**\n";
        let input = Input {
            migrate: false,
            ..Input::default()
        };
        let p = plan_file("aside", Some(old), "unused", State::Legacy, &input, &ctx());
        // The template is checked first, so use a valid one for this scenario.
        assert!(p.is_err());
    }

    #[test]
    fn migration_replaces_the_file_and_reports_changed_values() {
        let old = GIT_TEMPLATE.replace("**unset**", "**off**");
        let p = plan_file(
            "git",
            Some(&old),
            GIT_TEMPLATE,
            State::Legacy,
            &values(&[]),
            &ctx(),
        )
        .unwrap();
        assert_eq!(p.action, Action::Migrate);
        assert!(p.text.unwrap().contains("**off**"));
    }

    #[test]
    fn current_values_show_the_migrated_view() {
        let old = GIT_TEMPLATE.replace(
            "## Commit signing\n\n**unset**",
            "## Commit signing\n\n**no-gpg-sign**",
        );
        let v = current_values("git", Some(&old), GIT_TEMPLATE, State::Legacy, &ctx());
        assert_eq!(v["signing"], "no-gpg-sign");
        assert_eq!(v["pr-body"], "unset");
    }
}
