//! P004: links whose target file or anchor is missing, and project documents that
//! link into `_palette/`. Also reports header relations that name a record that does
//! not exist.
//!
//! A link inside a changeset's edit body is text of the edit's target document: it is
//! read from that document's folder, and its target may be a document or section that
//! the record's own edits or its dependency closure's edits create.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::{Cx, Finding};
use crate::changeset::Changeset;
use crate::docs::Role;
use crate::rst::{Doc, is_external, split_anchor};
use crate::util::join_lexical;

fn parent(p: &Path) -> PathBuf {
    p.parent().map(Path::to_path_buf).unwrap_or_default()
}

pub(super) fn check(cx: &Cx, out: &mut Vec<Finding>) {
    for (idx, f) in cx.snap.files.iter().enumerate() {
        if matches!(f.role, Role::Staging(_)) {
            continue;
        }
        let Some(doc) = &f.doc else { continue };
        let dir = parent(&f.path);
        let internal_source = cx.snap.loc.is_internal(&f.path);
        let edits = if f.role == Role::Changeset {
            Changeset::parse(cx.snap, idx)
        } else {
            None
        };
        // The record's document state, when this file is the changeset it was built from.
        let state = edits.as_ref().and_then(|cs| {
            let id = cs.record()?;
            if cx.an.sets.get(&id)?.file != idx {
                return None;
            }
            cx.an.states.get(&id)
        });
        // Deliverable links in backlog items belong to P013.
        let mut skip_lines: BTreeSet<usize> = BTreeSet::new();
        if f.role == Role::Backlog {
            let b = crate::backlog::Backlog::parse(doc);
            for it in &b.items {
                if let Some(fld) = it.field("Deliverable") {
                    skip_lines.extend(fld.start..fld.end);
                }
            }
        }
        for l in doc.links() {
            if is_external(&l.target) || skip_lines.contains(&l.line) {
                continue;
            }
            let (path, anchor) = split_anchor(&l.target);
            let body_doc = edits
                .as_ref()
                .and_then(|cs| cs.body_target(doc, l.line))
                .and_then(|logical| cx.snap.loc.maintained_file(logical));
            let (from_file, from_dir, internal_source) = match &body_doc {
                Some(p) => (p.clone(), parent(p), cx.snap.loc.is_internal(p)),
                None => (f.path.clone(), dir.clone(), internal_source),
            };
            let target = if path.is_empty() {
                Some(from_file)
            } else {
                join_lexical(&from_dir, path)
            };
            let Some(target) = target else {
                out.push(Finding::error(
                    "P004",
                    f,
                    l.line,
                    format!("link `{}` leaves the file system root", l.target),
                ));
                continue;
            };
            if !internal_source && target.starts_with(cx.snap.loc.palette_dir()) {
                out.push(Finding::error(
                    "P004",
                    f,
                    l.line,
                    format!("a document under a project path must not link into `_palette/` (`{}`); that folder is not committed", l.target),
                ));
                continue;
            }
            let held = body_doc
                .as_ref()
                .and(state)
                .and_then(|st| st.held_at(cx.snap, &target).map(|l| (st, l)));
            let exists = held.is_some()
                || cx.src.read(&target).ok().flatten().is_some()
                || cx.src.is_dir(&target);
            if !exists {
                out.push(Finding::error(
                    "P004",
                    f,
                    l.line,
                    format!("link target `{path}` does not exist"),
                ));
                continue;
            }
            let anchors = || match held {
                Some((st, logical)) => st
                    .source(cx.snap, logical, true)
                    .map(|s| Doc::parse(s).anchors()),
                None => cx
                    .snap
                    .file(&target)
                    .and_then(|t| t.doc.as_ref())
                    .map(Doc::anchors),
            };
            if let Some(a) = anchor
                && let Some(names) = anchors()
                && !names.iter().any(|x| x == a)
            {
                out.push(Finding::error(
                    "P004",
                    f,
                    l.line,
                    format!(
                        "link anchor `#{a}` does not exist in `{}`",
                        path_or_self(path, body_doc.is_some())
                    ),
                ));
            }
        }
    }
    for e in cx.an.records.edges() {
        if cx.an.records.get(e.to).is_none() && e.kind != crate::records::EdgeKind::Link {
            let Some(from) = cx.an.records.get(e.from) else {
                continue;
            };
            out.push(Finding::error(
                "P004",
                &cx.snap.files[from.file],
                e.line,
                format!("{} names {}, which does not exist", e.kind.label(), e.to),
            ));
        }
    }
}

fn path_or_self(p: &str, in_edit_body: bool) -> &str {
    match (p.is_empty(), in_edit_body) {
        (false, _) => p,
        (true, false) => "this file",
        (true, true) => "the document this edit changes",
    }
}
