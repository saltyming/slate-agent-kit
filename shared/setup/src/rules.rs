//! Custom rule files and the combined primary file for harnesses that load only one file.
//!
//! Owns copying the user's own `*.md` rules into `rules/<kit>--<name>.md` with a
//! user-owned signature, refusing to replace kit-managed files, and generating
//! the combined primary file (`load = concat`) from scratch so a custom rule
//! appears exactly once however often install or configure runs. It does not
//! write files; `install` applies the plan.
//!
//! Main entry points: [`plan_custom_rules`], [`effective_custom_rules`] and
//! [`build_concat`].

use crate::error::{Error, IoContext, Result};
use crate::signature::{Owner, classify, ensure_custom_signature};
use std::fs;
use std::path::{Path, PathBuf};

/// What happens to one custom rule file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomDecision {
    /// The destination does not exist yet.
    Create,
    /// The destination is an earlier copy of this rule and is updated.
    Replace,
    /// The destination already has this content.
    Unchanged,
    /// The destination must not be touched; the string says why.
    Refused(String),
}

/// One custom rule file to install.
#[derive(Debug, Clone)]
pub struct CustomRule {
    /// The user's file.
    pub src: PathBuf,
    /// The file in `rules/`.
    pub dest: PathBuf,
    /// The text to write, with the user-owned signature.
    pub text: String,
    /// What happens to it.
    pub decision: CustomDecision,
}

/// The name a custom rule gets in `rules/`: the source name, prefixed with `<kit>--` unless it already is.
pub fn dest_name(kit: &str, src_name: &str) -> String {
    let prefix = format!("{kit}--");
    if src_name.starts_with(&prefix) {
        src_name.to_string()
    } else {
        format!("{prefix}{src_name}")
    }
}

/// A destination the kit itself installs, which a custom rule must not take.
#[derive(Debug, Clone)]
pub struct Reserved {
    /// The destination path.
    pub path: PathBuf,
    /// Why it is off limits, for the report.
    pub reason: &'static str,
}

/// Plans the copy of every `*.md` in `folder` into `rules_dir`.
///
/// `prior` lists the custom rules an earlier run installed; those may be
/// replaced. A destination that is kit-managed, or user-owned but not an
/// earlier copy of a custom rule (a prefs file, a file the user wrote there),
/// is refused.
pub fn plan_custom_rules(
    kit: &str,
    rules_dir: &Path,
    folder: &Path,
    prior: &[PathBuf],
    reserved: &[Reserved],
) -> Result<Vec<CustomRule>> {
    let entries = fs::read_dir(folder).map_err(|e| {
        Error::usage(format!(
            "the custom rules folder {} cannot be read: {e}",
            folder.display()
        ))
    })?;
    let mut sources: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("md")))
        .collect();
    sources.sort();
    let mut out = Vec::new();
    for src in sources {
        let name = src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let target = rules_dir.join(dest_name(kit, &name));
        if let Some(r) = reserved.iter().find(|r| r.path == target) {
            out.push(CustomRule {
                dest: target,
                src,
                text: String::new(),
                decision: CustomDecision::Refused(r.reason.to_string()),
            });
            continue;
        }
        let raw = fs::read(&src).ctx(|| format!("reading {}", src.display()))?;
        let raw = String::from_utf8(raw)
            .map_err(|_| Error::usage(format!("{} is not valid UTF-8", src.display())))?;
        let text = ensure_custom_signature(kit, "user", &raw);
        let dest = rules_dir.join(dest_name(kit, &name));
        let decision = match fs::read(&dest) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => CustomDecision::Create,
            Err(e) => return Err(Error::io(format!("reading {}", dest.display()), e)),
            Ok(bytes) => {
                let existing = String::from_utf8_lossy(&bytes).into_owned();
                match classify(kit, &existing) {
                    Owner::Managed => {
                        CustomDecision::Refused("it would replace a kit-managed file".into())
                    }
                    _ if existing == text => CustomDecision::Unchanged,
                    Owner::User if prior.iter().any(|p| p == &dest) => CustomDecision::Replace,
                    _ => CustomDecision::Refused("a file you own already has that name".into()),
                }
            }
        };
        out.push(CustomRule {
            src,
            dest,
            text,
            decision,
        });
    }
    Ok(out)
}

