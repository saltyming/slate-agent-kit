//! P017 directive structure: a `list-table` or `code-block` in a family that admits
//! them, checked against the grammar in `directives`.
//!
//! Owns which families admit the two directives; `syntax` (P001) asks it before
//! reporting a directive. Does not check where other directives appear.

use super::{Cx, Finding};
use crate::directives::{ADMITTED, Directive, name_at, problems};
use crate::docs::Role;
use crate::rst::Kind;

/// Whether documents of `role` admit `list-table` and `code-block`: records,
/// changesets, staging and maintained documents do; work documents, the layout and
/// generated indexes do not.
pub(super) fn admits(role: Role) -> bool {
    matches!(
        role,
        Role::Rfc
            | Role::Adr
            | Role::Changeset
            | Role::Staging(_)
            | Role::Design
            | Role::Spec
            | Role::Principles
            | Role::Glossary
            | Role::Contributing
    )
}

/// Whether line `line` of a `role` document is a directive P001 leaves to P017.
pub(super) fn is_admitted(role: Role, line: &str) -> bool {
    admits(role) && name_at(line).is_some_and(|(_, name, _)| ADMITTED.contains(&name.as_str()))
}

pub(super) fn check(cx: &Cx, out: &mut Vec<Finding>) {
    for f in &cx.snap.files {
        let Some(doc) = &f.doc else { continue };
        if !admits(f.role) {
            continue;
        }
        for i in 0..doc.src.len() {
            if doc.kinds[i] != Kind::Directive {
                continue;
            }
            let Some(d) = Directive::read(&doc.src, i) else {
                continue;
            };
            for (line, message) in problems(&doc.src, &d) {
                out.push(Finding::error("P017", f, line, message));
            }
        }
    }
}
