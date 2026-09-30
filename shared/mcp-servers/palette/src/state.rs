//! The state document read into decisions, open questions and discrepancies.
//!
//! Owns locating the three sections and their bullet entries (`- D-<n> ...` with
//! indented continuation lines). Does not edit the file and does not report findings.
//! Entry points: [`State::parse`], [`State::strict`].

use crate::errors::{PalError, Res};
use crate::rst::{Doc, Field, Kind};
use crate::schema::schemas;
use crate::util::re;

/// Which of the three sections an entry is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    /// `D-<n>`.
    Decision,
    /// `Q-<n>`.
    Question,
    /// `X-<n>`.
    Discrepancy,
}

impl EntryKind {
    /// The id prefix letter and dash.
    pub fn prefix(self) -> &'static str {
        match self {
            EntryKind::Decision => "D-",
            EntryKind::Question => "Q-",
            EntryKind::Discrepancy => "X-",
        }
    }

    /// Index of the section in the template.
    pub fn section_index(self) -> usize {
        match self {
            EntryKind::Decision => 0,
            EntryKind::Question => 1,
            EntryKind::Discrepancy => 2,
        }
    }

    /// The kind whose id starts `s` (`D-3`, `Q-1`, `X-2`).
    pub fn of_id(s: &str) -> Option<EntryKind> {
        match s.get(..2)? {
            "D-" => Some(EntryKind::Decision),
            "Q-" => Some(EntryKind::Question),
            "X-" => Some(EntryKind::Discrepancy),
            _ => None,
        }
    }
}

/// One bullet entry.
#[derive(Clone, Debug)]
pub struct Entry {
    /// Number after the prefix, when the bullet has one.
    pub number: Option<u32>,
    /// Every number of this section's kind the entry names (`D-1, D-2` and `D-4 to D-7`
    /// name 1, 2, 4 and 7).
    pub mentioned: Vec<u32>,
    /// First line (0-based).
    pub start: usize,
    /// Exclusive end line.
    pub end: usize,
    /// Text of the entry with continuation lines joined by single spaces.
    pub text: String,
}

impl Entry {
    /// Whether the entry is a graduated pointer (`D-1, D-2 Graduated to <link>.`; the
    /// older `... Written: <link>.` form counts too).
    pub fn is_pointer(&self) -> bool {
        self.text.contains("Graduated to") || self.text.contains("Written:")
    }
}

/// One section of the state document.
#[derive(Clone, Debug)]
pub struct Section {
    /// Heading index in `Doc::headings`.
    pub heading: usize,
    /// First body line.
    pub body_start: usize,
    /// Exclusive end of the body.
    pub body_end: usize,
    /// Entries in file order.
    pub entries: Vec<Entry>,
    /// The line holding `None.` when the section says so.
    pub none_line: Option<usize>,
}

/// A parsed state document.
#[derive(Clone, Debug, Default)]
pub struct State {
    /// The `Updated` header field.
    pub updated: Option<Field>,
    /// Sections in template order: decisions, questions, discrepancies.
    pub sections: [Option<Section>; 3],
}

impl State {
    /// Reads the state document leniently.
    pub fn parse(doc: &Doc) -> State {
        let mut st = State::default();
        let Some(schema) = schemas().get("state") else {
            return st;
        };
        st.updated = doc
            .header_fields()
            .into_iter()
            .find(|f| f.name == "Updated");
        let id_re = re(r"^-\s+[A-Z]+-(\d+)\b");
        for (k, spec) in schema.sections.iter().take(3).enumerate() {
            let Some(title) = spec.title.literal() else {
                continue;
            };
            let prefix = spec.id_prefix.clone().unwrap_or_default();
            let mention_re = re(&format!(r"\b{}(\d+)\b", regex::escape(&prefix)));
            let Some((idx, h)) = doc.headings_at(2).find(|(_, h)| h.title == title) else {
                continue;
            };
            let mut entries: Vec<Entry> = Vec::new();
            let mut none_line = None;
            let mut i = h.body_start;
            while i < h.body_end {
                let text = doc.src.text(i);
                if doc.kinds[i] == Kind::Text && text.starts_with("- ") {
                    let start = i;
                    let mut joined = text.trim().to_string();
                    let mut j = i + 1;
                    while j < h.body_end
                        && doc.kinds[j] == Kind::Text
                        && doc.src.text(j).starts_with([' ', '\t'])
                        && !doc.src.text(j).trim().is_empty()
                    {
                        joined.push(' ');
                        joined.push_str(doc.src.text(j).trim());
                        j += 1;
                    }
                    entries.push(Entry {
                        number: id_re.captures(text).and_then(|c| c[1].parse().ok()),
                        mentioned: mention_re
                            .captures_iter(&joined)
                            .filter_map(|c| c[1].parse().ok())
                            .collect(),
                        start,
                        end: j,
                        text: joined,
                    });
                    i = j;
                    continue;
                }
                if text.trim() == "None." {
                    none_line = Some(i);
                }
                i += 1;
            }
            st.sections[k] = Some(Section {
                heading: idx,
                body_start: h.body_start,
                body_end: h.body_end,
                entries,
                none_line,
            });
        }
        st
    }

    /// Reads the state document for a write tool; a missing section is a `parse_error`.
    pub fn strict(doc: &Doc, rel: &str) -> Res<State> {
        let st = State::parse(doc);
        if doc.title().is_none() {
            return Err(PalError::parse(rel, 1, "the state document has no title"));
        }
        if st.updated.is_none() {
            return Err(PalError::parse(
                rel,
                1,
                "the state document has no `Updated` field",
            ));
        }
        for (k, s) in st.sections.iter().enumerate() {
            if s.is_none() {
                let name = schemas()
                    .get("state")
                    .and_then(|sc| sc.sections.get(k))
                    .and_then(|x| x.title.literal())
                    .unwrap_or("?");
                return Err(PalError::parse(
                    rel,
                    1,
                    format!("the state document has no `{name}` section"),
                ));
            }
        }
        Ok(st)
    }

    /// The section for `kind`.
    pub fn section(&self, kind: EntryKind) -> Option<&Section> {
        self.sections[kind.section_index()].as_ref()
    }

    /// Highest number `kind` has used, counting every id an entry names (a pointer such
    /// as `D-4 to D-7` still holds those numbers).
    pub fn max_number(&self, kind: EntryKind) -> u32 {
        self.section(kind)
            .map(|s| {
                s.entries
                    .iter()
                    .flat_map(|e| e.mentioned.iter().copied().chain(e.number))
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0)
    }

    /// The pointer entry that names number `n` without starting with it (`D-2` inside
    /// `D-1, D-2 Graduated to ...`).
    pub fn pointer_naming(&self, kind: EntryKind, n: u32) -> Option<&Entry> {
        self.section(kind)?
            .entries
            .iter()
            .find(|e| e.is_pointer() && e.mentioned.contains(&n))
    }

    /// The entry with the given kind and number.
    pub fn entry(&self, kind: EntryKind, n: u32) -> Option<&Entry> {
        self.section(kind)?
            .entries
            .iter()
            .find(|e| e.number == Some(n))
    }
}
