//! Changesets: parsing, applying edits to maintained documents, and staging plans.
//!
//! Owns the changeset grammar (`Replace:`, `Replace body:`, `Insert after:`, `Insert
//! into:`, `Delete:`, `Create:`), resolving an edit's target section, applying edits
//! line by line, the dependency-ordered staging plan, the document state each record's
//! edits see, and the resolution and conflict checks behind P007. Which document a
//! changeset may edit is decided by [`Snapshot::target`].
//! Does not write files (the write tools do) and does not report findings.
//! Entry points: [`Changeset::parse`], [`apply_edit`], [`create_document`], [`plan`],
//! [`record_states`].

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::docs::{Role, Snapshot};
use crate::layout::{CREATE_TARGETS, Target};
use crate::records::{RecId, Records};
use crate::rst::{Doc, HOUSE_CHARS, Kind};
use crate::text::Source;
use crate::util::re;

/// The kind of an edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EditKind {
    /// Replace an existing section together with its subsections.
    Replace,
    /// Replace an existing section's heading and body up to its first subsection,
    /// keeping the subsections.
    ReplaceBody,
    /// Insert a section after an existing one.
    InsertAfter,
    /// Insert a new last subsection into an existing section.
    InsertInto,
    /// Delete a section.
    Delete,
    /// Create a new document.
    Create,
}

impl EditKind {
    /// The verb written in the changeset heading.
    pub fn verb(self) -> &'static str {
        match self {
            EditKind::Replace => "Replace",
            EditKind::ReplaceBody => "Replace body",
            EditKind::InsertAfter => "Insert after",
            EditKind::InsertInto => "Insert into",
            EditKind::Delete => "Delete",
            EditKind::Create => "Create",
        }
    }

    /// Parses a verb.
    pub fn parse(verb: &str) -> Option<EditKind> {
        [
            EditKind::Replace,
            EditKind::ReplaceBody,
            EditKind::InsertAfter,
            EditKind::InsertInto,
            EditKind::Delete,
            EditKind::Create,
        ]
        .into_iter()
        .find(|k| k.verb() == verb)
    }
}

/// One edit heading and the range of its body in the changeset file.
#[derive(Clone, Debug)]
pub struct Edit {
    /// Kind.
    pub kind: EditKind,
    /// Section title (with parent prefix) or, for `Create`, the new document's title.
    pub target: String,
    /// 0-based heading line.
    pub line: usize,
    /// Heading index in the changeset's `Doc::headings`.
    pub heading: usize,
    /// Exclusive end of the edit (start of the next edit or document heading).
    pub end: usize,
}

impl Edit {
    /// Identity of an edit inside a changeset: `(kind, target)`.
    pub fn key(&self) -> (EditKind, String) {
        (self.kind, self.target.clone())
    }
}

/// The edits a changeset makes to one maintained document.
#[derive(Clone, Debug)]
pub struct DocEdits {
    /// The document as the changeset spells it (`spec/x.rst`, `principles.rst`,
    /// `docs/guide.rst`).
    pub doc: String,
    /// The document it resolves to; `None` when a changeset may not edit it (the
    /// changeset's problems say why), and then no edit of it is applied.
    pub target: Option<Target>,
    /// 0-based heading line.
    pub line: usize,
    /// Heading index.
    pub heading: usize,
    /// Edits in file order.
    pub edits: Vec<Edit>,
}

/// A parsed changeset file.
#[derive(Clone, Debug)]
pub struct Changeset {
    /// Index into `Snapshot::files`.
    pub file: usize,
    /// Record named by the title.
    pub id: Option<RecId>,
    /// Record named by the file name.
    pub file_id: Option<RecId>,
    /// Documents with their edits.
    pub docs: Vec<DocEdits>,
    /// Structural problems `(0-based line, message)`.
    pub problems: Vec<(usize, String)>,
}

impl Changeset {
    /// The document that the edit body holding 0-based line `line` writes into; `None`
    /// when the line lies outside every edit body or the document is not a target.
    pub fn body_target(&self, doc: &Doc, line: usize) -> Option<&Target> {
        self.docs
            .iter()
            .find(|de| {
                de.edits
                    .iter()
                    .any(|e| (doc.headings[e.heading].body_start..e.end).contains(&line))
            })
            .and_then(|de| de.target.as_ref())
    }

