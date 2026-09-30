//! The lint engine: rules P001 to P017 over a project snapshot.
//!
//! Owns the [`Finding`] type, the derived analysis shared by the rules (records,
//! changesets, staging plan) and the ordering of results. Each rule group lives in its
//! own submodule; the heuristic patterns are in `patterns`. Does not write files.
//! Entry points: [`run`], [`Analysis::build`], [`filter_paths`].

mod changes;
mod directive;
mod links;
pub mod patterns;
mod relations;
mod stray;
mod structure;
mod syntax;
mod work;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::changeset::{self, Changeset, StagingPlan};
use crate::docs::{DocFile, Snapshot};
use crate::records::{RecId, Records};
use crate::vfs::FileSource;

/// `error` or `warning`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Must be fixed; blocks write tools.
    Error,
    /// Worth fixing; never blocks.
    Warning,
}

impl Severity {
    /// The wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

/// One lint finding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// Rule id, `P001` to `P017`.
    pub rule: &'static str,
    /// Severity.
    pub severity: Severity,
    /// Project-relative file with `/` separators.
    pub file: String,
    /// 1-based line.
    pub line: usize,
    /// What is wrong and how to fix it.
    pub message: String,
}

impl Finding {
    pub(crate) fn error(
        rule: &'static str,
        f: &DocFile,
        line0: usize,
        message: impl Into<String>,
    ) -> Finding {
        Finding {
            rule,
            severity: Severity::Error,
            file: f.rel.clone(),
            line: line0 + 1,
            message: message.into(),
        }
    }

    pub(crate) fn warning(
        rule: &'static str,
        f: &DocFile,
        line0: usize,
        message: impl Into<String>,
    ) -> Finding {
        Finding {
            severity: Severity::Warning,
            ..Finding::error(rule, f, line0, message)
        }
    }

    pub(crate) fn at(
        rule: &'static str,
        severity: Severity,
        rel: &str,
        line0: usize,
        message: impl Into<String>,
    ) -> Finding {
        Finding {
            rule,
            severity,
            file: rel.to_string(),
            line: line0 + 1,
            message: message.into(),
        }
    }
}

/// Facts derived from a snapshot that several rules and the write tools share.
pub struct Analysis {
    /// Records with their relations.
    pub records: Records,
    /// Changesets by record.
    pub sets: BTreeMap<RecId, Changeset>,
    /// The staging plan.
    pub plan: StagingPlan,
}

impl Analysis {
    /// Derives records, changesets and the staging plan from `snap`.
    pub fn build(snap: &Snapshot) -> Analysis {
        let records = Records::build(snap);
        let sets = changeset::collect(snap);
        let plan = changeset::plan(snap, &records, &sets);
        Analysis {
            records,
            sets,
            plan,
        }
    }
}

pub(crate) struct Cx<'a> {
    pub src: &'a dyn FileSource,
    pub snap: &'a Snapshot,
    pub an: &'a Analysis,
}

/// Runs every rule; errors come first, then by file and line.
pub fn run(src: &dyn FileSource, snap: &Snapshot, an: &Analysis) -> Vec<Finding> {
    let cx = Cx { src, snap, an };
    let mut out = Vec::new();
    syntax::check(&cx, &mut out);
    directive::check(&cx, &mut out);
    structure::check(&cx, &mut out);
    links::check(&cx, &mut out);
    relations::check(&cx, &mut out);
    changes::check(&cx, &mut out);
    work::check(&cx, &mut out);
    stray::check(&cx, &mut out);
    out.sort_by(|a, b| {
        (a.severity, &a.file, a.line, a.rule, &a.message)
            .cmp(&(b.severity, &b.file, b.line, b.rule, &b.message))
    });
    out.dedup();
    out
}

/// Keeps the findings whose file is one of `paths`, or lies inside one of them when a
/// path is a folder. Paths are project-relative (`/`) or absolute.
pub fn filter_paths(findings: Vec<Finding>, root: &Path, paths: &[PathBuf]) -> Vec<Finding> {
    let wanted: Vec<String> = paths
        .iter()
        .map(|p| {
            let rel = p.strip_prefix(root).unwrap_or(p);
            rel.components()
                .filter_map(|c| match c {
                    std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect();
    findings
        .into_iter()
        .filter(|f| {
            wanted
                .iter()
                .any(|w| w.is_empty() || f.file == *w || f.file.starts_with(&format!("{w}/")))
        })
        .collect()
}

/// Renders findings as one JSON object per line inside a JSON document.
pub fn to_json(findings: &[Finding]) -> String {
    let errors = findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .count();
    let warnings = findings.len() - errors;
    let mut out = format!("{{\"errors\": {errors}, \"warnings\": {warnings}, \"findings\": [");
    for (i, f) in findings.iter().enumerate() {
        out.push_str(if i == 0 { "\n  " } else { ",\n  " });
        out.push_str(
            &serde_json::json!({
                "rule": f.rule,
                "severity": f.severity.as_str(),
                "file": f.file,
                "line": f.line,
                "message": f.message,
            })
            .to_string(),
        );
    }
    out.push_str(if findings.is_empty() { "]}" } else { "\n]}" });
    out
}

/// Renders findings for the command line: `file:line: severity Pnnn message`.
pub fn to_text(findings: &[Finding]) -> String {
    let mut out = String::new();
    for f in findings {
        out.push_str(&format!(
            "{}:{}: {} {} {}\n",
            f.file,
            f.line,
            f.severity.as_str(),
            f.rule,
            f.message
        ));
    }
    let errors = findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .count();
    out.push_str(&format!(
        "{errors} error(s), {} warning(s)\n",
        findings.len() - errors
    ));
    out
}
