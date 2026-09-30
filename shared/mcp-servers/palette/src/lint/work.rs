//! P009 status placement, P010 development stages, P011 budgets and P013 backlog.
//!
//! P009 to P011 read the work documents (backlog, phase, deliverable, state) line by
//! line with the patterns from `patterns`. P013 checks backlog items against phase and
//! deliverable files.

use std::collections::BTreeMap;

use super::patterns::*;
use super::{Cx, Finding};
use crate::backlog::{Backlog, in_phase};
use crate::docs::Role;
use crate::rst::Kind;
use crate::state::State;
use crate::util::{join_lexical, re};

pub(super) fn check(cx: &Cx, out: &mut Vec<Finding>) {
    for f in &cx.snap.files {
        if !f.role.is_work() {
            continue;
        }
        let Some(doc) = &f.doc else { continue };
        let no_status = matches!(f.role, Role::Phase(_) | Role::Deliverable(_) | Role::State);
        if no_status {
            for fld in doc.fields(0, doc.src.len()) {
                if fld.name.eq_ignore_ascii_case("status") {
                    out.push(Finding::error(
                        "P009",
                        f,
                        fld.start,
                        "a Status field is not allowed here; item status is recorded only in the backlog",
                    ));
                }
            }
        }
        for i in 0..doc.src.len() {
            if !matches!(doc.kinds[i], Kind::Text) {
                continue;
            }
            let text = doc.src.text(i);
            if no_status {
                if CHECKBOX.is_match(text) {
                    out.push(Finding::error(
                        "P009",
                        f,
                        i,
                        "checkboxes are not allowed; status is recorded only in the backlog",
                    ));
                } else if STATUS_MARKER.is_match(text) {
                    out.push(Finding::error("P009", f, i, "a done/pending marker is not allowed; status is recorded only in the backlog"));
                }
            }
            let mut seen: Vec<&str> = Vec::new();
            for (rx, what) in STAGE_PATTERNS.iter() {
                if rx.is_match(text) && !seen.contains(what) {
                    seen.push(what);
                    out.push(Finding::warning(
                        "P010",
                        f,
                        i,
                        format!("{what}: documents record decisions and facts, not development stages; history lives in version control"),
                    ));
                }
            }
        }
        match f.role {
            Role::State => {
                if doc.src.len() > STATE_MAX_LINES {
                    out.push(Finding::warning(
                        "P011",
                        f,
                        0,
                        format!(
                            "state has {} lines; keep it within {STATE_MAX_LINES}",
                            doc.src.len()
                        ),
                    ));
                }
                let st = State::parse(doc);
                for sec in st.sections.iter().flatten() {
                    for e in &sec.entries {
                        if e.end - e.start > STATE_ENTRY_MAX_LINES {
                            out.push(Finding::warning(
                                "P011",
                                f,
                                e.start,
                                format!("state entry has {} lines; keep it within {STATE_ENTRY_MAX_LINES}", e.end - e.start),
                            ));
                        }
                    }
                }
            }
            Role::Phase(_) | Role::Deliverable(_) => {
                if doc.src.len() > PHASE_MAX_LINES {
                    out.push(Finding::warning(
                        "P011",
                        f,
                        0,
                        format!(
                            "file has {} lines; keep it within {PHASE_MAX_LINES}",
                            doc.src.len()
                        ),
                    ));
                }
            }
            Role::Backlog => {
                let b = Backlog::parse(doc);
                for it in &b.items {
                    if let Some((s, e)) = it.body
                        && e - s > ITEM_BODY_MAX_LINES
                    {
                        out.push(Finding::warning(
                            "P011",
                            f,
                            s,
                            format!("item {} has a body of {} lines; keep it within {ITEM_BODY_MAX_LINES}", it.label(), e - s),
                        ));
                    }
                }
                backlog_rules(cx, f, &b, out);
            }
            _ => {}
        }
    }
}

fn backlog_rules(cx: &Cx, f: &crate::docs::DocFile, b: &Backlog, out: &mut Vec<Finding>) {
    let mut seen: BTreeMap<u32, usize> = BTreeMap::new();
    let link_re = re(r"`[^`<]*<([^>`]+)>`_+");
    let dir = f.path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    for it in &b.items {
        if seen.insert(it.id, it.line).is_some() {
            out.push(Finding::error(
                "P013",
                f,
                it.line,
                format!("duplicate item id {}", it.label()),
            ));
        }
        let status = it.value("Status").unwrap_or("");
        if let Some(n) = in_phase(status)
            && cx.snap.file(&cx.snap.loc.phase_file(n)).is_none()
        {
            out.push(Finding::error(
                "P013",
                f,
                it.field("Status").map(|x| x.start).unwrap_or(it.line),
                format!(
                    "item {} is in-phase-{n} but there is no phase {n} file ({})",
                    it.label(),
                    crate::util::rel_slash(&cx.snap.root, &cx.snap.loc.phase_file(n))
                ),
            ));
        }
        if let Some(fld) = it.field("Deliverable")
            && let Some(c) = link_re.captures(&fld.value)
        {
            let target = c[1].trim().split('#').next().unwrap_or("").to_string();
            let missing = match join_lexical(&dir, &target) {
                Some(p) => cx.src.read(&p).ok().flatten().is_none(),
                None => true,
            };
            if missing {
                out.push(Finding::error(
                    "P013",
                    f,
                    fld.start,
                    format!(
                        "item {} links to deliverable `{target}`, which does not exist",
                        it.label()
                    ),
                ));
            }
        }
        if status == "done" && it.value("Outcome").is_some_and(|o| o.trim() == "none") {
            out.push(Finding::warning(
                "P013",
                f,
                it.field("Outcome").map(|x| x.start).unwrap_or(it.line),
                format!("item {} is done but its Outcome is none; point to the RFC, ADR or changelog entry that records the result", it.label()),
            ));
        }
    }
}