    /// The record this changeset belongs to.
    pub fn record(&self) -> Option<RecId> {
        self.id.or(self.file_id)
    }

    /// Total number of edits.
    pub fn edit_count(&self) -> usize {
        self.docs.iter().map(|d| d.edits.len()).sum()
    }

    /// Reads changeset `file` of `snap`. `None` when the file did not parse as UTF-8.
    pub fn parse(snap: &Snapshot, file: usize) -> Option<Changeset> {
        let f = &snap.files[file];
        let doc = f.doc.as_ref()?;
        let title_re = re(r"^Changeset:\s*(RFC|ADR)-(\d{4})$");
        let name_re = re(r"^(rfc|adr)-(\d{4})\.rst$");
        let edit_re =
            re(r"^(Replace body|Replace|Insert after|Insert into|Delete|Create):\s*(.+)$");
        let id = doc
            .title()
            .and_then(|t| title_re.captures(&t.title))
            .and_then(|c| RecId::parse(&format!("{}-{}", &c[1], &c[2])));
        let file_id = name_re
            .captures(&f.name)
            .and_then(|c| RecId::parse(&format!("{}-{}", &c[1], &c[2])));
        let mut cs = Changeset {
            file,
            id,
            file_id,
            docs: Vec::new(),
            problems: Vec::new(),
        };
        for (di, dh) in doc.headings_at(2) {
            let path = dh.title.clone();
            let target = match snap.target(&path) {
                Ok(t) => Some(t),
                Err(msg) => {
                    cs.problems.push((dh.line, msg));
                    None
                }
            };
            let mut edits = Vec::new();
            for ci in doc.children(di) {
                let h = &doc.headings[ci];
                match edit_re.captures(&h.title) {
                    Some(c) => {
                        if let Some(kind) = EditKind::parse(&c[1]) {
                            edits.push(Edit {
                                kind,
                                target: c[2].trim().to_string(),
                                line: h.line,
                                heading: ci,
                                end: h.end,
                            });
                        }
                    }
                    None => cs.problems.push((
                        h.line,
                        format!("`{}` is not an edit (`Replace:`, `Replace body:`, `Insert after:`, `Insert into:`, `Delete:` or `Create:`)", h.title),
                    )),
                }
            }
            cs.docs.push(DocEdits {
                doc: path,
                target,
                line: dh.line,
                heading: di,
                edits,
            });
        }
        Some(cs)
    }
}

fn underline(level: usize, title: &str) -> String {
    let c = HOUSE_CHARS[level.clamp(1, 5) - 1];
    c.to_string().repeat(title.chars().count().max(3))
}

/// The edit body of `edit` with headings moved to their level in the target document:
/// a `^` heading (level 4) becomes `first_level`, a `"` heading (level 5) the next one.
fn converted_body(cs: &Doc, edit: &Edit, first_level: usize) -> Result<Vec<String>, String> {
    let h = &cs.headings[edit.heading];
    let mut out: Vec<String> = Vec::new();
    let mut heading_at: BTreeMap<usize, usize> = BTreeMap::new();
    for (i, sub) in cs.headings.iter().enumerate() {
        if sub.start > h.start && sub.start < edit.end {
            heading_at.insert(sub.start, i);
        }
    }
    let mut l = h.body_start;
    while l < edit.end {
        if let Some(&hi) = heading_at.get(&l) {
            let sub = &cs.headings[hi];
            if sub.level != 4 && sub.level != 5 {
                return Err(format!(
                    "line {}: a heading inside an edit must be underlined with `^` or `\"`",
                    sub.line + 1
                ));
            }
            let level = first_level + (sub.level - 4);
            if level > 5 {
                return Err(format!(
                    "line {}: `{}` would be nested deeper than the house style allows",
                    sub.line + 1,
                    sub.title
                ));
            }
            out.push(sub.title.clone());
            out.push(underline(level, &sub.title));
            l = sub.underline + 1;
            continue;
        }
        out.push(cs.src.text(l).to_string());
        l += 1;
    }
    while out.first().is_some_and(|s| s.trim().is_empty()) {
        out.remove(0);
    }
    while out.last().is_some_and(|s| s.trim().is_empty()) {
        out.pop();
    }
    Ok(out)
}

