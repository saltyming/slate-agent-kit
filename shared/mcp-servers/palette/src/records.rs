//! RFC and ADR records and the relations between them.
//!
//! Owns reading a record's header into typed fields, the relation graph (Depends,
//! Supersedes, Related, Within and body links), the dependency closure, incoming links,
//! supersession, cycle detection and dependency ordering. Does not report findings
//! (that is `lint`) and does not write files.
//! Entry point: [`Records::build`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;

use crate::docs::{Role, Snapshot};
use crate::layout::Family;
use crate::rst::{Field, is_external, split_anchor};
use crate::schema::split_entries;
use crate::util::{join_lexical, re};

/// RFC or ADR.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RecordKind {
    /// Request for comments.
    Rfc,
    /// Architecture decision record.
    Adr,
}

impl RecordKind {
    /// `RFC` or `ADR`.
    pub fn upper(self) -> &'static str {
        match self {
            RecordKind::Rfc => "RFC",
            RecordKind::Adr => "ADR",
        }
    }

    /// `rfc` or `adr`.
    pub fn lower(self) -> &'static str {
        match self {
            RecordKind::Rfc => "rfc",
            RecordKind::Adr => "adr",
        }
    }

    /// The family holding this kind of record.
    pub fn family(self) -> Family {
        match self {
            RecordKind::Rfc => Family::Rfc,
            RecordKind::Adr => Family::Adr,
        }
    }

    /// Parses `rfc`/`RFC`/`adr`/`ADR`.
    pub fn parse(s: &str) -> Option<RecordKind> {
        match s.to_ascii_lowercase().as_str() {
            "rfc" => Some(RecordKind::Rfc),
            "adr" => Some(RecordKind::Adr),
            _ => None,
        }
    }
}

/// A record identifier such as `RFC-0004`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RecId {
    /// Kind.
    pub kind: RecordKind,
    /// Number.
    pub number: u32,
}

impl fmt::Display for RecId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{:04}", self.kind.upper(), self.number)
    }
}

impl RecId {
    /// Parses `RFC-0004` (case-insensitive prefix, four digits).
    pub fn parse(s: &str) -> Option<RecId> {
        let s = s.trim();
        let (k, n) = s.split_once('-')?;
        if n.len() != 4 || !n.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        Some(RecId {
            kind: RecordKind::parse(k)?,
            number: n.parse().ok()?,
        })
    }
}

/// A relation entry `RFC-0003 (what is used)`.
#[derive(Clone, Debug)]
pub struct RelEntry {
    /// Target.
    pub id: RecId,
    /// Parenthetical text, without the `in part:` marker.
    pub note: Option<String>,
    /// `Supersedes` only: the entry replaces a part (`in part: ...`), not the record.
    pub partial: bool,
    /// 0-based line of the field the entry is in.
    pub line: usize,
}

/// A maintained-document reference `spec/x.rst (Section; Other)`.
#[derive(Clone, Debug)]
pub struct DocRef {
    /// Logical document path.
    pub doc: String,
    /// Sections named in the parenthetical.
    pub sections: Vec<String>,
    /// 0-based line of the field.
    pub line: usize,
}

/// How an edge arose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EdgeKind {
    /// `Depends`.
    Depends,
    /// `Supersedes`.
    Supersedes,
    /// `Related`.
    Related,
    /// `Amends` (a legacy relation of older records).
    Amends,
    /// `Within` (ADR to RFC).
    Within,
    /// A hyperlink in the body.
    Link,
}

impl EdgeKind {
    /// Label used in status output.
    pub fn label(self) -> &'static str {
        match self {
            EdgeKind::Depends => "Depends",
            EdgeKind::Supersedes => "Supersedes",
            EdgeKind::Related => "Related",
            EdgeKind::Amends => "Amends",
            EdgeKind::Within => "Within",
            EdgeKind::Link => "link",
        }
    }
}

