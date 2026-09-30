//! P005 relations and P006 changes.
//!
//! P005: links to newer records, cycles, redundant `Depends` entries, a target in both
//! `Depends` and `Related`, whole `Supersedes` targets that are not `Superseded`, and
//! `Amends` outside what the contributing document admits.
//! P006: `Changes` targets that neither exist, nor are created by the record's changeset,
//! nor exist once the record's dependency closure's changesets are applied; principles,
//! glossary and contributing entries whose document or sections are missing; and other
//! project documents that are missing, not `.md` or `.rst`, outside the project, internal,
//! or a palette document under its real path. An entry without a parenthetical fails
//! the template format, P002, and P006 does not repeat it.

use std::collections::BTreeSet;

use super::patterns::DIRECT_USE_KEYWORD;
use super::{Cx, Finding};
use crate::changeset::{Changeset, EditKind, resolve_section};
use crate::docs::Role;
use crate::layout::{CHANGES_FORMS, ChangesDoc, Family, changes_doc};
use crate::records::DocRef;
use crate::records::{AmendsCutoff, RecId, RecordKind, Records};
use crate::rst::Doc;
use crate::util::is_iso_date;
use crate::util::{same_path_nocase, starts_with_nocase};

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
    let is_adr = cx.snap.files[r.file].role == Role::Adr;
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
        let id = Records::id_of(r);
        let own = id.and_then(|id| cx.an.sets.get(&id));
        let state = id.and_then(|id| cx.an.states.get(&id));
        for c in &r.changes {
            let problem = match changes_doc(&c.doc) {
                Ok(ChangesDoc::Maintained(_)) => None,
                Ok(ChangesDoc::Single(fam)) => Some(single_document(cx, fam, c)),
                Ok(ChangesDoc::Project(parts)) => Some(project_document(cx, &parts, c)),
                Err(msg) => Some(Err(msg)),
            };
            // Principles, glossary, contributing and other project documents are
            // checked on disk alone: no changeset edits them.
            if let Some(result) = problem {
                if let Err(msg) = result {
                    out.push(Finding::error("P006", f, c.line, msg));
                }
                continue;
            }
            let maintained = cx
                .snap
                .loc
                .maintained_file(&c.doc)
                .and_then(|p| cx.snap.file(&p))
                .and_then(|d| d.doc.as_ref());
            // The document as the record's dependency closure leaves it.
            let from_deps = state
                .and_then(|st| st.source(cx.snap, &c.doc, false))
                .map(Doc::parse);
            let creates = own.is_some_and(|cs| {
                cs.docs
                    .iter()
                    .any(|d| d.doc == c.doc && d.edits.iter().any(|e| e.kind == EditKind::Create))
            });
            let marked_created = c.sections.iter().any(|s| s.eq_ignore_ascii_case("created"));
            if maintained.is_none() && !creates {
                if from_deps.is_none() {
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
                if marked_created {
                    out.push(Finding::error(
                        "P006",
                        f,
                        c.line,
                        format!(
                            "`{}` is marked (created) but a dependency's changeset creates it, not this record's; name the sections this record changes instead",
                            c.doc
                        ),
                    ));
                }
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
                let found = |d: &Doc| match resolve_section(d, s) {
                    Ok(_) => true,
                    Err(msg) => msg.contains("not unique"),
                };
                let exists = maintained.is_some_and(found) || from_deps.as_ref().is_some_and(found);
                let last = s.rsplit(" / ").next().unwrap_or(s).trim();
                if !exists && !created.contains(last) {
                    out.push(Finding::error(
                        "P006",
                        f,
                        c.line,
                        format!("section `{s}` of `{}` neither exists nor is created by this record's changeset or its dependencies' changesets", c.doc),
                    ));
                }
            }
        }
    }
}

/// Why `(created)` is refused outside design and spec documents.
const NOT_CREATED: &str = "changesets and staging hold only design and spec documents, so a record cannot create it with `(created)`";

/// A `Changes` entry naming the principles, glossary or contributing document: the
/// document exists where the layout places it and has the sections named.
fn single_document(cx: &Cx, fam: Family, c: &DocRef) -> Result<(), String> {
    if c.sections.iter().any(|s| s.eq_ignore_ascii_case("created")) {
        return Err(format!(
            "`{}` cannot be marked (created): {NOT_CREATED}; name the sections that change",
            c.doc
        ));
    }
    let path = cx.snap.loc.file_of(fam);
    let Some(doc) = cx.snap.file(&path).and_then(|d| d.doc.as_ref()) else {
        return Err(format!(
            "`{}` does not exist (the layout places it at `{}`)",
            c.doc,
            crate::util::rel_slash(&cx.snap.root, &path)
        ));
    };
    let missing: Vec<&str> = c
        .sections
        .iter()
        .filter(|s| match resolve_section(doc, s) {
            Ok(_) => false,
            Err(msg) => !msg.contains("not unique"),
        })
        .map(String::as_str)
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "section `{}` of `{}` does not exist; a changeset cannot add it ({NOT_CREATED}), so name sections the document has",
            missing.join("`, `"),
            c.doc
        ))
    }
}

/// A `Changes` entry naming another project document: an existing `.md` or `.rst` file
/// inside the project and outside `_palette/`, not a palette document, with a
/// parenthetical saying what changes.
fn project_document(cx: &Cx, parts: &[String], c: &DocRef) -> Result<(), String> {
    let doc = &c.doc;
    if c.note.eq_ignore_ascii_case("created") {
        return Err(format!(
            "`{doc}` cannot be marked (created): {NOT_CREATED}; say what changes in the document"
        ));
    }
    let loc = &cx.snap.loc;
    let path = parts.iter().fold(cx.snap.root.clone(), |a, p| a.join(p));
    if starts_with_nocase(&path, &loc.palette_dir()) {
        return Err(format!(
            "`{doc}` is inside `_palette/`, which holds internal work documents that a record does not change"
        ));
    }
    for fam in [Family::Design, Family::Spec] {
        let dir = loc.dir_of(fam);
        if starts_with_nocase(&path, &dir)
            && path.components().count() == dir.components().count() + 1
        {
            return Err(format!(
                "`{doc}` is a {} document; write its logical name `{}/{}`",
                fam.key(),
                fam.key(),
                parts.last().map(String::as_str).unwrap_or_default()
            ));
        }
    }
    for fam in [Family::Principles, Family::Glossary, Family::Contributing] {
        if same_path_nocase(&path, &loc.file_of(fam)) {
            return Err(format!(
                "`{doc}` is the {} document; write its logical name `{}.rst`",
                fam.key(),
                fam.key()
            ));
        }
    }
    // Every palette document the snapshot classified: records, changesets, staging,
    // indexes and the work documents wherever the layout places them.
    let role = cx
        .snap
        .files
        .iter()
        .find(|f| same_path_nocase(&f.path, &path))
        .map(|f| f.role);
    match role {
        Some(Role::Rfc | Role::Adr) => {
            return Err(format!(
                "`{doc}` is a record; relations between records go in Depends, Supersedes or Related"
            ));
        }
        Some(_) => {
            return Err(format!(
                "`{doc}` is a palette work, generated or index document, which a record does not change. Accepted forms: {CHANGES_FORMS}"
            ));
        }
        None => {}
    }
    if cx.src.read(&path).ok().flatten().is_none() {
        return Err(format!("`{doc}` does not exist in the project"));
    }
    Ok(())
}
