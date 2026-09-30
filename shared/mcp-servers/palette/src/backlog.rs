//! The backlog document read into items and phase entries.
//!
//! Owns locating the `Phases` and `Items` sections, item headings (`B-<n> <title>`),
//! item field blocks and bodies, and phase entries (`:phase-<N>: <link> — active`).
//! Does not edit the file (the write tools do) and does not report findings.
//! Entry points: [`Backlog::parse`], [`Backlog::strict`].

use crate::errors::{PalError, Res};
use crate::rst::{Doc, Field};
use crate::schema::schemas;
use crate::util::re;

/// One backlog item.
#[derive(Clone, Debug)]
pub struct Item {
    /// The number in `B-<n>`.
    pub id: u32,
    /// Index of the item's heading in `Doc::headings`.
    pub heading: usize,
    /// Title after the identifier.
    pub title: String,
    /// 0-based heading line.
    pub line: usize,
    /// Exclusive end of the item.
    pub end: usize,
    /// Fields of the item.
    pub fields: Vec<Field>,
    /// Exclusive end of the field block (the heading body start when there are no fields).
    pub fields_end: usize,
    /// Body lines `[start, end)` without surrounding blank lines.
    pub body: Option<(usize, usize)>,
}

impl Item {
    /// A field's value by name.
    pub fn value(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|f| f.name == name)
            .map(|f| f.value.as_str())
    }

    /// A field by name.
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// The identifier text `B-<n>`.
    pub fn label(&self) -> String {
        format!("B-{}", self.id)
    }
}

/// One entry of the `Phases` section.
#[derive(Clone, Debug)]
pub struct PhaseEntry {
    /// The number in `phase-<N>`.
    pub number: u32,
    /// The field.
    pub field: Field,
    /// Link target when the value starts with a hyperlink.
    pub link: Option<String>,
    /// `active` or `closed` when the value ends with ` — <state>`.
    pub state: Option<String>,
}

/// A parsed backlog.
#[derive(Clone, Debug, Default)]
pub struct Backlog {
    /// Items in file order.
    pub items: Vec<Item>,
    /// Phase entries in file order.
    pub phases: Vec<PhaseEntry>,
    /// Heading index of `Phases`.
    pub phases_heading: Option<usize>,
    /// Heading index of `Items`.
    pub items_heading: Option<usize>,
    /// Structural problems `(0-based line, message)` that stop a write tool.
    pub problems: Vec<(usize, String)>,
}

impl Backlog {
    /// Reads the backlog leniently: whatever parses is returned and the rest is listed
    /// in `problems`.
    pub fn parse(doc: &Doc) -> Backlog {
        let mut b = Backlog::default();
        let schema = schemas().get("backlog");
        let titles: Vec<String> = schema
            .map(|s| {
                s.sections
                    .iter()
                    .filter_map(|x| x.title.literal().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let (phases_title, items_title) = (
            titles.first().cloned().unwrap_or_default(),
            titles.get(1).cloned().unwrap_or_default(),
        );
        for (i, h) in doc.headings_at(2) {
            if h.title == phases_title {
                b.phases_heading = Some(i);
            } else if h.title == items_title {
                b.items_heading = Some(i);
            }
        }
        if doc.title().is_none() {
            b.problems.push((0, "the backlog has no title".to_string()));
        }
        let phase_re = re(r"^phase-(\d+)$");
        let link_re = re(r"^`[^`<]*<([^>`]+)>`_+");
        match b.phases_heading {
            None => b
                .problems
                .push((0, format!("the backlog has no `{phases_title}` section"))),
            Some(pi) => {
                for f in doc.body_fields(pi) {
                    let Some(c) = phase_re.captures(&f.name) else {
                        b.problems.push((
                            f.start,
                            format!("`{}` is not a phase entry (`phase-<N>`)", f.name),
                        ));
                        continue;
                    };
                    let number = c[1].parse().unwrap_or(0);
                    let link = link_re.captures(&f.value).map(|c| c[1].trim().to_string());
                    let state = f
                        .value
                        .rsplit_once(" — ")
                        .map(|(_, s)| s.trim().to_string())
                        .filter(|s| s == "active" || s == "closed");
                    b.phases.push(PhaseEntry {
                        number,
                        field: f,
                        link,
                        state,
                    });
                }
            }
        }
        let item_re = re(r"^B-(\d+)\s+(.+)$");
        match b.items_heading {
            None => b
                .problems
                .push((0, format!("the backlog has no `{items_title}` section"))),
            Some(ii) => {
                for c in doc.children(ii) {
                    let h = &doc.headings[c];
                    let Some(m) = item_re.captures(&h.title) else {
                        b.problems.push((
                            h.line,
                            format!("`{}` is not an item heading (`B-<n> <title>`)", h.title),
                        ));
                        continue;
                    };
                    let fields = doc.body_fields(c);
                    let fields_end = fields.last().map(|f| f.end).unwrap_or(h.body_start);
                    let mut first = None;
                    let mut last = None;
                    for l in fields_end..h.end {
                        if !doc.src.text(l).trim().is_empty() {
                            first.get_or_insert(l);
                            last = Some(l);
                        }
                    }
                    let body = first.zip(last).map(|(s, e)| (s, e + 1));
                    b.items.push(Item {
                        id: m[1].parse().unwrap_or(0),
                        heading: c,
                        title: m[2].trim().to_string(),
                        line: h.line,
                        end: h.end,
                        fields,
                        fields_end,
                        body,
                    });
                }
            }
        }
        b
    }

    /// Reads the backlog for a write tool; any structural problem is a `parse_error`.
    pub fn strict(doc: &Doc, rel: &str) -> Res<Backlog> {
        let b = Backlog::parse(doc);
        if let Some((line, msg)) = b.problems.first() {
            return Err(PalError::parse(rel, line + 1, msg.clone()));
        }
        for it in &b.items {
            if it.value("Status").is_none() {
                return Err(PalError::parse(
                    rel,
                    it.line + 1,
                    format!("item {} has no Status field", it.label()),
                ));
            }
        }
        Ok(b)
    }

    /// The item numbered `id`.
    pub fn item(&self, id: u32) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    /// The phase entry numbered `n`.
    pub fn phase(&self, n: u32) -> Option<&PhaseEntry> {
        self.phases.iter().find(|p| p.number == n)
    }

    /// Every phase whose entry says `active`; several may be active at once.
    pub fn active_phases(&self) -> Vec<&PhaseEntry> {
        self.phases
            .iter()
            .filter(|p| p.state.as_deref() == Some("active"))
            .collect()
    }
}

/// Parses `B-<n>` (or a bare number) into the number.
pub fn parse_item_ref(s: &str) -> Option<u32> {
    let t = s.trim();
    let t = t
        .strip_prefix("B-")
        .or_else(|| t.strip_prefix("b-"))
        .unwrap_or(t);
    t.parse().ok()
}

/// Splits `in-phase-<N>` into `N`.
pub fn in_phase(status: &str) -> Option<u32> {
    status
        .strip_prefix("in-phase-")
        .and_then(|n| n.parse().ok())
}