/// The custom rules that belong in the combined primary file: those installed
/// earlier (still present and user-owned) plus the ones this run installs.
///
/// Returns `(destination, text)` sorted by destination file name.
pub fn effective_custom_rules(
    kit: &str,
    prior: &[PathBuf],
    planned: &[CustomRule],
) -> Vec<(PathBuf, String)> {
    let mut out: Vec<(PathBuf, String)> = Vec::new();
    for p in prior {
        if planned.iter().any(|c| &c.dest == p) {
            continue;
        }
        if let Ok(text) = fs::read_to_string(p)
            && classify(kit, &text) == Owner::User
        {
            out.push((p.clone(), text));
        }
    }
    for c in planned {
        if !matches!(c.decision, CustomDecision::Refused(_)) {
            out.push((c.dest.clone(), c.text.clone()));
        }
    }
    out.sort_by(|a, b| a.0.file_name().cmp(&b.0.file_name()));
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

/// Joins the primary file, the rule files and the custom rules with `---` separators.
///
/// Every chunk is separated from the previous one by a blank line, a line `---`
/// and a blank line; the result ends with one newline.
pub fn build_concat(primary: &str, rules: &[String], custom: &[String]) -> String {
    let mut chunks: Vec<&str> = vec![primary];
    chunks.extend(rules.iter().map(String::as_str));
    chunks.extend(custom.iter().map(String::as_str));
    let mut out = chunks
        .iter()
        .map(|c| c.trim_end_matches(['\n', '\r']))
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, text).unwrap();
        p
    }

    #[test]
    fn concat_matches_the_old_shape() {
        let text = build_concat(
            "# Primary\n",
            &["rule one\n".into(), "rule two\n".into()],
            &["custom\n".into()],
        );
        assert_eq!(
            text,
            "# Primary\n\n---\n\nrule one\n\n---\n\nrule two\n\n---\n\ncustom\n"
        );
        assert_eq!(build_concat("only", &[], &[]), "only\n");
    }

    #[test]
    fn destination_names_are_prefixed_once() {
        assert_eq!(dest_name("k", "style.md"), "k--style.md");
        assert_eq!(dest_name("k", "k--style.md"), "k--style.md");
    }

    #[test]
    fn new_rules_get_the_signature_and_the_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("mine");
        let rules = dir.path().join("rules");
        fs::create_dir_all(&folder).unwrap();
        fs::create_dir_all(&rules).unwrap();
        write(&folder, "style.md", "# Style\n");
        write(&folder, "signed.md", "<!-- k-custom:mine -->\n# Signed\n");
        write(&folder, "notes.txt", "ignored");
        let plan = plan_custom_rules("k", &rules, &folder, &[], &[]).unwrap();
        assert_eq!(plan.len(), 2);
        let signed = plan
            .iter()
            .find(|c| c.dest.ends_with("k--signed.md"))
            .unwrap();
        assert_eq!(signed.text, "<!-- k-custom:mine -->\n# Signed\n");
        let style = plan
            .iter()
            .find(|c| c.dest.ends_with("k--style.md"))
            .unwrap();
        assert_eq!(style.text, "<!-- k-custom:user -->\n# Style\n");
        assert!(plan.iter().all(|c| c.decision == CustomDecision::Create));
    }

    #[test]
    fn kit_managed_and_foreign_files_are_refused_earlier_copies_are_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("mine");
        let rules = dir.path().join("rules");
        fs::create_dir_all(&folder).unwrap();
        fs::create_dir_all(&rules).unwrap();
        write(&folder, "task-execution.md", "# Mine\n");
        write(&folder, "old.md", "# New text\n");
        write(&folder, "foreign.md", "# Foreign\n");
        write(&folder, "same.md", "# Same\n");
        write(
            &rules,
            "k--task-execution.md",
            "<!-- slate-agent-kit:common -->\nkit rule\n",
        );
        let old = write(&rules, "k--old.md", "<!-- k-custom:user -->\n# Old text\n");
        write(
            &rules,
            "k--foreign.md",
            "<!-- k-custom:git-prefs -->\nsomething else\n",
        );
        write(&rules, "k--same.md", "<!-- k-custom:user -->\n# Same\n");
        let plan =
            plan_custom_rules("k", &rules, &folder, std::slice::from_ref(&old), &[]).unwrap();
        let by = |n: &str| {
            plan.iter()
                .find(|c| c.dest.ends_with(n))
                .unwrap()
                .decision
                .clone()
        };
        assert!(
            matches!(by("k--task-execution.md"), CustomDecision::Refused(m) if m.contains("kit-managed"))
        );
        assert_eq!(by("k--old.md"), CustomDecision::Replace);
        assert!(matches!(by("k--foreign.md"), CustomDecision::Refused(_)));
        assert_eq!(by("k--same.md"), CustomDecision::Unchanged);
    }

    #[test]
    fn destinations_the_kit_installs_are_reserved_even_before_they_exist() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("mine");
        let rules = dir.path().join("rules");
        fs::create_dir_all(&folder).unwrap();
        fs::create_dir_all(&rules).unwrap();
        write(&folder, "git-prefs.md", "# My git prefs\n");
        write(&folder, "task-execution.md", "# My take\n");
        write(&folder, "other.md", "# Other\n");
        let reserved = vec![
            Reserved {
                path: rules.join("k--git-prefs.md"),
                reason: "the name is reserved for a prefs file",
            },
            Reserved {
                path: rules.join("k--task-execution.md"),
                reason: "it would replace a kit-managed file",
            },
        ];
        let plan = plan_custom_rules("k", &rules, &folder, &[], &reserved).unwrap();
        let by = |n: &str| {
            plan.iter()
                .find(|c| c.dest.ends_with(n))
                .unwrap()
                .decision
                .clone()
        };
        assert!(matches!(by("k--git-prefs.md"), CustomDecision::Refused(m) if m.contains("prefs")));
        assert!(
            matches!(by("k--task-execution.md"), CustomDecision::Refused(m) if m.contains("kit-managed"))
        );
        assert_eq!(by("k--other.md"), CustomDecision::Create);
    }

    #[test]
    fn effective_rules_include_earlier_ones_once_and_sort_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let rules = dir.path().join("rules");
        fs::create_dir_all(&rules).unwrap();
        let b = write(&rules, "k--b.md", "<!-- k-custom:user -->\nB\n");
        let planned = vec![
            CustomRule {
                src: PathBuf::from("a.md"),
                dest: rules.join("k--a.md"),
                text: "<!-- k-custom:user -->\nA\n".into(),
                decision: CustomDecision::Create,
            },
            CustomRule {
                src: PathBuf::from("b.md"),
                dest: b.clone(),
                text: "<!-- k-custom:user -->\nB2\n".into(),
                decision: CustomDecision::Replace,
            },
            CustomRule {
                src: PathBuf::from("c.md"),
                dest: rules.join("k--c.md"),
                text: String::new(),
                decision: CustomDecision::Refused("x".into()),
            },
        ];
        let eff = effective_custom_rules("k", std::slice::from_ref(&b), &planned);
        let names: Vec<_> = eff
            .iter()
            .map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["k--a.md", "k--b.md"]);
        assert!(
            eff[1].1.contains("B2"),
            "the planned text wins over the old copy"
        );
        // Earlier rules that are no longer in the folder still belong to the combined file.
        let eff = effective_custom_rules("k", std::slice::from_ref(&b), &[]);
        assert_eq!(eff.len(), 1);
        assert!(eff[0].1.contains("B\n"));
    }

    #[test]
    fn a_missing_folder_is_a_usage_error() {
        let dir = tempfile::tempdir().unwrap();
        let err =
            plan_custom_rules("k", dir.path(), &dir.path().join("nope"), &[], &[]).unwrap_err();
        assert!(err.to_string().contains("cannot be read"));
    }
}