/// Finds the section `target` (`Title` or `Parent / Title`) in `doc`.
pub fn resolve_section(doc: &Doc, target: &str) -> Result<usize, String> {
    let comps: Vec<&str> = target.split(" / ").map(str::trim).collect();
    let Some((last, parents)) = comps.split_last() else {
        return Err("the section title is empty".to_string());
    };
    let mut found = Vec::new();
    for (i, h) in doc.headings.iter().enumerate() {
        if h.level < 2 || h.title != *last {
            continue;
        }
        let anc = doc.ancestors(i);
        if anc.len() >= parents.len()
            && anc[anc.len() - parents.len()..]
                .iter()
                .map(String::as_str)
                .eq(parents.iter().copied())
        {
            found.push(i);
        }
    }
    match found.len() {
        0 => Err(format!("section `{target}` does not exist")),
        1 => Ok(found[0]),
        _ => Err(format!(
            "section `{target}` is not unique; prefix it with its parent titles (`Parent / {last}`)"
        )),
    }
}

/// Applies one edit of changeset `cs` to the maintained document `target`.
pub fn apply_edit(target: &mut Source, cs: &Doc, edit: &Edit) -> Result<(), String> {
    if edit.kind == EditKind::Create {
        return Err(
            "a Create edit makes a new document; it cannot edit an existing one".to_string(),
        );
    }
    let doc = Doc::parse(target.clone());
    let ti = resolve_section(&doc, &edit.target)?;
    let t = doc.headings[ti].clone();
    let n = target.len();
    match edit.kind {
        EditKind::Delete => {
            target.splice(t.start..t.end, &[]);
            if t.end >= n {
                while target
                    .lines
                    .last()
                    .is_some_and(|l| l.text.trim().is_empty())
                {
                    target.lines.pop();
                }
            }
            Ok(())
        }
        EditKind::Replace
        | EditKind::ReplaceBody
        | EditKind::InsertAfter
        | EditKind::InsertInto => {
            let first_level = if edit.kind == EditKind::InsertInto {
                t.level + 1
            } else {
                t.level
            };
            if first_level > 5 {
                return Err(format!("`{}` has no room for a subsection", edit.target));
            }
            let block = converted_body(cs, edit, first_level)?;
            let first = block
                .first()
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            let starts_with_heading = block.get(1).is_some_and(|u| {
                u.chars()
                    .next()
                    .is_some_and(|c| c == HOUSE_CHARS[first_level - 1])
                    && u.chars().all(|c| c == HOUSE_CHARS[first_level - 1])
            });
            if first.is_empty() || !starts_with_heading {
                return Err(format!(
                    "the body of `{}: {}` must begin with the section's title underlined with `^`",
                    edit.kind.verb(),
                    edit.target
                ));
            }
            match edit.kind {
                EditKind::Replace => {
                    let mut new = block;
                    if t.end < n {
                        new.push(String::new());
                    }
                    target.splice(t.start..t.end, &new);
                }
                EditKind::ReplaceBody => {
                    let h = &cs.headings[edit.heading];
                    let headings = cs
                        .headings
                        .iter()
                        .filter(|sub| sub.start > h.start && sub.start < edit.end)
                        .count();
                    if headings > 1 {
                        return Err(format!(
                            "the body of `{}: {}` holds the section's title underlined with `^` and text with no further heading; the subsections stay as they are (use `Replace:` to replace them too, or `Insert into:` to add one)",
                            edit.kind.verb(),
                            edit.target
                        ));
                    }
                    let mut new = block;
                    if t.body_end < n {
                        new.push(String::new());
                    }
                    target.splice(t.start..t.body_end, &new);
                }
                _ => {
                    let at = t.end;
                    let mut new = Vec::new();
                    if at == n {
                        if target
                            .lines
                            .last()
                            .is_some_and(|l| !l.text.trim().is_empty())
                        {
                            new.push(String::new());
                        }
                        new.extend(block);
                    } else {
                        if at > 0 && !target.text(at - 1).trim().is_empty() {
                            new.push(String::new());
                        }
                        new.extend(block);
                        new.push(String::new());
                    }
                    target.splice(at..at, &new);
                }
            }
            Ok(())
        }
        EditKind::Create => Err("unreachable".to_string()),
    }
}

