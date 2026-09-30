//! Changesets: parsing, applying edits to maintained documents, and staging plans.
//!
//! Owns the changeset grammar (`Replace:`, `Insert after:`, `Insert into:`, `Delete:`,
//! `Create:`), resolving an edit's target section, applying edits line by line, the
//! dependency-ordered staging plan, the document state each record's edits see, and the
//! resolution and conflict checks behind P007.
//! Does not write files (the write tools do) and does not report findings.
//! Entry points: [`Changeset::parse`], [`apply_edit`], [`create_document`], [`plan`],
//! [`record_states`].

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::docs::{Role, Snapshot};
use crate::layout::{CHANGESET_TARGETS, split_logical};
use crate::records::{RecId, Records};
use crate::rst::{Doc, HOUSE_CHARS, Kind};
use crate::text::Source;
use crate::util::{is_kebab_rst, re};

/// The kind of an edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EditKind {
    /// Replace an existing section.
    Replace,
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
    /// Logical document path (`spec/x.rst`).
    pub doc: String,
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
    /// The logical path of the maintained document that the edit body holding 0-based
    /// line `line` writes into; `None` when the line lies outside every edit body.
    pub fn body_target(&self, doc: &Doc, line: usize) -> Option<&str> {
        self.docs
            .iter()
            .find(|de| {
                de.edits
                    .iter()
                    .any(|e| (doc.headings[e.heading].body_start..e.end).contains(&line))
            })
            .map(|de| de.doc.as_str())
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
        let edit_re = re(r"^(Replace|Insert after|Insert into|Delete|Create):\s*(.+)$");
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
            match split_logical(&path) {
                Some((_, name)) if is_kebab_rst(name) => {}
                _ => cs.problems.push((
                    dh.line,
                    format!("`{path}` cannot be edited by a changeset: {CHANGESET_TARGETS}"),
                )),
            }
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
                        format!("`{}` is not an edit (`Replace:`, `Insert after:`, `Insert into:`, `Delete:` or `Create:`)", h.title),
                    )),
                }
            }
            cs.docs.push(DocEdits {
                doc: path,
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
        EditKind::Replace | EditKind::InsertAfter | EditKind::InsertInto => {
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

fn maintained_source(snap: &Snapshot, logical: &str) -> Option<Source> {
    let path = snap.loc.maintained_file(logical)?;
    snap.file(&path)?.doc.as_ref().map(|d| d.src.clone())
}

/// Applies the edits of `id`'s changeset to `docs` (working copies keyed by logical
/// path). Edits that fail are collected in `failures` and skipped.
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
        if split_logical(&de.doc).is_none() {
            continue;
        }
        let entry = docs
            .entry(de.doc.clone())
            .or_insert_with(|| maintained_source(snap, &de.doc));
        for e in &de.edits {
            let res = match (e.kind, entry.as_mut()) {
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
    let mut applied: BTreeMap<String, Vec<RecId>> = BTreeMap::new();
    let mut failures = Vec::new();
    for id in records.dependency_order(&accepted) {
        let Some(cs) = sets.get(&id) else { continue };
        apply_changeset(snap, id, cs, &mut working, &mut failures);
        for de in &cs.docs {
            applied.entry(de.doc.clone()).or_default().push(id);
        }
    }
    let mut out = StagingPlan {
        docs: BTreeMap::new(),
        failures,
    };
    for (logical, src) in working {
        if let Some(source) = src
            && applied.contains_key(&logical)
        {
            out.docs.insert(
                logical.clone(),
                Staged {
                    source,
                    records: applied.remove(&logical).unwrap_or_default(),
                    from_existing: maintained_source(snap, &logical).is_some(),
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

    /// The logical path of the document at `path` when an edit of this state holds it
    /// and it exists there after the record's own edits.
    pub fn held_at(&self, snap: &Snapshot, path: &Path) -> Option<&str> {
        self.own
            .iter()
            .find(|(l, src)| {
                src.is_some() && snap.loc.maintained_file(l).is_some_and(|p| p == path)
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
