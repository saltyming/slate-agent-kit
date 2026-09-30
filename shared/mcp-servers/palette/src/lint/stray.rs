//! P015 stray files: anything under `_palette/` or inside a placed family folder that
//! belongs to no document family.
//!
//! Walks the internal folder and the project-path family folders through the
//! `FileSource` and reports every file the snapshot did not load. The message says
//! where such content belongs, so the finding is actionable.

use std::path::{Path, PathBuf};

use super::{Cx, Finding, Severity};
use crate::layout::{Family, Placement};
use crate::util::rel_slash;

/// Files the internal folder holds besides documents.
const INTERNAL_FILES: [&str; 3] = ["layout.rst", ".gitignore", ".palette.lock"];

pub(super) fn check(cx: &Cx, out: &mut Vec<Finding>) {
    let loc = &cx.snap.loc;
    let palette = loc.palette_dir();
    if cx.src.is_dir(&palette) {
        let mut files = Vec::new();
        walk(cx, &palette, &mut files);
        for p in files {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
            let allowed = p.parent() == Some(palette.as_path())
                && name.as_deref().is_some_and(|n| INTERNAL_FILES.contains(&n));
            if !allowed && cx.snap.file(&p).is_none() {
                report(cx, &p, out);
            }
        }
    }
    for fam in [
        Family::Rfc,
        Family::Adr,
        Family::Changeset,
        Family::Design,
        Family::Spec,
        Family::Staging,
    ] {
        if *loc.placement(fam) == Placement::Internal {
            continue;
        }
        let dir = loc.dir_of(fam);
        let Ok(entries) = cx.src.list(&dir) else {
            continue;
        };
        for e in entries {
            let p = dir.join(&e.name);
            if e.is_dir {
                if fam == Family::Staging && (e.name == "design" || e.name == "spec") {
                    let Ok(inner) = cx.src.list(&p) else { continue };
                    for f in inner {
                        let q = p.join(&f.name);
                        if f.is_dir || cx.snap.file(&q).is_none() {
                            report(cx, &q, out);
                        }
                    }
                } else {
                    report(cx, &p, out);
                }
            } else if cx.snap.file(&p).is_none() {
                report(cx, &p, out);
            }
        }
    }
}

fn walk(cx: &Cx, dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = cx.src.list(dir) else {
        return;
    };
    for e in entries {
        let p = dir.join(&e.name);
        if e.is_dir {
            walk(cx, &p, out);
        } else {
            out.push(p);
        }
    }
}

fn report(cx: &Cx, path: &Path, out: &mut Vec<Finding>) {
    let rel = rel_slash(&cx.snap.root, path);
    out.push(Finding {
        rule: "P015",
        severity: Severity::Error,
        file: rel.clone(),
        line: 1,
        message: format!(
            "`{rel}` belongs to no document family; research and evidence go into the RFC or ADR that relies on them, a decision or question into state, and anything else outside the palette folders"
        ),
    });
}