/// Builds the new document of a `Create:` edit.
pub fn create_document(cs: &Doc, edit: &Edit) -> Result<Source, String> {
    let body = converted_body(cs, edit, 2)?;
    let mut lines = vec![
        edit.target.clone(),
        underline(1, &edit.target),
        String::new(),
    ];
    lines.extend(body);
    Ok(Source::from_lines(&lines, crate::text::Eol::Lf))
}

/// Why an edit did not apply.
#[derive(Clone, Debug)]
pub struct Failure {
    /// The record whose changeset holds the edit.
    pub record: RecId,
    /// Index of the changeset file in the snapshot.
    pub file: usize,
    /// 0-based line of the edit heading.
    pub line: usize,
    /// Explanation.
    pub message: String,
}

/// The dependency-ordered result of applying accepted changesets.
#[derive(Default)]
pub struct StagingPlan {
    /// Staged documents by logical path.
    pub docs: BTreeMap<String, Staged>,
    /// Edits that did not resolve.
    pub failures: Vec<Failure>,
}

/// One staged document.
pub struct Staged {
    /// The document it mirrors.
    pub target: Target,
    /// The document with every accepted edit applied.
    pub source: Source,
    /// Records whose edits were applied, in order.
    pub records: Vec<RecId>,
    /// Line-ending style of the base document or first changeset.
    pub from_existing: bool,
}

/// All changesets of a snapshot, keyed by record.
pub fn collect(snap: &Snapshot) -> BTreeMap<RecId, Changeset> {
    let mut out = BTreeMap::new();
    for (i, f) in snap.files.iter().enumerate() {
        if f.role == Role::Changeset
            && let Some(cs) = Changeset::parse(snap, i)
            && let Some(id) = cs.record()
        {
            out.entry(id).or_insert(cs);
        }
    }
    out
}

/// The current lines of the document a target spelling names, when it is a target.
fn maintained_source(snap: &Snapshot, spelling: &str) -> Option<Source> {
    snap.target_source(&snap.target(spelling).ok()?)
}

/// Applies the edits of `id`'s changeset to `docs` (working copies keyed by the
/// target's spelling). Edits that fail are collected in `failures` and skipped; a
/// document that is not a target is never touched.
fn apply_changeset(
    snap: &Snapshot,
    id: RecId,
    cs: &Changeset,
    docs: &mut BTreeMap<String, Option<Source>>,
    failures: &mut Vec<Failure>,
) {
    let Some(cs_doc) = snap.files[cs.file].doc.as_ref() else {
        return;
    };
    for de in &cs.docs {
        let Some(target) = &de.target else {
            continue;
        };
        let entry = docs
            .entry(de.doc.clone())
            .or_insert_with(|| snap.target_source(target));
        for e in &de.edits {
            let res = match (e.kind, entry.as_mut()) {
                (EditKind::Create, _) if !target.can_create() => {
                    Err(format!("`{}`: {CREATE_TARGETS}", de.doc))
                }
                (EditKind::Create, None) => create_document(cs_doc, e).map(|s| *entry = Some(s)),
                (EditKind::Create, Some(_)) => Err(format!(
                    "`{}` already exists; a Create edit needs a new document",
                    de.doc
                )),
                (_, Some(src)) => apply_edit(src, cs_doc, e),
                (_, None) => Err(format!(
                    "`{}` does not exist and no edit creates it",
                    de.doc
                )),
            };
            if let Err(message) = res {
                failures.push(Failure {
                    record: id,
                    file: cs.file,
                    line: e.line,
                    message,
                });
            }
        }
    }
}

/// Statuses whose changesets take part in checks.
pub fn is_checked_status(status: &str) -> bool {
    matches!(status, "Draft" | "Proposed" | "Accepted")
}

