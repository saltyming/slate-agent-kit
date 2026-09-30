//! P005 relations and P006 changes.
//!
//! P005: links to newer records, cycles, redundant `Depends` entries, a target in both
//! `Depends` and `Related`, whole `Supersedes` targets that are not `Superseded`, and
//! `Amends` outside what the contributing document admits.
//! P006: `Changes` targets that neither exist nor are created by the record's changeset.

use std::collections::BTreeSet;

use super::patterns::DIRECT_USE_KEYWORD;
use super::{Cx, Finding};
use crate::changeset::{Changeset, EditKind, resolve_section};
use crate::layout::split_logical;
use crate::records::{AmendsCutoff, RecId, RecordKind, Records};
use crate::util::is_iso_date;

pub(super) fn check(cx: &Cx, out: &mut Vec<Finding>) {
    relations(cx, out);
    changes(cx, out);
}

fn relations(cx: &Cx, out: &mut Vec<Finding>) {
    let recs = &cx.an.records;
    for e in recs.edges() {
        let (Some(from), Some(to)) = (recs.get(e.from), recs.get(e.to)) else {
            continue;
        };
        let f = &cx.snap.files[from.file];
        if e.from == e.to {
            out.push(Finding::error(
                "P005",
                f,
                e.line,
                format!("{} links to the record itself", e.kind.label()),
            ));
        } else if e.from.kind == e.to.kind {
            if e.to.number > e.from.number {
                out.push(Finding::error(
                    "P005",
                    f,
                    e.line,
                    format!(
                        "{} {} is a record created later than {}; link only to older records",
                        e.kind.label(),
                        e.to,
                        e.from
                    ),
                ));
            }
        } else if let (Some(a), Some(b)) = (from.date.as_deref(), to.date.as_deref())
            && is_iso_date(a)
            && is_iso_date(b)
            && b > a
        {
            out.push(Finding::error(
                "P005",
                f,
                e.line,
                format!(
                    "{} {} is dated {b}, later than {} ({a}); link only to older records",
                    e.kind.label(),
                    e.to,
                    e.from
                ),
            ));
        }
    }
    for cycle in recs.cycles() {
        let first = cycle[0];
        let Some(r) = recs.get(first) else { continue };
        let line = recs
            .edges()
            .iter()
            .find(|e| e.from == first && cycle.contains(&e.to) && e.to != first)
            .map(|e| e.line)
            .unwrap_or(0);
        let shown: Vec<String> = cycle.iter().take(8).map(|c| c.to_string()).collect();
        let more = if cycle.len() > 8 {
            format!(" and {} more", cycle.len() - 8)
        } else {
            String::new()
        };
        out.push(Finding::error(
            "P005",
            &cx.snap.files[r.file],
            line,
            format!(
                "relation cycle among {}{more}; records may link only to older records",
                shown.join(", ")
            ),
        ));
    }
    for r in &recs.list {
        let f = &cx.snap.files[r.file];
        let listed: Vec<&crate::records::RelEntry> = r
            .depends
            .iter()
            .filter(|e| recs.get(e.id).is_some())
            .collect();
        for e in &listed {
            let via = listed
                .iter()
                .find(|o| o.id != e.id && recs.closure(o.id).contains(&e.id));
            let states_direct = e
                .note
                .as_deref()
                .is_some_and(|n| n.to_lowercase().contains(DIRECT_USE_KEYWORD));
            if let Some(via) = via
                && !states_direct
            {
                out.push(Finding::warning(
                    "P005",
                    f,
                    e.line,
                    format!(
                        "Depends {} is already reachable through {}; remove it or state the direct use in its parenthetical (using the word \"{DIRECT_USE_KEYWORD}\")",
                        e.id, via.id
                    ),
                ));
            }
        }
        let deps: BTreeSet<RecId> = r.depends.iter().map(|e| e.id).collect();
        for e in &r.related {
            if deps.contains(&e.id) {
                out.push(Finding::warning(
                    "P005",
                    f,
                    e.line,
                    format!(
                        "{} is in both Depends and Related; keep it in Depends",
                        e.id
                    ),
                ));
            }
        }
        // A whole supersession sets the older record's status; a partial one leaves it.
        for e in r.supersedes.iter().filter(|e| !e.partial) {
            let Some(target) = recs.get(e.id) else {
                continue;
            };
            let status = target.status.as_deref().unwrap_or("(none)");
            if status != "Superseded" {
                out.push(Finding::error(
                    "P005",
                    f,
                    e.line,
                    format!("{} is named in Supersedes but its status is {status}; its status must be Superseded (write `in part: ...` to replace only a part)", e.id),
                ));
            }
        }
        // The other direction: a Superseded record must be named as a whole by a
        // newer record.
        if r.status.as_deref() == Some("Superseded")
            && let Some(id) = Records::id_of(r)
            && recs.wholly_superseded_by(id).is_empty()
        {
            let line = r.field("Status").map(|x| x.start).unwrap_or(0);
            out.push(Finding::error(
                "P005",
                f,
                line,
                format!("{id} is Superseded but no newer record names it in Supersedes as a whole; add the Supersedes entry to the record that replaces it, or correct the status"),
            ));
        }
        amends(cx, r, out);
    }
}

