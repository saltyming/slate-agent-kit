//! Record and changeset tools.
//!
//! `palette_record_create`, `palette_record_update`, `palette_changeset_edit` and
//! `palette_changeset_promote`. Records are written from the RFC and ADR templates; the
//! time-varying header fields (`Implementation`, `Verification`, `Implementers`,
//! `Revised`) are written only here. Changeset edits are validated by the lint that
//! runs before the commit.

use std::collections::BTreeMap;
use std::path::PathBuf;

use super::{Ctx, Session, WriteResult, run_write};
use crate::changeset::{self, Changeset, Edit, EditKind};
use crate::edit;
use crate::errors::{PalError, Res};
use crate::layout::split_logical;
use crate::params::*;
use crate::records::{PARTIAL_MARKER, RecId, RecordKind};
use crate::rst::Doc;
use crate::schema::{schemas, split_entries};
use crate::text::{Eol, Source};
use crate::util::{is_kebab, slugify};

const DEFAULT_SCOPE: &str = "the change this record describes";

fn record_path(s: &Session<'_, '_>, id: RecId) -> PathBuf {
    s.snap
        .loc
        .dir_of(id.kind.family())
        .join(format!("{}-{:04}", id.kind.lower(), id.number))
}

/// The note and partial flag of a relation input: `partial: true`, or a note that
/// already starts with the `in part:` marker (which is stripped so it is never
/// written twice), makes the entry partial.
fn relation_parts(r: &RelationIn) -> (String, bool) {
    let note = edit::one_line(&r.note).replace(';', ",");
    let lower = note.to_lowercase();
    if let Some(rest) = lower.strip_prefix(PARTIAL_MARKER) {
        let cut = note.len() - rest.len();
        (note[cut..].trim().to_string(), true)
    } else {
        (note, r.partial == Some(true))
    }
}

fn rel_value(list: &[RelationIn]) -> Res<String> {
    if list.is_empty() {
        return Ok("none".to_string());
    }
    let mut parts = Vec::new();
    for r in list {
        let id = RecId::parse(&r.record).ok_or_else(|| {
            PalError::invalid(format!(
                "`{}` is not a record identifier (`RFC-0001`)",
                r.record
            ))
        })?;
        let (note, partial) = relation_parts(r);
        if note.is_empty() {
            return Err(PalError::invalid(format!(
                "{id}: the parenthetical (what is used, replaced or gathered) is required"
            )));
        }
        if partial {
            parts.push(format!("{id} ({PARTIAL_MARKER} {note})"));
        } else {
            parts.push(format!("{id} ({note})"));
        }
    }
    Ok(parts.join("; "))
}

fn changes_value(list: &[ChangeIn]) -> Res<String> {
    if list.is_empty() {
        return Ok("none".to_string());
    }
    let mut parts = Vec::new();
    for c in list {
        let secs: Vec<String> = c
            .sections
            .clone()
            .unwrap_or_default()
            .iter()
            .map(|x| edit::one_line(x).replace(';', ","))
            .filter(|x| !x.is_empty())
            .collect();
        if secs.is_empty() {
            return Err(PalError::invalid(format!(
                "{}: name at least one section, or `created` for a new document",
                c.document
            )));
        }
        parts.push(format!("{} ({})", c.document.trim(), secs.join("; ")));
    }
    Ok(parts.join("; "))
}

fn require(name: &str, v: &str) -> Res<String> {
    let t = edit::one_line(v);
    if t.is_empty() {
        Err(PalError::invalid(format!("{name} is required")))
    } else {
        Ok(t)
    }
}

fn mark_superseded(s: &mut Session<'_, '_>, older: RecId) -> Res<()> {
    let rec =
        s.an.records
            .get(older)
            .ok_or_else(|| s.missing_record(older))?;
    match rec.status.as_deref() {
        Some("Superseded") => return Ok(()),
        Some("Accepted") => {}
        other => {
            return Err(PalError::invariant(format!(
                "{older} is {}; only an Accepted record can be superseded",
                other.unwrap_or("(no status)")
            )));
        }
    }
    let path = s.snap.files[rec.file].path.clone();
    let mut src = s.source(&path)?;
    edit::set_header_field(&mut src, "Status", "Superseded")
        .map_err(|m| PalError::parse(&s.rel(&path), 1, m))?;
    s.put(&path, &src)
}