/// Builds the staging documents: every `Accepted` record's changeset applied to its
/// maintained documents in dependency order.
pub fn plan(snap: &Snapshot, records: &Records, sets: &BTreeMap<RecId, Changeset>) -> StagingPlan {
    let accepted: Vec<RecId> = sets
        .keys()
        .copied()
        .filter(|id| records.get(*id).and_then(|r| r.status.as_deref()) == Some("Accepted"))
        .collect();
    let mut working: BTreeMap<String, Option<Source>> = BTreeMap::new();
    let mut applied: BTreeMap<String, (Target, Vec<RecId>)> = BTreeMap::new();
    let mut failures = Vec::new();
    for id in records.dependency_order(&accepted) {
        let Some(cs) = sets.get(&id) else { continue };
        apply_changeset(snap, id, cs, &mut working, &mut failures);
        for de in &cs.docs {
            if let Some(t) = &de.target {
                applied
                    .entry(de.doc.clone())
                    .or_insert_with(|| (t.clone(), Vec::new()))
                    .1
                    .push(id);
            }
        }
    }
    let mut out = StagingPlan {
        docs: BTreeMap::new(),
        failures,
    };
    for (spelling, src) in working {
        if let Some(source) = src
            && let Some((target, records)) = applied.remove(&spelling)
        {
            let from_existing = snap.target_source(&target).is_some();
            out.docs.insert(
                spelling,
                Staged {
                    target,
                    source,
                    records,
                    from_existing,
                },
            );
        }
    }
    out
}

/// The maintained documents as one record's changeset sees them: the edits of the
/// record's dependency closure applied in dependency order, then its own edits. Only
/// documents some edit touches are held; every other one is read from disk.
pub struct RecordState {
    /// Touched documents after the closure's edits, by logical path (`None`: no such
    /// document).
    pub base: BTreeMap<String, Option<Source>>,
    /// `base` with the record's own edits applied too.
    pub own: BTreeMap<String, Option<Source>>,
    /// The record's own edits that did not resolve.
    pub failures: Vec<Failure>,
}

impl RecordState {
    /// Logical document `logical` before the record's own edits (`own` false) or after
    /// them (`own` true); `None` when it neither exists on disk nor is created.
    pub fn source(&self, snap: &Snapshot, logical: &str, own: bool) -> Option<Source> {
        let map = if own { &self.own } else { &self.base };
        match map.get(logical) {
            Some(held) => held.clone(),
            None => maintained_source(snap, logical),
        }
    }

    /// The spelling of the document at `path` when an edit of this state holds it and
    /// it exists there after the record's own edits.
    pub fn held_at(&self, snap: &Snapshot, path: &Path) -> Option<&str> {
        self.own
            .iter()
            .find(|(l, src)| {
                src.is_some()
                    && snap
                        .target(l)
                        .is_ok_and(|t| snap.loc.target_file(&t) == path)
            })
            .map(|(l, _)| l.as_str())
    }
}

/// The document state of every record with an identifier: its dependency closure's
/// changesets applied, then its own. The basis for resolving edits (P007), `Changes`
/// targets (P006) and links inside edit bodies (P004).
pub fn record_states(
    snap: &Snapshot,
    records: &Records,
    sets: &BTreeMap<RecId, Changeset>,
) -> BTreeMap<RecId, RecordState> {
    let mut ids: BTreeSet<RecId> = records.list.iter().filter_map(Records::id_of).collect();
    ids.extend(sets.keys().copied());
    let mut out = BTreeMap::new();
    for id in ids {
        let closure: Vec<RecId> = records
            .dependency_order(&records.closure(id).into_iter().collect::<Vec<_>>())
            .into_iter()
            .filter(|c| sets.contains_key(c))
            .collect();
        let mut base: BTreeMap<String, Option<Source>> = BTreeMap::new();
        let mut ignored = Vec::new();
        for dep in closure {
            if let Some(dep_cs) = sets.get(&dep) {
                apply_changeset(snap, dep, dep_cs, &mut base, &mut ignored);
            }
        }
        let mut own = base.clone();
        let mut failures = Vec::new();
        if let Some(cs) = sets.get(&id) {
            apply_changeset(snap, id, cs, &mut own, &mut failures);
        }
        out.insert(
            id,
            RecordState {
                base,
                own,
                failures,
            },
        );
    }
    out
}