/// `Amends` is admitted only up to the cutoff the contributing document states, and
/// an ADR amends only an ADR (it decides within an RFC's contract and cannot change
/// it).
fn amends(cx: &Cx, r: &crate::records::Record, out: &mut Vec<Finding>) {
    if r.amends.is_empty() {
        return;
    }
    let f = &cx.snap.files[r.file];
    let line = r.field("Amends").map(|x| x.start).unwrap_or(0);
    let is_adr = cx.snap.files[r.file].role == crate::docs::Role::Adr;
    for e in &r.amends {
        if is_adr && e.id.kind == RecordKind::Rfc {
            out.push(Finding::error(
                "P005",
                f,
                e.line,
                format!(
                    "Amends {} names an RFC from an ADR; an ADR amends only an ADR",
                    e.id
                ),
            ));
        }
    }
    match crate::records::amends_cutoff(cx.snap) {
        AmendsCutoff::Until(until) => {
            if let Some(date) = r.date.as_deref()
                && is_iso_date(date)
                && date > until.as_str()
            {
                out.push(Finding::error(
                    "P005",
                    f,
                    line,
                    format!("Amends is admitted on records dated up to {until} (the contributing document's Records section); this record is dated {date}. Write the relation as Supersedes or Related"),
                ));
            }
        }
        AmendsCutoff::None => out.push(Finding::error(
            "P005",
            f,
            line,
            "Amends is a relation of older records and the contributing document admits none (its Records section says `:Amends: none` or is absent). Write the relation as Supersedes or Related, or set `:Amends: until <date>` there",
        )),
    }
}

fn created_titles(cs: &Changeset, cs_doc: &crate::rst::Doc, doc_path: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for de in cs.docs.iter().filter(|d| d.doc == doc_path) {
        for e in &de.edits {
            if e.kind == EditKind::Create {
                out.insert(e.target.clone());
            }
            for h in &cs_doc.headings {
                if h.start > cs_doc.headings[e.heading].start
                    && h.start < e.end
                    && (h.level == 4 || h.level == 5)
                {
                    out.insert(h.title.clone());
                }
            }
        }
    }
    out
}

fn changes(cx: &Cx, out: &mut Vec<Finding>) {
    for r in &cx.an.records.list {
        if r.changes.is_empty() {
            continue;
        }
        let f = &cx.snap.files[r.file];
        // A `Changes` value that is not in the template format is already reported by
        // P002; reading it entry by entry would only repeat that finding.
        let format_ok = r.field("Changes").is_none_or(|fld| {
            let kind = if f.role == crate::docs::Role::Rfc {
                "rfc"
            } else {
                "adr"
            };
            crate::schema::schemas()
                .get(kind)
                .and_then(|s| {
                    s.header
                        .iter()
                        .find(|h| h.literal_name() == Some("Changes"))
                })
                .is_none_or(|spec| spec.value.check(&fld.value, true).is_ok())
        });
        if !format_ok {
            continue;
        }
        let own = Records::id_of(r).and_then(|id| cx.an.sets.get(&id));
        for c in &r.changes {
            if split_logical(&c.doc).is_none() {
                out.push(Finding::error(
                    "P006",
                    f,
                    c.line,
                    format!("`{}` is not a maintained document path (`design/<topic>.rst` or `spec/<topic>.rst`)", c.doc),
                ));
                continue;
            }
            let maintained = cx
                .snap
                .loc
                .maintained_file(&c.doc)
                .and_then(|p| cx.snap.file(&p))
                .and_then(|d| d.doc.as_ref());
            let creates = own.is_some_and(|cs| {
                cs.docs
                    .iter()
                    .any(|d| d.doc == c.doc && d.edits.iter().any(|e| e.kind == EditKind::Create))
            });
            if maintained.is_none() && !creates {
                out.push(Finding::error(
                    "P006",
                    f,
                    c.line,
                    format!(
                        "`{}` does not exist and this record's changeset does not create it",
                        c.doc
                    ),
                ));
                continue;
            }
            let created = match own {
                Some(cs) => cx.snap.files[cs.file]
                    .doc
                    .as_ref()
                    .map(|d| created_titles(cs, d, &c.doc))
                    .unwrap_or_default(),
                None => BTreeSet::new(),
            };
            for s in &c.sections {
                if s.eq_ignore_ascii_case("created") {
                    continue;
                }
                let exists = maintained.is_some_and(|d| match resolve_section(d, s) {
                    Ok(_) => true,
                    Err(msg) => msg.contains("not unique"),
                });
                let last = s.rsplit(" / ").next().unwrap_or(s).trim();
                if !exists && !created.contains(last) {
                    out.push(Finding::error(
                        "P006",
                        f,
                        c.line,
                        format!("section `{s}` of `{}` neither exists nor is created by this record's changeset", c.doc),
                    ));
                }
            }
        }
    }
}