/// A link between two records.
#[derive(Clone, Debug)]
pub struct Edge {
    /// The linking record.
    pub from: RecId,
    /// The linked record.
    pub to: RecId,
    /// How it arose.
    pub kind: EdgeKind,
    /// 0-based line in the linking file.
    pub line: usize,
    /// Parenthetical text for header relations.
    pub note: Option<String>,
}

/// One record file read into typed fields.
#[derive(Clone, Debug)]
pub struct Record {
    /// Index into `Snapshot::files`.
    pub file: usize,
    /// Identifier from the title.
    pub id: Option<RecId>,
    /// Identifier from the file name.
    pub file_id: Option<RecId>,
    /// Title text after the identifier.
    pub title: String,
    /// `Status`.
    pub status: Option<String>,
    /// `Date`.
    pub date: Option<String>,
    /// `Depends` entries.
    pub depends: Vec<RelEntry>,
    /// `Supersedes` entries.
    pub supersedes: Vec<RelEntry>,
    /// `Related` entries.
    pub related: Vec<RelEntry>,
    /// `Amends` entries (absent from the templates; older records carry them).
    pub amends: Vec<RelEntry>,
    /// `Within` entries that name a record.
    pub within: Vec<RelEntry>,
    /// `Within` entries that name a maintained document.
    pub within_docs: Vec<DocRef>,
    /// `Changes` entries.
    pub changes: Vec<DocRef>,
    /// Header fields as written.
    pub fields: Vec<Field>,
    /// Hyperlinks in the file that resolve to other record files: `(target, line)`.
    pub body_links: Vec<(RecId, usize)>,
}

impl Record {
    /// A header field by name.
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }
}

/// All records of a snapshot.
pub struct Records {
    /// Records in file order.
    pub list: Vec<Record>,
    by_id: BTreeMap<RecId, usize>,
}

/// Until when a record may carry `Amends`, read from the contributing document's
/// `Records` section (`:Amends: none | until <YYYY-MM-DD>`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AmendsCutoff {
    /// No contributing document, no `Records` section, or `none`: no record may
    /// carry `Amends`.
    None,
    /// Records dated on or before this date may carry it.
    Until(String),
}

/// Reads the cutoff from the contributing document of `snap`.
pub fn amends_cutoff(snap: &Snapshot) -> AmendsCutoff {
    let Some(doc) = snap
        .single(Family::Contributing)
        .and_then(|f| f.doc.as_ref())
    else {
        return AmendsCutoff::None;
    };
    let Some((idx, _)) = doc
        .headings_at(2)
        .find(|(_, h)| h.title.trim() == "Records")
    else {
        return AmendsCutoff::None;
    };
    let Some(field) = doc
        .body_fields(idx)
        .into_iter()
        .find(|f| f.name == "Amends")
    else {
        return AmendsCutoff::None;
    };
    match field.value.trim().strip_prefix("until ") {
        Some(date) if crate::util::is_iso_date(date.trim()) => {
            AmendsCutoff::Until(date.trim().to_string())
        }
        _ => AmendsCutoff::None,
    }
}

/// The marker that makes a `Supersedes` entry partial: `RFC-0104 (in part: ...)`.
pub const PARTIAL_MARKER: &str = "in part:";

fn parse_rel(field: &Field) -> Vec<RelEntry> {
    let rel = re(r"(?s)^(RFC|ADR)-(\d{4})\s*(?:\((.*)\))?$");
    let mut out = Vec::new();
    for e in split_entries(&field.value) {
        if let Some(c) = rel.captures(&e)
            && let Some(id) = RecId::parse(&format!("{}-{}", &c[1], &c[2]))
        {
            let raw = c.get(3).map(|m| m.as_str().trim().to_string());
            let (note, partial) = match raw {
                Some(n) if n.to_lowercase().starts_with(PARTIAL_MARKER) => {
                    (Some(n[PARTIAL_MARKER.len()..].trim().to_string()), true)
                }
                other => (other, false),
            };
            out.push(RelEntry {
                id,
                note,
                partial,
                line: field.start,
            });
        }
    }
    out
}