/// The edits of every checked record that do not resolve against its maintained
/// documents with only the record's dependency closure applied.
pub fn check_resolution(records: &Records, states: &BTreeMap<RecId, RecordState>) -> Vec<Failure> {
    let mut out = Vec::new();
    for (id, st) in states {
        let checked = records
            .get(*id)
            .and_then(|r| r.status.as_deref())
            .is_some_and(is_checked_status);
        if checked {
            out.extend(st.failures.iter().cloned());
        }
    }
    out
}

/// Pairs of records that edit the same section with no dependency between them:
/// `(a, b, document, target)`.
pub fn find_conflicts(
    records: &Records,
    sets: &BTreeMap<RecId, Changeset>,
) -> Vec<(RecId, RecId, String, String)> {
    let mut owners: BTreeMap<(String, String), BTreeSet<RecId>> = BTreeMap::new();
    for (id, cs) in sets {
        let checked = records
            .get(*id)
            .and_then(|r| r.status.as_deref())
            .is_some_and(is_checked_status);
        if !checked {
            continue;
        }
        for de in &cs.docs {
            for e in &de.edits {
                let target = if e.kind == EditKind::Create {
                    "(new document)".to_string()
                } else {
                    e.target.clone()
                };
                owners
                    .entry((de.doc.clone(), target))
                    .or_default()
                    .insert(*id);
            }
        }
    }
    let mut out = Vec::new();
    for ((doc, target), ids) in owners {
        let ids: Vec<RecId> = ids.into_iter().collect();
        for (i, a) in ids.iter().enumerate() {
            for b in &ids[i + 1..] {
                if !records.closure(*a).contains(b) && !records.closure(*b).contains(a) {
                    out.push((*a, *b, doc.clone(), target.clone()));
                }
            }
        }
    }
    out
}

/// Line ranges of a changeset used by the write tools.
pub fn edit_range(doc: &Doc, edit: &Edit) -> (usize, usize) {
    (doc.headings[edit.heading].start, edit.end)
}

/// Text lines of an edit heading and body for a new edit.
pub fn edit_lines(kind: EditKind, target: &str, body: &str) -> Vec<String> {
    let title = format!("{}: {}", kind.verb(), target);
    let mut out = vec![title.clone(), underline(3, &title)];
    let body_lines: Vec<&str> = body.lines().collect();
    let mut start = 0;
    let mut end = body_lines.len();
    while start < end && body_lines[start].trim().is_empty() {
        start += 1;
    }
    while end > start && body_lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    if start < end {
        out.push(String::new());
        out.extend(
            body_lines[start..end]
                .iter()
                .map(|l| l.trim_end_matches('\r').to_string()),
        );
    }
    out
}

