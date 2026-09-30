//! `palette_status`: a bounded summary of the project or of one record.
//!
//! Owns the text of the summary and its size limit: the output never exceeds
//! [`LIMIT`] characters and names what it left out. Reads a snapshot and its analysis;
//! never touches files.
//! Entry point: [`summary`].

use std::collections::BTreeMap;

use crate::backlog::Backlog;
use crate::docs::Snapshot;
use crate::layout::Family;
use crate::lint::{Analysis, Finding, Severity};
use crate::records::{RecId, Records};
use crate::state::{EntryKind, State};

/// Hard limit on the length of the output, in characters.
pub const LIMIT: usize = 4_000;
const RESERVE: usize = 260;

struct Out {
    text: String,
    omitted: Vec<String>,
}

impl Out {
    fn used(&self) -> usize {
        self.text.chars().count()
    }

    fn fits(&self, line: &str) -> bool {
        self.used() + line.chars().count() + 1 + RESERVE <= LIMIT
    }

    /// Adds a line if it fits; otherwise records it as omitted under `what`.
    fn line(&mut self, line: String, what: &str) {
        if self.fits(&line) {
            self.text.push_str(&line);
            self.text.push('\n');
        } else {
            self.omitted.push(what.to_string());
        }
    }

    fn finish(mut self) -> String {
        if !self.omitted.is_empty() {
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            for o in &self.omitted {
                *counts.entry(o.clone()).or_default() += 1;
            }
            let list: Vec<String> = counts
                .into_iter()
                .map(|(k, v)| format!("{v} {k}"))
                .collect();
            self.text.push_str(&format!("Omitted to stay under {LIMIT} characters: {}. Use palette_lint or read the documents for the rest.\n", list.join(", ")));
        }
        let mut t = self.text;
        if t.chars().count() > LIMIT {
            t = t.chars().take(LIMIT - 1).collect();
        }
        t
    }
}

fn clip(s: &str, n: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= n {
        s
    } else {
        let cut: String = s.chars().take(n.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

/// The summary of the whole project, or of `record` when given.
pub fn summary(
    snap: &Snapshot,
    an: &Analysis,
    findings: &[Finding],
    record: Option<RecId>,
) -> String {
    let mut o = Out {
        text: String::new(),
        omitted: Vec::new(),
    };
    let errors = findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .count();
    let warnings = findings.len() - errors;
    match record {
        Some(id) => record_summary(&mut o, snap, an, id, errors),
        None => project_summary(&mut o, snap, findings, errors, warnings),
    }
    o.finish()
}

fn project_summary(
    o: &mut Out,
    snap: &Snapshot,
    findings: &[Finding],
    errors: usize,
    warnings: usize,
) {
    o.line(format!("Project: {}", snap.root.display()), "header line");
    let backlog = snap
        .single(Family::Backlog)
        .and_then(|f| f.doc.as_ref())
        .map(Backlog::parse);
    match &backlog {
        Some(b) => {
            let phases: Vec<String> = b
                .phases
                .iter()
                .map(|p| {
                    format!(
                        "phase-{} {}",
                        p.number,
                        p.state.as_deref().unwrap_or("(no state)")
                    )
                })
                .collect();
            o.line(
                format!(
                    "Phases: {}",
                    if phases.is_empty() {
                        "none".to_string()
                    } else {
                        phases.join("; ")
                    }
                ),
                "phase line",
            );
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            for it in &b.items {
                *counts
                    .entry(it.value("Status").unwrap_or("(no status)").to_string())
                    .or_default() += 1;
            }
            let items: Vec<String> = counts.iter().map(|(k, v)| format!("{k} {v}")).collect();
            o.line(
                format!(
                    "Items ({}): {}",
                    b.items.len(),
                    if items.is_empty() {
                        "none".to_string()
                    } else {
                        items.join(", ")
                    }
                ),
                "item count line",
            );
        }
        None => o.line("Backlog: missing or unreadable".to_string(), "backlog line"),
    }
    o.line(
        format!("Lint: {errors} error(s), {warnings} warning(s)"),
        "lint line",
    );
    let state = snap
        .single(Family::State)
        .and_then(|f| f.doc.as_ref())
        .map(State::parse);
    let Some(st) = state else {
        o.line("State: missing or unreadable".to_string(), "state line");
        return;
    };
    for (kind, label) in [
        (EntryKind::Question, "Open questions"),
        (EntryKind::Discrepancy, "Discrepancies"),
        (EntryKind::Decision, "Decisions not yet graduated"),
    ] {
        let entries: Vec<&crate::state::Entry> = st
            .section(kind)
            .map(|s| {
                s.entries
                    .iter()
                    .filter(|e| kind != EntryKind::Decision || !e.is_pointer())
                    .collect()
            })
            .unwrap_or_default();
        o.line(
            format!("{label} ({}):", entries.len()),
            &format!("{label} heading"),
        );
        for e in entries {
            o.line(
                format!("- {}", clip(e.text.trim_start_matches("- "), 120)),
                &label.to_lowercase(),
            );
        }
    }
    let _ = findings;
}

fn record_summary(o: &mut Out, snap: &Snapshot, an: &Analysis, id: RecId, errors: usize) {
    let recs: &Records = &an.records;
    let Some(rec) = recs.get(id) else {
        o.line(format!("{id} does not exist"), "line");
        return;
    };
    let file = &snap.files[rec.file];
    o.line(
        format!("{id}: {} ({})", clip(&rec.title, 100), file.rel),
        "header line",
    );
    for f in &rec.fields {
        o.line(
            format!("{}: {}", f.name, clip(&f.value, 150)),
            "header field",
        );
    }
    let incoming: Vec<String> = recs
        .incoming(id)
        .into_iter()
        .map(|(r, k)| format!("{r} ({})", k.label()))
        .collect();
    o.line(format!("Linked from: {}", list(&incoming)), "incoming line");
    let closure: Vec<String> = recs.closure(id).iter().map(|r| r.to_string()).collect();
    o.line(
        format!("Dependency closure: {}", list(&closure)),
        "closure line",
    );
    let sup: Vec<String> = recs
        .superseded_by(id)
        .iter()
        .map(|r| r.to_string())
        .collect();
    o.line(
        format!("Superseded by: {}", list(&sup)),
        "supersession line",
    );
    match an.sets.get(&id) {
        None => o.line(
            "Pending changeset edits: none".to_string(),
            "changeset line",
        ),
        Some(cs) => {
            o.line(
                format!("Pending changeset edits: {}", cs.edit_count()),
                "changeset line",
            );
            let accepted = rec.status.as_deref() == Some("Accepted");
            for de in &cs.docs {
                for e in &de.edits {
                    o.line(
                        format!("- {}: {} ({})", e.kind.verb(), clip(&e.target, 90), de.doc),
                        "changeset edit",
                    );
                }
                let staged = snap
                    .loc
                    .staging_file(&de.doc)
                    .map(|p| crate::util::rel_slash(&snap.root, &p))
                    .unwrap_or_default();
                let note = if accepted {
                    staged
                } else {
                    format!(
                        "not applied while {}",
                        rec.status.as_deref().unwrap_or("unknown")
                    )
                };
                o.line(format!("  staging document: {note}"), "staging line");
            }
        }
    }
    o.line(format!("Project lint errors: {errors}"), "lint line");
}

fn list(v: &[String]) -> String {
    if v.is_empty() {
        "none".to_string()
    } else {
        v.join(", ")
    }
}
