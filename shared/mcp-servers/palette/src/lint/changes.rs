//! P007 changesets and P008 generated files.
//!
//! P007: edits that do not resolve, independent changesets editing one section, a
//! changeset without a record, and a `complete` record that still has edits.
//! P008: indexes and staging documents that differ from what the server generates.

use super::{Cx, Finding, Severity};
use crate::changeset::{self, Changeset};
use crate::docs::Role;
use crate::generated;
use crate::records::Records;
use crate::util::rel_slash;

pub(super) fn check(cx: &Cx, out: &mut Vec<Finding>) {
    changesets(cx, out);
    generated_files(cx, out);
}

fn changesets(cx: &Cx, out: &mut Vec<Finding>) {
    let recs = &cx.an.records;
    for (i, f) in cx.snap.files.iter().enumerate() {
        if f.role != Role::Changeset {
            continue;
        }
        let Some(cs) = Changeset::parse(cx.snap, i) else {
            continue;
        };
        for (line, msg) in &cs.problems {
            out.push(Finding::error("P007", f, *line, msg.clone()));
        }
        match cs.record() {
            None => out.push(Finding::error(
                "P007",
                f,
                0,
                "the changeset does not name a record (`Changeset: RFC-NNNN`)",
            )),
            Some(id) if recs.get(id).is_none() => {
                out.push(Finding::error(
                    "P007",
                    f,
                    0,
                    format!("changeset has no record: {id} does not exist"),
                ));
            }
            Some(_) => {}
        }
    }
    for fail in changeset::check_resolution(cx.snap, recs, &cx.an.sets) {
        out.push(Finding::error(
            "P007",
            &cx.snap.files[fail.file],
            fail.line,
            format!(
                "edit does not resolve for {}: {}",
                fail.record, fail.message
            ),
        ));
    }
    for (a, b, doc, target) in changeset::find_conflicts(recs, &cx.an.sets) {
        // Report on both changesets, so the write that introduces the conflict is the one refused.
        for (mine, other) in [(a, b), (b, a)] {
            let Some(cs) = cx.an.sets.get(&mine) else {
                continue;
            };
            let line = cs
                .docs
                .iter()
                .filter(|d| d.doc == doc)
                .flat_map(|d| d.edits.iter())
                .find(|e| e.target == target || target == "(new document)")
                .map(|e| e.line)
                .unwrap_or(0);
            out.push(Finding::error(
                "P007",
                &cx.snap.files[cs.file],
                line,
                format!("{mine} and {other} both edit `{target}` in `{doc}` and neither depends on the other; add a Depends between them"),
            ));
        }
    }
    for r in &recs.list {
        let Some(id) = Records::id_of(r) else {
            continue;
        };
        let token = r
            .field("Implementation")
            .map(|f| f.value.split(" — ").next().unwrap_or("").trim().to_string());
        if token.as_deref() == Some("complete")
            && let Some(cs) = cx.an.sets.get(&id)
            && cs.edit_count() > 0
        {
            let line = r.field("Implementation").map(|f| f.start).unwrap_or(0);
            out.push(Finding::error(
                "P007",
                &cx.snap.files[r.file],
                line,
                format!(
                    "{id} is marked complete but its changeset still has {} edit(s); promote them",
                    cs.edit_count()
                ),
            ));
        }
    }
}

fn generated_files(cx: &Cx, out: &mut Vec<Finding>) {
    let gen_files = generated::generate(cx.snap, &cx.an.records, &cx.an.sets, &cx.an.plan);
    for (path, src) in &gen_files {
        let rel = rel_slash(&cx.snap.root, path);
        match cx.snap.file(path) {
            None => out.push(Finding::at(
                "P008",
                Severity::Error,
                &rel,
                0,
                "generated file is missing; any palette write tool regenerates it",
            )),
            Some(existing) => {
                let same = existing
                    .doc
                    .as_ref()
                    .is_some_and(|d| generated::same_text(&d.src, src));
                if !same {
                    out.push(Finding::error(
                        "P008",
                        existing,
                        0,
                        "generated file differs from what the server would generate; do not edit it by hand, any palette write tool regenerates it",
                    ));
                }
            }
        }
    }
    for f in &cx.snap.files {
        if matches!(f.role, Role::Staging(_)) && !gen_files.contains_key(&f.path) {
            out.push(Finding::error(
                "P008",
                f,
                0,
                "staging document that no accepted changeset produces; remove it (staging is only generated)",
            ));
        }
    }
}