/// Whether line `i` of `doc` belongs to a heading (title, underline or overline).
pub fn is_heading_line(doc: &Doc, i: usize) -> bool {
    matches!(doc.kinds[i], Kind::Title | Kind::Adornment)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cs_and_edit(body: &str, verb: &str, target: &str) -> (Doc, Edit) {
        let head = format!("{verb}: {target}");
        let text = format!(
            "Changeset: RFC-0001\n===================\n\nspec/x.rst\n----------\n\n{head}\n{}\n\n{body}\n",
            "~".repeat(head.chars().count())
        );
        let doc = Doc::parse(Source::from_text(&text));
        let h = doc
            .headings
            .iter()
            .position(|h| h.level == 3)
            .expect("edit heading");
        let e = Edit {
            kind: EditKind::parse(verb).expect("verb"),
            target: target.to_string(),
            line: doc.headings[h].line,
            heading: h,
            end: doc.headings[h].end,
        };
        (doc, e)
    }

    const TARGET: &str = "Topic\n=====\n\n:Status: Contract\n\nOne\n---\n\nfirst\n\nTwo\n---\n\nsecond\n\nSub\n~~~\n\nsub text\n\nThree\n-----\n\nthird\n";

    #[test]
    fn replace_changes_only_the_named_section() {
        let (cs, e) = cs_and_edit(
            "New two\n^^^^^^^\n\nreplaced\n\nInner\n\"\"\"\"\"\n\ndeep",
            "Replace",
            "Two",
        );
        let mut t = Source::from_text(TARGET);
        apply_edit(&mut t, &cs, &e).expect("apply");
        let text = t.to_text();
        assert!(
            text.contains("New two\n-------\n\nreplaced\n\nInner\n~~~~~\n\ndeep\n\nThree\n"),
            "{text}"
        );
        assert!(text.starts_with("Topic\n=====\n\n:Status: Contract\n\nOne\n---\n\nfirst\n\n"));
        assert!(!text.contains("second"));
    }

    #[test]
    fn insert_after_and_into() {
        let (cs, e) = cs_and_edit("Added\n^^^^^\n\nnew text", "Insert after", "One");
        let mut t = Source::from_text(TARGET);
        apply_edit(&mut t, &cs, &e).expect("apply");
        assert!(
            t.to_text()
                .contains("first\n\nAdded\n-----\n\nnew text\n\nTwo\n---")
        );

        let (cs, e) = cs_and_edit("Fresh\n^^^^^\n\nfresh text", "Insert into", "Two");
        let mut t = Source::from_text(TARGET);
        apply_edit(&mut t, &cs, &e).expect("apply");
        assert!(
            t.to_text()
                .contains("sub text\n\nFresh\n~~~~~\n\nfresh text\n\nThree\n-----"),
            "{}",
            t.to_text()
        );
    }

    #[test]
    fn delete_removes_section_and_subsections() {
        let (cs, e) = cs_and_edit("", "Delete", "Two");
        let mut t = Source::from_text(TARGET);
        apply_edit(&mut t, &cs, &e).expect("apply");
        let text = t.to_text();
        assert!(!text.contains("Two") && !text.contains("Sub") && text.contains("Three"));
    }

    const TWO_SUBS: &str = "Topic\n=====\n\n:Status: Contract\n\nOne\n---\n\nfirst\n\nTwo\n---\n\nsecond\n\nSub A\n~~~~~\n\na text\n\nSub B\n~~~~~\n\nb text\n\nThree\n-----\n\nthird\n";

    #[test]
    fn replace_body_keeps_the_subsections() {
        for eol in ["\n", "\r\n"] {
            let (cs, e) = cs_and_edit(
                "Two renamed\n^^^^^^^^^^^\n\nnew body",
                "Replace body",
                "Two",
            );
            let mut t = Source::from_text(&TWO_SUBS.replace('\n', eol));
            apply_edit(&mut t, &cs, &e).expect("apply");
            let text = t.to_text().replace("\r\n", "\n");
            assert!(
                text.contains("first\n\nTwo renamed\n-----------\n\nnew body\n\nSub A\n~~~~~\n\na text\n\nSub B\n~~~~~\n\nb text\n\nThree\n"),
                "{text}"
            );
            assert!(!text.contains("second"));
            if eol == "\r\n" {
                assert!(!t.to_text().replace("\r\n", "").contains('\n'));
            }
        }
    }

    #[test]
    fn replace_body_with_the_heading_alone_keeps_one_blank_line() {
        let (cs, e) = cs_and_edit("Two\n^^^", "Replace body", "Two");
        let mut t = Source::from_text(TWO_SUBS);
        apply_edit(&mut t, &cs, &e).expect("apply");
        assert!(
            t.to_text().contains("first\n\nTwo\n---\n\nSub A\n~~~~~\n"),
            "{}",
            t.to_text()
        );
        // With no subsection, the next sibling follows after one blank line.
        let (cs, e) = cs_and_edit("One\n^^^", "Replace body", "One");
        let mut t = Source::from_text(TWO_SUBS);
        apply_edit(&mut t, &cs, &e).expect("apply");
        assert!(
            t.to_text()
                .contains(":Status: Contract\n\nOne\n---\n\nTwo\n---\n"),
            "{}",
            t.to_text()
        );
        // The last section of the document ends the file.
        let (cs, e) = cs_and_edit("Three\n^^^^^", "Replace body", "Three");
        let mut t = Source::from_text(TWO_SUBS);
        apply_edit(&mut t, &cs, &e).expect("apply");
        assert!(
            t.to_text().ends_with("b text\n\nThree\n-----\n"),
            "{}",
            t.to_text()
        );
    }

    #[test]
    fn replace_body_with_a_second_heading_is_refused() {
        let (cs, e) = cs_and_edit(
            "Two\n^^^\n\ntext\n\nInner\n\"\"\"\"\"\n\ndeep",
            "Replace body",
            "Two",
        );
        let mut t = Source::from_text(TWO_SUBS);
        let err = apply_edit(&mut t, &cs, &e).expect_err("second heading");
        assert!(err.contains("no further heading"), "{err}");
        assert_eq!(t.to_text(), TWO_SUBS);
    }

    #[test]
    fn replace_removes_the_subsections_that_replace_body_keeps() {
        let (cs, e) = cs_and_edit("Two\n^^^\n\nnew body", "Replace", "Two");
        let mut t = Source::from_text(TWO_SUBS);
        apply_edit(&mut t, &cs, &e).expect("apply");
        let text = t.to_text();
        assert!(text.contains("Two\n---\n\nnew body\n\nThree\n"), "{text}");
        assert!(!text.contains("Sub A") && !text.contains("Sub B"));
    }

    #[test]
    fn a_renamed_section_takes_later_edits_by_its_new_name() {
        let mut t = Source::from_text(TWO_SUBS);
        let (cs, e) = cs_and_edit("Second\n^^^^^^\n\nnew body", "Replace body", "Two");
        apply_edit(&mut t, &cs, &e).expect("rename");
        let (cs, e) = cs_and_edit("Fresh\n^^^^^\n\nfresh text", "Insert into", "Second");
        apply_edit(&mut t, &cs, &e).expect("insert into the new name");
        let text = t.to_text();
        assert!(
            text.contains("Second\n------\n\nnew body\n\nSub A\n~~~~~\n\na text\n\nSub B\n~~~~~\n\nb text\n\nFresh\n~~~~~\n\nfresh text\n\nThree\n"),
            "{text}"
        );
        let (cs, e) = cs_and_edit("Two\n^^^", "Replace body", "Two");
        assert!(apply_edit(&mut t, &cs, &e).is_err_and(|m| m.contains("does not exist")));
    }

    #[test]
    fn replace_body_is_a_kind_of_its_own() {
        assert_eq!(EditKind::parse("Replace body"), Some(EditKind::ReplaceBody));
        assert_eq!(EditKind::ReplaceBody.verb(), "Replace body");
        assert_ne!(EditKind::parse("Replace"), Some(EditKind::ReplaceBody));
    }

    #[test]
    fn parent_prefix_disambiguates() {
        let doc = Doc::parse(Source::from_text(
            "Title\n=====\n\nAaa\n---\n\nSame\n~~~~\n\nx\n\nBbb\n---\n\nSame\n~~~~\n\ny\n",
        ));
        assert!(resolve_section(&doc, "Same").is_err_and(|e| e.contains("not unique")));
        assert!(resolve_section(&doc, "Bbb / Same").is_ok());
        assert!(resolve_section(&doc, "Missing").is_err());
    }

    #[test]
    fn crlf_target_keeps_crlf() {
        let (cs, e) = cs_and_edit("Added\n^^^^^\n\nnew text", "Insert after", "One");
        let mut t = Source::from_text(&TARGET.replace('\n', "\r\n"));
        apply_edit(&mut t, &cs, &e).expect("apply");
        assert!(
            t.to_text()
                .contains("Added\r\n-----\r\n\r\nnew text\r\n\r\nTwo")
        );
        assert!(!t.to_text().replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn create_builds_a_document() {
        let (cs, e) = cs_and_edit(
            ":Status: Contract\n\nScope\n^^^^^\n\ntext\n\nSub\n\"\"\"\n\nmore",
            "Create",
            "New spec",
        );
        let s = create_document(&cs, &e).expect("create");
        assert_eq!(
            s.to_text(),
            "New spec\n========\n\n:Status: Contract\n\nScope\n-----\n\ntext\n\nSub\n~~~\n\nmore\n"
        );
    }
}
