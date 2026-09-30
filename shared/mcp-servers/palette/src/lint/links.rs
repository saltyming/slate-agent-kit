//! P004: links whose target file or anchor is missing, and project documents that
//! link into `_palette/`. Also reports header relations that name a record that does
//! not exist.

use std::collections::BTreeSet;

use super::{Cx, Finding};
use crate::docs::Role;
use crate::rst::{is_external, split_anchor};
use crate::util::join_lexical;

pub(super) fn check(cx: &Cx, out: &mut Vec<Finding>) {
    for f in &cx.snap.files {
        if matches!(f.role, Role::Staging(_)) {
            continue;
        }
        let Some(doc) = &f.doc else { continue };
        let dir = f.path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let internal_source = cx.snap.loc.is_internal(&f.path);
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
            let target = if path.is_empty() {
                Some(f.path.clone())
            } else {
                join_lexical(&dir, path)
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
            let exists = cx.src.read(&target).ok().flatten().is_some() || cx.src.is_dir(&target);
            if !exists {
                out.push(Finding::error(
                    "P004",
                    f,
                    l.line,
                    format!("link target `{path}` does not exist"),
                ));
                continue;
            }
            if let Some(a) = anchor
                && let Some(t) = cx.snap.file(&target).and_then(|t| t.doc.as_ref())
                && !t.anchors().iter().any(|x| x == a)
            {
                out.push(Finding::error(
                    "P004",
                    f,
                    l.line,
                    format!(
                        "link anchor `#{a}` does not exist in `{}`",
                        path_or_self(path)
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

fn path_or_self(p: &str) -> &str {
    if p.is_empty() { "this file" } else { p }
}