fn parse_docrefs(field: &Field) -> Vec<DocRef> {
    let dref = re(r"(?s)^(.+?)\s*\((.*)\)$");
    let mut out = Vec::new();
    for e in split_entries(&field.value) {
        if e == "none" || RecId::parse(e.split([' ', '(']).next().unwrap_or("")).is_some() {
            continue;
        }
        let (doc, sections) = match dref.captures(&e) {
            Some(c) => (
                c[1].trim().trim_matches('`').to_string(),
                split_entries(&c[2])
                    .into_iter()
                    .map(|s| s.trim().to_string())
                    .collect(),
            ),
            None => (e.trim().trim_matches('`').to_string(), Vec::new()),
        };
        out.push(DocRef {
            doc,
            sections,
            line: field.start,
        });
    }
    out
}

impl Records {
    /// Reads every RFC and ADR file of `snap`.
    pub fn build(snap: &Snapshot) -> Records {
        let title_re = re(r"^(RFC|ADR)-(\d{4}):\s*(.*)$");
        let name_re = re(r"^(rfc|adr)-(\d{4})(?:-[a-z0-9-]+)?\.rst$");
        let mut list = Vec::new();
        for (idx, f) in snap.files.iter().enumerate() {
            if !matches!(f.role, Role::Rfc | Role::Adr) {
                continue;
            }
            let Some(doc) = &f.doc else { continue };
            let fields = doc.header_fields();
            let (id, title) = match doc
                .title()
                .and_then(|t| title_re.captures(&t.title).map(|c| (c, t)))
            {
                Some((c, _)) => (
                    RecId::parse(&format!("{}-{}", &c[1], &c[2])),
                    c[3].to_string(),
                ),
                None => (
                    None,
                    doc.title().map(|t| t.title.clone()).unwrap_or_default(),
                ),
            };
            let file_id = name_re
                .captures(&f.name)
                .and_then(|c| RecId::parse(&format!("{}-{}", &c[1], &c[2])));
            let mut rec = Record {
                file: idx,
                id,
                file_id,
                title,
                status: None,
                date: None,
                depends: Vec::new(),
                supersedes: Vec::new(),
                related: Vec::new(),
                amends: Vec::new(),
                within: Vec::new(),
                within_docs: Vec::new(),
                changes: Vec::new(),
                fields: fields.clone(),
                body_links: Vec::new(),
            };
            for fld in &fields {
                match fld.name.as_str() {
                    "Status" => rec.status = Some(fld.value.trim().to_string()),
                    "Date" => rec.date = Some(fld.value.trim().to_string()),
                    "Depends" => rec.depends = parse_rel(fld),
                    "Supersedes" => rec.supersedes = parse_rel(fld),
                    "Related" => rec.related = parse_rel(fld),
                    "Amends" => rec.amends = parse_rel(fld),
                    "Within" => {
                        rec.within = parse_rel(fld);
                        rec.within_docs = parse_docrefs(fld);
                    }
                    "Changes" => rec.changes = parse_docrefs(fld),
                    _ => {}
                }
            }
            list.push(rec);
        }
        let mut by_id = BTreeMap::new();
        for (i, r) in list.iter().enumerate() {
            if let Some(id) = r.id.or(r.file_id) {
                by_id.entry(id).or_insert(i);
            }
        }
        let mut by_path: BTreeMap<PathBuf, RecId> = BTreeMap::new();
        for r in &list {
            if let Some(id) = r.id.or(r.file_id) {
                by_path.insert(snap.files[r.file].path.clone(), id);
            }
        }
        for r in &mut list {
            let f = &snap.files[r.file];
            let Some(doc) = &f.doc else { continue };
            let dir = f.path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
            for l in doc.links() {
                if is_external(&l.target) {
                    continue;
                }
                let (path, _) = split_anchor(&l.target);
                if path.is_empty() {
                    continue;
                }
                if let Some(target) = join_lexical(&dir, path)
                    && let Some(id) = by_path.get(&target)
                {
                    r.body_links.push((*id, l.line));
                }
            }
        }
        Records { list, by_id }
    }

    /// The record with identifier `id`.
    pub fn get(&self, id: RecId) -> Option<&Record> {
        self.by_id.get(&id).map(|i| &self.list[*i])
    }