/// `palette_record_create`.
pub fn record_create(ctx: &Ctx, p: RecordCreateParams) -> Res<WriteResult> {
    run_write(ctx, &p.project, p.dry_run.unwrap_or(false), |s| {
        let kind = RecordKind::parse(p.kind.trim())
            .ok_or_else(|| PalError::invalid("kind must be `rfc` or `adr`"))?;
        let schema = schemas()
            .get(kind.lower())
            .ok_or_else(|| PalError::invariant("the record template is missing"))?;
        let title = require("title", &p.title)?;
        let authors = require("authors", &p.authors)?;
        let description = require("description", &p.description)?;
        let number = s.next_record_number(kind);
        let id = RecId { kind, number };
        let slug = match &p.slug {
            Some(x) if !x.trim().is_empty() => x.trim().to_string(),
            _ => slugify(&title),
        };
        if !is_kebab(&slug) {
            return Err(PalError::invalid(
                "slug must be lowercase ASCII kebab-case; give one, or use a title with ASCII letters",
            ));
        }
        let today = s.today.clone();
        let mut values: BTreeMap<&str, String> = BTreeMap::new();
        values.insert("Status", "Draft".to_string());
        values.insert(
            "Implementation",
            format!(
                "not-started — {}",
                p.implementation_scope
                    .as_deref()
                    .map(edit::one_line)
                    .filter(|x| !x.is_empty())
                    .unwrap_or_else(|| DEFAULT_SCOPE.to_string())
            ),
        );
        values.insert("Verification", format!("none — {today}; not verified yet"));
        match kind {
            RecordKind::Rfc => {
                values.insert("Areas", require("areas", p.areas.as_deref().unwrap_or(""))?)
            }
            RecordKind::Adr => values.insert(
                "Within",
                require("within", p.within.as_deref().unwrap_or(""))?,
            ),
        };
        values.insert("Authors", authors);
        values.insert(
            "Reviewers",
            p.reviewers
                .as_deref()
                .map(edit::one_line)
                .filter(|x| !x.is_empty())
                .unwrap_or_else(|| "none yet".to_string()),
        );
        values.insert("Implementers", "none yet".to_string());
        values.insert("Accepted", "none".to_string());
        values.insert("Date", today.clone());
        values.insert("Revised", "none".to_string());
        values.insert(
            "Depends",
            rel_value(&p.depends.clone().unwrap_or_default())?,
        );
        let supersedes = p.supersedes.clone().unwrap_or_default();
        values.insert("Supersedes", rel_value(&supersedes)?);
        values.insert(
            "Related",
            rel_value(&p.related.clone().unwrap_or_default())?,
        );
        values.insert(
            "Changes",
            changes_value(&p.changes.clone().unwrap_or_default())?,
        );
        values.insert("Description", description);

        let mut lines = edit::heading_lines(
            1,
            &schema
                .title
                .raw
                .replace("<NNNN>", &format!("{number:04}"))
                .replace("<Title>", &title),
        );
        lines.push(String::new());
        for spec in &schema.header {
            let name = spec.literal_name().unwrap_or("");
            let v = values.get(name).ok_or_else(|| {
                PalError::invariant(format!(
                    "the record template has a field `{name}` this server does not know"
                ))
            })?;
            lines.extend(edit::field_lines(name, v));
        }
        let mut given = p.sections.clone().unwrap_or_default();
        for spec in &schema.sections {
            let t = spec.title.literal().unwrap_or("").to_string();
            lines.push(String::new());
            lines.extend(edit::heading_lines(2, &t));
            lines.push(String::new());
            match given.remove(&t) {
                Some(text) if !text.trim().is_empty() => lines.extend(edit::verbatim_lines(&text)),
                _ => lines.push("None.".to_string()),
            }
        }
        if let Some(unknown) = given.keys().next() {
            return Err(PalError::invalid(format!(
                "`{unknown}` is not a section of the {} template; sections are: {}",
                kind.upper(),
                schema
                    .sections
                    .iter()
                    .filter_map(|x| x.title.literal())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        for r in supersedes.iter().filter(|r| !relation_parts(r).1) {
            let older = RecId::parse(&r.record).ok_or_else(|| {
                PalError::invalid(format!("`{}` is not a record identifier", r.record))
            })?;
            mark_superseded(s, older)?;
        }
        let path =
            record_path(s, id).with_file_name(format!("{}-{:04}-{slug}.rst", kind.lower(), number));
        let src = Source::from_lines(&lines, Eol::Lf);
        s.put(&path, &src)?;
        Ok(vec![id.to_string()])
    })
}

fn find_record<'a>(s: &'a Session<'_, '_>, raw: &str) -> Res<(RecId, &'a crate::records::Record)> {
    let id = RecId::parse(raw).ok_or_else(|| {
        PalError::invalid(format!("`{raw}` is not a record identifier (`RFC-0001`)"))
    })?;
    let rec = s.an.records.get(id).ok_or_else(|| s.missing_record(id))?;
    Ok((id, rec))
}

fn check_status_move(from: &str, to: &str) -> Res<()> {
    let ok = matches!(
        (from, to),
        ("Draft", "Proposed")
            | ("Proposed", "Accepted")
            | ("Draft" | "Proposed" | "Accepted", "Rejected" | "Withdrawn")
    );
    if to == "Superseded" {
        return Err(PalError::invariant(
            "Superseded is set only through a newer record's Supersedes",
        ));
    }
    if ok {
        Ok(())
    } else {
        Err(PalError::invariant(format!(
            "a record cannot move from {from} to {to}; status moves Draft → Proposed → Accepted, or to Rejected or Withdrawn"
        )))
    }
}

/// `palette_record_update`.
pub fn record_update(ctx: &Ctx, p: RecordUpdateParams) -> Res<WriteResult> {
    run_write(ctx, &p.project, p.dry_run.unwrap_or(false), |s| {
        let (id, rec) = find_record(s, &p.record)?;
        let path = s.snap.files[rec.file].path.clone();
        let rel = s.rel(&path);
        let status = rec.status.clone().unwrap_or_default();
        let old_supersedes: Vec<RecId> = rec.supersedes.iter().map(|e| e.id).collect();
        let old_whole: Vec<RecId> = rec
            .supersedes
            .iter()
            .filter(|e| !e.partial)
            .map(|e| e.id)
            .collect();
        let old_revised = rec
            .field("Revised")
            .map(|f| f.value.clone())
            .unwrap_or_else(|| "none".to_string());
        let content_change = p.title.is_some()
            || p.description.is_some()
            || p.authors.is_some()
            || p.reviewers.is_some()
            || p.areas.is_some()
            || p.within.is_some()
            || p.depends.is_some()
            || p.supersedes.is_some()
            || p.related.is_some()
            || p.changes.is_some()
            || p.sections.is_some();
        let free = matches!(status.as_str(), "Draft" | "Proposed");
        let clarification = p.clarification == Some(true);
        if clarification && free {
            return Err(PalError::invalid(
                "clarification applies to records that are no longer Draft or Proposed; change a draft freely",
            ));
        }
        let note = p
            .revision_note
            .as_deref()
            .map(edit::one_line)
            .unwrap_or_default();
        if content_change && !free {
            if !clarification {
                return Err(PalError::invariant(format!(
                    "{id} is {status}; its body changes only with clarification: true (a mechanical correction) and a revision_note. A change of substance needs a new record that supersedes it."
                )));
            }
            if note.is_empty() {
                return Err(PalError::invalid(
                    "a clarification needs revision_note: the Revised entry, one line",
                ));
            }
        }
        if clarification && !content_change {
            return Err(PalError::invalid(
                "clarification: true needs a content change to describe",
            ));
        }
        let mut src = s.source(&path)?;
        let set = |src: &mut Source, name: &str, v: &str| -> Res<()> {
            edit::set_header_field(src, name, v)
                .map(|_| ())
                .map_err(|m| PalError::parse(&rel, 1, m))
        };
        if let Some(t) = &p.title {
            let title = require("title", t)?;
            let doc = Doc::parse(src.clone());
            let idx = doc
                .headings
                .iter()
                .position(|h| h.level == 1)
                .ok_or_else(|| PalError::parse(&rel, 1, "the record has no title"))?;
            edit::retitle(&mut src, &doc, idx, &format!("{}: {title}", id));
        }
        for (name, v) in [
            ("Description", p.description.as_deref()),
            ("Authors", p.authors.as_deref()),
            ("Reviewers", p.reviewers.as_deref()),
            ("Areas", p.areas.as_deref()),
            ("Within", p.within.as_deref()),
        ] {
            if let Some(v) = v {
                set(&mut src, name, &require(name, v)?)?;
            }
        }
        if let Some(d) = &p.depends {
            set(&mut src, "Depends", &rel_value(d)?)?;
        }
        if let Some(d) = &p.related {
            set(&mut src, "Related", &rel_value(d)?)?;
        }
        if let Some(c) = &p.changes {
            set(&mut src, "Changes", &changes_value(c)?)?;
        }
        if let Some(sup) = &p.supersedes {
            let new_ids: Vec<RecId> = sup
                .iter()
                .map(|r| {
                    RecId::parse(&r.record).ok_or_else(|| {
                        PalError::invalid(format!("`{}` is not a record identifier", r.record))
                    })
                })
                .collect::<Res<_>>()?;
            let new_whole: Vec<RecId> = sup
                .iter()
                .zip(&new_ids)
                .filter(|(r, _)| !relation_parts(r).1)
                .map(|(_, n)| *n)
                .collect();
            if let Some(gone) = old_supersedes.iter().find(|o| !new_ids.contains(o)) {
                return Err(PalError::invariant(format!(
                    "{gone} is already superseded by {id}; that cannot be undone through this tool"
                )));
            }
            if let Some(gone) = old_whole.iter().find(|o| !new_whole.contains(o)) {
                return Err(PalError::invariant(format!(
                    "{gone} is superseded as a whole by {id}; that cannot become a partial supersession through this tool"
                )));
            }
            set(&mut src, "Supersedes", &rel_value(sup)?)?;
            for n in new_whole.iter().filter(|n| !old_whole.contains(n)) {
                mark_superseded(s, *n)?;
            }
        }
        if let Some(secs) = &p.sections {
            for (t, text) in secs {
                let doc = Doc::parse(src.clone());
                let idx = doc
                    .headings_at(2)
                    .find(|(_, h)| h.title == *t)
                    .map(|(i, _)| i)
                    .ok_or_else(|| PalError::invalid(format!("`{t}` is not a section of {id}")))?;
                let body = if text.trim().is_empty() {
                    vec!["None.".to_string()]
                } else {
                    edit::verbatim_lines(text)
                };
                edit::replace_section_body(&mut src, &doc, idx, &body);
            }
        }
        if let Some(new) = p.status.as_deref().map(str::trim) {
            check_status_move(&status, new)?;
            set(&mut src, "Status", new)?;
            if new == "Accepted" {
                let who = p
                    .accepted_by
                    .as_deref()
                    .map(edit::one_line)
                    .filter(|x| !x.is_empty())
                    .ok_or_else(|| PalError::invalid("accepted_by is required to move a record to Accepted: the person who accepted it"))?;
                set(&mut src, "Accepted", &format!("{who} ({})", s.now))?;
            }
        }
        if clarification {
            let entry = format!("{} — {}", s.today, note.replace(';', ","));
            let value = if old_revised.trim() == "none" {
                entry
            } else {
                format!("{old_revised}; {entry}")
            };
            set(&mut src, "Revised", &value)?;
        }
        s.put(&path, &src)?;
        Ok(Vec::new())
    })
}

fn parse_kind(k: &str) -> Res<EditKind> {
    match k.trim() {
        "replace" => Ok(EditKind::Replace),
        "insert_after" => Ok(EditKind::InsertAfter),
        "insert_into" => Ok(EditKind::InsertInto),
        "delete" => Ok(EditKind::Delete),
        "create" => Ok(EditKind::Create),
        other => Err(PalError::invalid(format!(
            "kind must be replace, insert_after, insert_into, delete or create, got `{other}`"
        ))),
    }
}

fn changeset_path(s: &Session<'_, '_>, id: RecId) -> PathBuf {
    s.snap
        .loc
        .dir_of(crate::layout::Family::Changeset)
        .join(format!("{}-{:04}.rst", id.kind.lower(), id.number))
}

fn insert_block(src: &mut Source, at: usize, block: &[String]) {
    let n = src.len();
    let mut new = Vec::new();
    if at > 0 && !src.text(at - 1).trim().is_empty() {
        new.push(String::new());
    }
    new.extend(block.iter().cloned());
    if at < n {
        new.push(String::new());
    }
    src.splice(at..at, &new);
}

fn trim_trailing_blank(src: &mut Source) {
    while src.lines.last().is_some_and(|l| l.text.trim().is_empty()) {
        src.lines.pop();
    }
}

/// `palette_changeset_edit`.
pub fn changeset_edit(ctx: &Ctx, p: ChangesetEditParams) -> Res<WriteResult> {
    run_write(ctx, &p.project, p.dry_run.unwrap_or(false), |s| {
        let (id, rec) = find_record(s, &p.record)?;
        let status = rec.status.clone().unwrap_or_default();
        if !changeset::is_checked_status(&status) {
            return Err(PalError::invariant(format!(
                "{id} is {status}; only Draft, Proposed and Accepted records have changesets"
            )));
        }
        let kind = parse_kind(&p.kind)?;
        let doc_path = p.document.trim().to_string();
        match split_logical(&doc_path) {
            Some((_, name)) if crate::util::is_kebab_rst(name) => {}
            _ => {
                return Err(PalError::invalid(
                    "document must be `design/<topic>.rst` or `spec/<topic>.rst` (lowercase kebab-case)",
                ));
            }
        }
        let target = require("target", &p.target)?;
        let body = p.body.clone().unwrap_or_default();
        let action = p.action.trim();
        if !matches!(action, "add" | "replace" | "remove") {
            return Err(PalError::invalid("action must be add, replace or remove"));
        }
        if action != "remove" && kind != EditKind::Delete && body.trim().is_empty() {
            return Err(PalError::invalid(
                "body is required: the new section title underlined with `^`, then its text",
            ));
        }
        let path = changeset_path(s, id);
        let existing = s.snap.files.iter().position(|f| f.path == path);
        let block = changeset::edit_lines(
            kind,
            &target,
            if kind == EditKind::Delete { "" } else { &body },
        );
        let title = format!("Changeset: {id}");
        let Some(fi) = existing else {
            if action != "add" {
                return Err(PalError::not_found(format!(
                    "{id} has no changeset yet; use action add"
                )));
            }
            let mut lines = edit::heading_lines(1, &title);
            lines.push(String::new());
            lines.extend(edit::heading_lines(2, &doc_path));
            lines.push(String::new());
            lines.extend(block);
            s.put(&path, &Source::from_lines(&lines, Eol::Lf))?;
            return Ok(Vec::new());
        };
        let cs = Changeset::parse(&s.snap, fi).ok_or_else(|| {
            PalError::parse(&s.snap.files[fi].rel, 1, "the changeset is not valid UTF-8")
        })?;
        let rel = s.snap.files[fi].rel.clone();
        if let Some((line, msg)) = cs.problems.first() {
            return Err(PalError::parse(&rel, line + 1, msg.clone()));
        }
        let cs_doc = s.snap.files[fi]
            .doc
            .as_ref()
            .ok_or_else(|| PalError::parse(&rel, 1, "not readable"))?;
        let mut src = cs_doc.src.clone();
        let de = cs.docs.iter().find(|d| d.doc == doc_path);
        let found: Option<(&Edit, usize)> = de.and_then(|d| {
            d.edits
                .iter()
                .position(|e| e.kind == kind && e.target == target)
                .map(|i| (&d.edits[i], i))
        });
        match action {
            "add" => {
                if found.is_some() {
                    return Err(PalError::invalid(format!(
                        "the changeset already has `{}: {target}` for {doc_path}; use action replace",
                        kind.verb()
                    )));
                }
                match de {
                    Some(d) => insert_block(&mut src, cs_doc.headings[d.heading].end, &block),
                    None => {
                        let mut full = edit::heading_lines(2, &doc_path);
                        full.push(String::new());
                        full.extend(block);
                        let n = src.len();
                        insert_block(&mut src, n, &full);
                    }
                }
            }
            "replace" => {
                let (e, _) = found.ok_or_else(|| {
                    PalError::not_found(format!(
                        "the changeset has no `{}: {target}` for {doc_path}",
                        kind.verb()
                    ))
                })?;
                let (a, b) = changeset::edit_range(cs_doc, e);
                let mut new = block;
                if b < src.len() {
                    new.push(String::new());
                }
                src.splice(a..b, &new);
            }
            _ => {
                let (e, _) = found.ok_or_else(|| {
                    PalError::not_found(format!(
                        "the changeset has no `{}: {target}` for {doc_path}",
                        kind.verb()
                    ))
                })?;
                let (a, b) = changeset::edit_range(cs_doc, e);
                let only_edit_of_doc = de.is_some_and(|d| d.edits.len() == 1);
                if only_edit_of_doc {
                    let h = &cs_doc.headings[de.map(|d| d.heading).unwrap_or(0)];
                    src.splice(h.start..h.end, &[]);
                } else {
                    src.splice(a..b, &[]);
                }
                trim_trailing_blank(&mut src);
                if cs.docs.len() == 1 && only_edit_of_doc {
                    s.remove(&path)?;
                    return Ok(Vec::new());
                }
            }
        }
        s.put(&path, &src)?;
        Ok(Vec::new())
    })
}

fn ensure_date(src: &mut Source, today: &str) {
    let doc = Doc::parse(src.clone());
    let fields = doc.header_fields();
    if fields.iter().any(|f| f.name == "Date") {
        return;
    }
    if let Some(last) = fields.last() {
        src.splice(last.end..last.end, &[format!(":Date: {today}")]);
    }
}

/// `palette_changeset_promote`.
pub fn changeset_promote(ctx: &Ctx, p: ChangesetPromoteParams) -> Res<WriteResult> {
    run_write(ctx, &p.project, p.dry_run.unwrap_or(false), |s| {
        let (id, rec) = find_record(s, &p.record)?;
        let status = rec.status.clone().unwrap_or_default();
        let rec_file = rec.file;
        let impl_value = rec.field("Implementation").map(|f| f.value.clone());
        if status != "Accepted" {
            return Err(PalError::invariant(format!(
                "{id} is {status}; only an Accepted record's edits can be promoted"
            )));
        }
        let cs =
            s.an.sets
                .get(&id)
                .ok_or_else(|| PalError::not_found(format!("{id} has no changeset")))?
                .clone();
        let impl_kind = p.implementation.trim().to_string();
        if !matches!(impl_kind.as_str(), "partial" | "complete") {
            return Err(PalError::invalid(
                "implementation must be partial or complete",
            ));
        }
        let implementers = require("implementers", &p.implementers)?;
        let ver_note = require("verification_note", &p.verification_note)?;
        let cs_path = s.snap.files[cs.file].path.clone();
        let cs_rel = s.snap.files[cs.file].rel.clone();
        let cs_doc = s.snap.files[cs.file]
            .doc
            .as_ref()
            .ok_or_else(|| PalError::parse(&cs_rel, 1, "not readable"))?
            .clone();
        // Select the edits.
        let mut selected: Vec<(String, Edit)> = Vec::new();
        match &p.edits {
            None => {
                for de in &cs.docs {
                    for e in &de.edits {
                        selected.push((de.doc.clone(), e.clone()));
                    }
                }
            }
            Some(list) => {
                for r in list {
                    let kind = parse_kind(&r.kind)?;
                    let doc = r.document.trim();
                    let hit = cs
                        .docs
                        .iter()
                        .filter(|d| d.doc == doc)
                        .flat_map(|d| d.edits.iter())
                        .find(|e| e.kind == kind && e.target == r.target.trim())
                        .ok_or_else(|| {
                            PalError::not_found(format!(
                                "the changeset of {id} has no `{}: {}` for {doc}",
                                kind.verb(),
                                r.target.trim()
                            ))
                        })?;
                    selected.push((doc.to_string(), hit.clone()));
                }
            }
        }
        if selected.is_empty() {
            return Err(PalError::invalid("there are no edits to promote"));
        }
        // Dependencies first: their pending edits to the same documents come earlier.
        for dep in s.an.records.closure(id) {
            if let Some(dcs) = s.an.sets.get(&dep) {
                for de in &dcs.docs {
                    if selected.iter().any(|(d, _)| *d == de.doc) && !de.edits.is_empty() {
                        return Err(PalError::invariant(format!(
                            "{dep}, which {id} depends on, still has edits to {} in its changeset; promote those first",
                            de.doc
                        )));
                    }
                }
            }
        }
        let total = cs.edit_count();
        let remaining = total - selected.len();
        match (impl_kind.as_str(), remaining) {
            ("complete", r) if r > 0 => {
                return Err(PalError::invalid(format!(
                    "{r} edit(s) remain in the changeset; promote them too, or use partial"
                )));
            }
            ("partial", 0) => {
                return Err(PalError::invalid(
                    "every edit is promoted; use implementation: complete",
                ));
            }
            _ => {}
        }
        // Apply to the maintained documents.
        let mut working: BTreeMap<String, (Option<Source>, bool)> = BTreeMap::new();
        for (doc, e) in &selected {
            let path = s.snap.loc.maintained_file(doc).ok_or_else(|| {
                PalError::invalid(format!("`{doc}` is not a maintained document path"))
            })?;
            if s.snap.file(&path).is_some() {
                s.file(&path)?;
            }
            let entry = working.entry(doc.clone()).or_insert_with(|| {
                (
                    s.snap
                        .file(&path)
                        .and_then(|f| f.doc.as_ref())
                        .map(|d| d.src.clone()),
                    false,
                )
            });
            let res = match (e.kind, entry.0.as_mut()) {
                (EditKind::Create, None) => changeset::create_document(&cs_doc, e).map(|mut n| {
                    ensure_date(&mut n, &s.today);
                    entry.0 = Some(n);
                    entry.1 = true;
                }),
                (EditKind::Create, Some(_)) => Err(format!("`{doc}` already exists")),
                (_, Some(src)) => changeset::apply_edit(src, &cs_doc, e),
                (_, None) => Err(format!("`{doc}` does not exist")),
            };
            res.map_err(|m| {
                PalError::invariant(format!(
                    "`{}: {}` does not resolve against {doc}: {m}",
                    e.kind.verb(),
                    e.target
                ))
            })?;
        }
        for (doc, (src, _)) in &working {
            if let (Some(src), Some(path)) = (src, s.snap.loc.maintained_file(doc)) {
                s.put(&path, src)?;
            }
        }
        // Remove the promoted edits from the changeset.
        if remaining == 0 {
            s.remove(&cs_path)?;
        } else {
            let mut src = cs_doc.src.clone();
            let mut spans: Vec<(usize, usize)> = selected
                .iter()
                .map(|(_, e)| changeset::edit_range(&cs_doc, e))
                .collect();
            for de in &cs.docs {
                if de.edits.iter().all(|e| {
                    selected
                        .iter()
                        .any(|(d, x)| *d == de.doc && x.key() == e.key())
                }) {
                    let h = &cs_doc.headings[de.heading];
                    spans.retain(|(a, b)| !(*a >= h.start && *b <= h.end));
                    spans.push((h.start, h.end));
                }
            }
            spans.sort();
            for (a, b) in spans.into_iter().rev() {
                src.splice(a..b, &[]);
            }
            trim_trailing_blank(&mut src);
            s.put(&cs_path, &src)?;
        }
        // The record's time-varying fields.
        let rpath = s.snap.files[rec_file].path.clone();
        let rrel = s.rel(&rpath);
        let mut rsrc = s.source(&rpath)?;
        let scope = p
            .implementation_scope
            .as_deref()
            .map(edit::one_line)
            .filter(|x| !x.is_empty())
            .or_else(|| {
                impl_value
                    .as_deref()
                    .and_then(|v| v.split_once(" — ").map(|(_, r)| r.trim().to_string()))
            })
            .unwrap_or_else(|| DEFAULT_SCOPE.to_string());
        let verification = format!(
            "{} — {}; {}",
            p.verification.trim(),
            s.today,
            ver_note.replace(';', ",")
        );
        for (name, value) in [
            ("Implementation", format!("{impl_kind} — {scope}")),
            ("Implementers", implementers),
            ("Verification", verification),
        ] {
            edit::set_header_field(&mut rsrc, name, &value)
                .map_err(|m| PalError::parse(&rrel, 1, m))?;
        }
        s.put(&rpath, &rsrc)?;
        Ok(Vec::new())
    })
}

/// Splits a `Revised` value into its entries (used by status output).
pub fn revised_entries(value: &str) -> Vec<String> {
    if value.trim() == "none" {
        Vec::new()
    } else {
        split_entries(value)
    }
}
