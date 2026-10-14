//! `palette reflow`: rewrapping the prose of existing documents.
//!
//! Owns choosing the documents a reflow covers (every document the layout places that
//! the server does not generate, or those under the given paths) and running the
//! rewrap as one write transaction: per-project lock, regenerated indexes and staging,
//! lint of the files written, and only changed files written. The wrapping itself is
//! in `reflow`. Not an MCP tool: the write tools wrap only the fragments they receive,
//! and this command is the one operation that rewraps a document already on disk.

use super::{Ctx, WriteResult, run_transaction};
use crate::docs::Role;
use crate::errors::{PalError, Res};
use crate::reflow::reflow_source;

/// What `palette reflow` says when every selected document is already wrapped.
pub const NOTHING_TO_REFLOW: &str = "nothing to reflow: every document is already wrapped";

/// The components of a project-relative `path` given on the command line.
fn path_components(path: &str) -> Res<Vec<String>> {
    let normalized = path.trim().replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').filter(|c| !c.is_empty()).collect();
    let bytes = normalized.as_bytes();
    let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    let absolute = normalized.starts_with('/') || drive;
    if parts.is_empty() || absolute || parts.iter().any(|c| *c == ".." || *c == ".") {
        return Err(PalError::invalid(format!(
            "path `{path}` must be a project-relative file or folder without `.` or `..`"
        )));
    }
    Ok(parts.into_iter().map(str::to_string).collect())
}

/// Whether the server generates or owns a document of `role`, so that a reflow leaves
/// it to regeneration: the layout, the indexes and the staging documents.
fn generated(role: Role) -> bool {
    matches!(role, Role::Layout | Role::Index(_) | Role::Staging(_))
}

/// `palette reflow <project> [paths]`: rewraps the prose of every document the layout
/// places and the server does not generate, or of those equal to or under one of
/// `paths` (project-relative files or folders). The transaction regenerates the indexes
/// and staging from the rewrapped text and refuses a result that does not lint clean.
/// Like `palette generate`, it works on a checkout without `_palette/`, where the
/// families are the ones the documents themselves identify.
pub fn reflow(ctx: &Ctx, project: &str, paths: &[String]) -> Res<WriteResult> {
    let wanted: Vec<(String, Vec<String>)> = paths
        .iter()
        .map(|p| path_components(p).map(|c| (p.clone(), c)))
        .collect::<Res<_>>()?;
    let mut result = run_transaction(ctx, project, false, false, |s| {
        let mut hit = vec![false; wanted.len()];
        let mut changed = Vec::new();
        for f in s.snap.files.iter().filter(|f| !generated(f.role)) {
            let parts: Vec<&str> = f.rel.split('/').collect();
            let mut selected = wanted.is_empty();
            for (i, (_, w)) in wanted.iter().enumerate() {
                if parts.len() >= w.len() && parts.iter().zip(w).all(|(a, b)| *a == b) {
                    hit[i] = true;
                    selected = true;
                }
            }
            if !selected {
                continue;
            }
            if let Some(line) = f.bad_utf8 {
                return Err(PalError::parse(&f.rel, line, "the file is not valid UTF-8"));
            }
            let Some(doc) = &f.doc else { continue };
            match reflow_source(&doc.src) {
                Ok(out) if out != doc.src => changed.push((f.path.clone(), out)),
                Ok(_) => {}
                Err(why) => return Err(PalError::invariant(format!("{}: {why}", f.rel))),
            }
        }
        if let Some(i) = hit.iter().position(|h| !h) {
            return Err(PalError::not_found(format!(
                "`{}` names no document the layout places; indexes, staging documents and layout.rst are generated and are not rewrapped",
                wanted[i].0
            )));
        }
        for (path, out) in changed {
            s.put_raw(&path, &out)?;
        }
        Ok(Vec::new())
    })?;
    if result.files.is_empty() {
        result.message = Some(NOTHING_TO_REFLOW.to_string());
    }
    Ok(result)
}