    /// The identifier of `r` (title first, file name second).
    pub fn id_of(r: &Record) -> Option<RecId> {
        r.id.or(r.file_id)
    }

    /// Every link between existing records, from headers and bodies.
    pub fn edges(&self) -> Vec<Edge> {
        let mut out = Vec::new();
        for r in &self.list {
            let Some(from) = Records::id_of(r) else {
                continue;
            };
            let mut push = |list: &[RelEntry], kind| {
                for e in list {
                    out.push(Edge {
                        from,
                        to: e.id,
                        kind,
                        line: e.line,
                        note: e.note.clone(),
                    });
                }
            };
            push(&r.depends, EdgeKind::Depends);
            push(&r.supersedes, EdgeKind::Supersedes);
            push(&r.related, EdgeKind::Related);
            push(&r.amends, EdgeKind::Amends);
            push(&r.within, EdgeKind::Within);
            for (to, line) in &r.body_links {
                out.push(Edge {
                    from,
                    to: *to,
                    kind: EdgeKind::Link,
                    line: *line,
                    note: None,
                });
            }
        }
        out
    }

    /// Direct `Depends` targets of `id` that exist.
    pub fn depends_of(&self, id: RecId) -> Vec<RecId> {
        self.get(id)
            .map(|r| {
                r.depends
                    .iter()
                    .map(|e| e.id)
                    .filter(|t| self.by_id.contains_key(t))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The dependency closure of `id`: every record reachable through `Depends`,
    /// excluding `id` itself. Safe on cycles.
    pub fn closure(&self, id: RecId) -> BTreeSet<RecId> {
        let mut seen = BTreeSet::new();
        let mut stack = self.depends_of(id);
        while let Some(n) = stack.pop() {
            if seen.insert(n) {
                stack.extend(self.depends_of(n));
            }
        }
        seen.remove(&id);
        seen
    }

    /// Records that link to `id`, with how.
    pub fn incoming(&self, id: RecId) -> Vec<(RecId, EdgeKind)> {
        let mut out: Vec<(RecId, EdgeKind)> = self
            .edges()
            .into_iter()
            .filter(|e| e.to == id && e.from != id)
            .map(|e| (e.from, e.kind))
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Records whose `Supersedes` names `id`, each with the part it replaces when
    /// the entry is partial (`None` for a whole supersession).
    pub fn superseded_by(&self, id: RecId) -> Vec<(RecId, Option<String>)> {
        let mut out: Vec<(RecId, Option<String>)> = self
            .list
            .iter()
            .flat_map(|r| {
                let from = Records::id_of(r);
                r.supersedes
                    .iter()
                    .filter(move |e| e.id == id)
                    .filter_map(move |e| {
                        let part = if e.partial {
                            Some(e.note.clone().unwrap_or_default())
                        } else {
                            None
                        };
                        Some((from?, part))
                    })
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Records whose `Supersedes` names `id` as a whole; only these set the status.
    pub fn wholly_superseded_by(&self, id: RecId) -> Vec<RecId> {
        self.superseded_by(id)
            .into_iter()
            .filter(|(_, part)| part.is_none())
            .map(|(from, _)| from)
            .collect()
    }

    /// Labels for the records `superseded_by` or `amended_by` returns: the note in
    /// parentheses, prefixed by `marker` when one is given (`RFC-0120 (in part:
    /// rights)`, `RFC-0121 (the names)`), or the bare identifier.
    pub fn labels(list: &[(RecId, Option<String>)], marker: Option<&str>) -> Vec<String> {
        list.iter()
            .map(|(id, note)| match (note, marker) {
                (Some(n), Some(m)) if !n.is_empty() => format!("{id} ({m} {n})"),
                (Some(n), None) if !n.is_empty() => format!("{id} ({n})"),
                (Some(_), Some(m)) => format!("{id} ({})", m.trim_end_matches(':')),
                _ => id.to_string(),
            })
            .collect()
    }

    /// Records whose `Amends` names `id`, each with what it changed.
    pub fn amended_by(&self, id: RecId) -> Vec<(RecId, Option<String>)> {
        let mut out: Vec<(RecId, Option<String>)> = self
            .list
            .iter()
            .flat_map(|r| {
                let from = Records::id_of(r);
                r.amends
                    .iter()
                    .filter(move |e| e.id == id)
                    .filter_map(move |e| Some((from?, e.note.clone())))
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Strongly connected groups of two or more records over every link.
    pub fn cycles(&self) -> Vec<Vec<RecId>> {
        let mut adj: BTreeMap<RecId, Vec<RecId>> = BTreeMap::new();
        for e in self.edges() {
            if e.from != e.to && self.by_id.contains_key(&e.to) {
                adj.entry(e.from).or_default().push(e.to);
            }
        }
        struct Tarjan<'a> {
            adj: &'a BTreeMap<RecId, Vec<RecId>>,
            index: BTreeMap<RecId, usize>,
            low: BTreeMap<RecId, usize>,
            on: BTreeSet<RecId>,
            stack: Vec<RecId>,
            next: usize,
            out: Vec<Vec<RecId>>,
        }
        impl Tarjan<'_> {
            fn visit(&mut self, v: RecId) {
                self.index.insert(v, self.next);
                self.low.insert(v, self.next);
                self.next += 1;
                self.stack.push(v);
                self.on.insert(v);
                let succ = self.adj.get(&v).cloned().unwrap_or_default();
                for w in succ {
                    if !self.index.contains_key(&w) {
                        self.visit(w);
                        let lw = self.low[&w];
                        let lv = self.low[&v];
                        self.low.insert(v, lv.min(lw));
                    } else if self.on.contains(&w) {
                        let iw = self.index[&w];
                        let lv = self.low[&v];
                        self.low.insert(v, lv.min(iw));
                    }
                }
                if self.low[&v] == self.index[&v] {
                    let mut comp = Vec::new();
                    while let Some(w) = self.stack.pop() {
                        self.on.remove(&w);
                        comp.push(w);
                        if w == v {
                            break;
                        }
                    }
                    if comp.len() > 1 {
                        comp.sort();
                        self.out.push(comp);
                    }
                }
            }
        }
        let mut t = Tarjan {
            adj: &adj,
            index: BTreeMap::new(),
            low: BTreeMap::new(),
            on: BTreeSet::new(),
            stack: Vec::new(),
            next: 0,
            out: Vec::new(),
        };
        let nodes: Vec<RecId> = self.by_id.keys().copied().collect();
        for n in nodes {
            if !t.index.contains_key(&n) {
                t.visit(n);
            }
        }
        t.out.sort();
        t.out
    }

    /// Orders `ids` so that a record comes after every record of its dependency closure
    /// that is also in `ids`. Ties (and cycles) break by date, kind, number.
    pub fn dependency_order(&self, ids: &[RecId]) -> Vec<RecId> {
        let set: BTreeSet<RecId> = ids.iter().copied().collect();
        let key = |id: &RecId| {
            (
                self.get(*id)
                    .and_then(|r| r.date.clone())
                    .unwrap_or_default(),
                *id,
            )
        };
        let mut remaining: BTreeSet<RecId> = set.clone();
        let mut out = Vec::new();
        while !remaining.is_empty() {
            let ready: Vec<RecId> = remaining
                .iter()
                .copied()
                .filter(|id| {
                    !self
                        .closure(*id)
                        .iter()
                        .any(|d| remaining.contains(d) && d != id)
                })
                .collect();
            let pick = if ready.is_empty() {
                remaining.iter().copied().min_by_key(key)
            } else {
                ready.into_iter().min_by_key(key)
            };
            match pick {
                Some(p) => {
                    remaining.remove(&p);
                    out.push(p);
                }
                None => break,
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rec_id_round_trip() {
        let id = RecId::parse("rfc-0004").expect("id");
        assert_eq!(id.to_string(), "RFC-0004");
        assert!(RecId::parse("RFC-4").is_none());
        assert!(RecId::parse("XYZ-0001").is_none());
    }
}
