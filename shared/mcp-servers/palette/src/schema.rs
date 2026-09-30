//! Document schemas derived from the embedded palette templates.
//!
//! Owns the template texts (embedded at build time), the title patterns, header and
//! section field specs, allowed-value grammar and required section lists derived from
//! them. Does not read project files and does not report findings; `lint` applies
//! these schemas. Entry points: [`schemas`], [`template_text`], [`ValueSpec`], [`Pattern`].

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::rst::Doc;
use crate::text::Source;
use crate::util::re;

/// Template names and their embedded text.
pub const TEMPLATES: &[(&str, &str)] = &[
    (
        "adr",
        include_str!("../../../workflows/palette/templates/adr.rst"),
    ),
    (
        "backlog",
        include_str!("../../../workflows/palette/templates/backlog.rst"),
    ),
    (
        "changeset",
        include_str!("../../../workflows/palette/templates/changeset.rst"),
    ),
    (
        "contributing",
        include_str!("../../../workflows/palette/templates/contributing.rst"),
    ),
    (
        "deliverable",
        include_str!("../../../workflows/palette/templates/deliverable.rst"),
    ),
    (
        "design",
        include_str!("../../../workflows/palette/templates/design.rst"),
    ),
    (
        "glossary",
        include_str!("../../../workflows/palette/templates/glossary.rst"),
    ),
    (
        "house-style",
        include_str!("../../../workflows/palette/templates/house-style.rst"),
    ),
    (
        "layout",
        include_str!("../../../workflows/palette/templates/layout.rst"),
    ),
    (
        "phase",
        include_str!("../../../workflows/palette/templates/phase.rst"),
    ),
    (
        "principles",
        include_str!("../../../workflows/palette/templates/principles.rst"),
    ),
    (
        "rfc",
        include_str!("../../../workflows/palette/templates/rfc.rst"),
    ),
    (
        "rubrics",
        include_str!("../../../workflows/palette/templates/rubrics.rst"),
    ),
    (
        "spec",
        include_str!("../../../workflows/palette/templates/spec.rst"),
    ),
    (
        "state",
        include_str!("../../../workflows/palette/templates/state.rst"),
    ),
];

/// Header fields whose value is a list of entries separated by `;`.
pub const REPEATING_FIELDS: [&str; 5] = ["Depends", "Supersedes", "Related", "Changes", "Revised"];

/// The template text named `name`.
pub fn template_text(name: &str) -> Option<&'static str> {
    TEMPLATES.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
}

/// One piece of a pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    /// Literal text.
    Lit(String),
    /// Free text placeholder `<...>`.
    Free(String),
    /// `<NNNN>`: a four-digit number.
    Num4,
    /// `<n>` or `<N>`: a number.
    Num,
    /// `<YYYY-MM-DD>`: an ISO date.
    Date,
    /// A placeholder repeated with `; ` in the template: one or more.
    List(Box<Part>),
    /// One of several literal tokens.
    OneOf(Vec<String>),
}

fn part_regex(p: &Part) -> String {
    match p {
        Part::Lit(s) => regex::escape(s),
        Part::Free(_) => ".+?".to_string(),
        Part::Num4 => r"\d{4}".to_string(),
        Part::Num => r"\d+".to_string(),
        Part::Date => r"\d{4}-\d{2}-\d{2}".to_string(),
        Part::List(inner) => {
            let r = part_regex(inner);
            format!(r"{r}(?:;\s*{r})*")
        }
        Part::OneOf(v) => {
            let alts: Vec<String> = v.iter().map(|t| regex::escape(t)).collect();
            format!("(?:{})", alts.join("|"))
        }
    }
}

fn parse_parts(text: &str) -> Vec<Part> {
    let mut parts: Vec<Part> = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        match rest.find('<') {
            Some(i) => match rest[i..].find('>') {
                Some(j) => {
                    if i > 0 {
                        parts.push(Part::Lit(rest[..i].to_string()));
                    }
                    let inner = &rest[i + 1..i + j];
                    parts.push(match inner {
                        "NNNN" => Part::Num4,
                        "n" | "N" => Part::Num,
                        "YYYY-MM-DD" => Part::Date,
                        other => Part::Free(other.to_string()),
                    });
                    rest = &rest[i + j + 1..];
                }
                None => {
                    parts.push(Part::Lit(rest.to_string()));
                    rest = "";
                }
            },
            None => {
                parts.push(Part::Lit(rest.to_string()));
                rest = "";
            }
        }
    }
    // `<x>; <x>` in a template means one or more x.
    let mut out: Vec<Part> = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        if let (Some(Part::Free(a)), Some(Part::Lit(sep)), Some(Part::Free(b))) =
            (parts.get(i), parts.get(i + 1), parts.get(i + 2))
            && a == b
            && sep.trim() == ";"
        {
            out.push(Part::List(Box::new(Part::Free(a.clone()))));
            i += 3;
            continue;
        }
        out.push(parts[i].clone());
        i += 1;
    }
    out
}

/// A text pattern with literal and placeholder parts, used for titles and field names.
#[derive(Clone, Debug)]
pub struct Pattern {
    /// The template text this pattern came from.
    pub raw: String,
    /// The parts in order.
    pub parts: Vec<Part>,
    re: Regex,
}

impl Pattern {
    /// Parses a pattern such as `RFC-<NNNN>: <Title>`.
    pub fn parse(raw: &str) -> Pattern {
        let parts = parse_parts(raw.trim());
        let body: String = parts
            .iter()
            .map(|p| format!("({})", part_regex(p)))
            .collect();
        Pattern {
            raw: raw.trim().to_string(),
            parts,
            re: re(&format!("^{body}$")),
        }
    }

    /// Whether `s` matches.
    pub fn is_match(&self, s: &str) -> bool {
        self.re.is_match(s)
    }

    /// The text captured by each placeholder part (numbers and free text), in order.
    pub fn captures(&self, s: &str) -> Option<Vec<String>> {
        let c = self.re.captures(s)?;
        let mut out = Vec::new();
        for (i, p) in self.parts.iter().enumerate() {
            if !matches!(p, Part::Lit(_)) {
                out.push(
                    c.get(i + 1)
                        .map(|m| m.as_str().to_string())
                        .unwrap_or_default(),
                );
            }
        }
        Some(out)
    }

    /// Whether the pattern has any placeholder.
    pub fn has_placeholder(&self) -> bool {
        self.parts.iter().any(|p| !matches!(p, Part::Lit(_)))
    }

    /// The literal text when the pattern has no placeholder.
    pub fn literal(&self) -> Option<&str> {
        if self.has_placeholder() {
            None
        } else {
            self.parts.first().and_then(|p| match p {
                Part::Lit(s) => Some(s.as_str()),
                _ => None,
            })
        }
    }
}

/// The allowed values of a header field, derived from the template value.
#[derive(Clone, Debug)]
pub struct ValueSpec {
    /// The template value text.
    pub raw: String,
    alts: Vec<Vec<Part>>,
    res: Vec<Regex>,
}

impl ValueSpec {
    /// Derives the grammar from a template value such as
    /// `Draft | Proposed`, `a | b — <text>` or `none | RFC-<NNNN> (<use>)`.
    pub fn parse(raw: &str) -> ValueSpec {
        let raw = raw.trim();
        let texts: Vec<&str> = raw.split(" | ").map(str::trim).collect();
        let is_token = |s: &str| !s.is_empty() && !s.contains('<');
        let mut alts: Vec<Vec<Part>> = Vec::new();

        let after_dash_tokens = texts.len() > 1
            && texts[0]
                .split_once(" — ")
                .is_some_and(|(_, after)| is_token(after))
            && texts[1..].iter().all(|t| is_token(t));
        let last_dash_free = texts.last().and_then(|last| {
            last.split_once(" — ")
                .filter(|(prefix, after)| is_token(prefix) && after.starts_with('<'))
        });

        if after_dash_tokens {
            // `<link> — active | closed`: the tokens follow the dash.
            let (prefix, first) = texts[0].split_once(" — ").unwrap_or((texts[0], ""));
            let mut toks = vec![first.to_string()];
            toks.extend(texts[1..].iter().map(|t| t.to_string()));
            let mut parts = parse_parts(prefix);
            parts.push(Part::Lit(" — ".to_string()));
            parts.push(Part::OneOf(toks));
            alts.push(parts);
        } else if let Some((prefix, after)) = last_dash_free
            && texts[..texts.len() - 1].iter().all(|t| is_token(t))
        {
            // `a | b — <text>`: the suffix attaches to every token.
            let mut toks: Vec<String> = texts[..texts.len() - 1]
                .iter()
                .map(|t| t.to_string())
                .collect();
            toks.push(prefix.to_string());
            let mut parts = vec![Part::OneOf(toks), Part::Lit(" — ".to_string())];
            parts.extend(parse_parts(after));
            alts.push(parts);
        } else {
            for t in &texts {
                alts.push(parse_parts(t));
            }
        }
        let res = alts
            .iter()
            .map(|a| {
                let body: String = a.iter().map(part_regex).collect();
                re(&format!("^{body}$"))
            })
            .collect();
        ValueSpec {
            raw: raw.to_string(),
            alts,
            res,
        }
    }

    /// Index of the first alternative that `v` matches.
    pub fn match_index(&self, v: &str) -> Option<usize> {
        self.res.iter().position(|r| r.is_match(v))
    }

    /// Whether alternative `i` has no placeholder (for example `none`).
    pub fn is_bare(&self, i: usize) -> bool {
        self.alts[i].iter().all(|p| matches!(p, Part::Lit(_)))
    }

    /// Checks a value. `repeating` fields take `;`-separated entries.
    pub fn check(&self, value: &str, repeating: bool) -> Result<(), String> {
        let value = value.trim();
        if value.is_empty() {
            return Err(format!("the value is empty; allowed: {}", self.raw));
        }
        if !repeating {
            return match self.match_index(value) {
                Some(_) => Ok(()),
                None => Err(format!("`{value}` does not match: {}", self.raw)),
            };
        }
        let entries = split_entries(value);
        for e in &entries {
            match self.match_index(e) {
                None => return Err(format!("entry `{e}` does not match: {}", self.raw)),
                Some(i) if self.is_bare(i) && entries.len() > 1 => {
                    return Err(format!("`{e}` must be the only entry"));
                }
                Some(_) => {}
            }
        }
        Ok(())
    }
}

/// Splits a repeating field value into entries at `;` outside parentheses and
/// inline literals.
pub fn split_entries(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    let mut in_lit = false;
    let chars: Vec<char> = value.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '`' => {
                if chars.get(i + 1) == Some(&'`') {
                    in_lit = !in_lit;
                    cur.push_str("``");
                    i += 2;
                    continue;
                }
                cur.push(c);
            }
            '(' if !in_lit => {
                depth += 1;
                cur.push(c);
            }
            ')' if !in_lit => {
                depth -= 1;
                cur.push(c);
            }
            ';' if depth <= 0 && !in_lit => {
                let t = cur.trim();
                if !t.is_empty() {
                    out.push(t.to_string());
                }
                cur.clear();
            }
            _ => cur.push(c),
        }
        i += 1;
    }
    let t = cur.trim();
    if !t.is_empty() {
        out.push(t.to_string());
    }
    out
}

/// A required or repeatable field.
#[derive(Clone, Debug)]
pub struct FieldSpec {
    /// Field name pattern (literal for header fields, `phase-<N>` for phase entries).
    pub name: Pattern,
    /// Allowed values.
    pub value: ValueSpec,
    /// Whether the value is a `;`-separated list.
    pub repeating: bool,
}

impl FieldSpec {
    /// The literal field name, when the name has no placeholder.
    pub fn literal_name(&self) -> Option<&str> {
        self.name.literal()
    }
}

/// A section a template shows.
#[derive(Clone, Debug)]
pub struct SectionSpec {
    /// Title pattern.
    pub title: Pattern,
    /// Whether the section can appear any number of times (its title has a placeholder).
    pub repeating: bool,
    /// Fields shown at the start of the section body.
    pub fields: Vec<FieldSpec>,
    /// Subsections shown under it.
    pub subs: Vec<SectionSpec>,
    /// Prefix of the entry ids its bullets carry (`D-` in `- D-<n> ...`).
    pub id_prefix: Option<String>,
}

/// Everything derived from one template.
#[derive(Clone, Debug)]
pub struct FamilySchema {
    /// Level-1 title pattern.
    pub title: Pattern,
    /// Required header fields in order.
    pub header: Vec<FieldSpec>,
    /// Second-level sections in order.
    pub sections: Vec<SectionSpec>,
}

fn field_spec(name: &str, value: &str) -> FieldSpec {
    FieldSpec {
        name: Pattern::parse(name),
        value: ValueSpec::parse(value),
        repeating: REPEATING_FIELDS.contains(&name),
    }
}

static BULLET_ID_RE: LazyLock<Regex> = LazyLock::new(|| re(r"^-\s+([A-Za-z]+-)<n>"));

fn section_spec(doc: &Doc, idx: usize) -> SectionSpec {
    let h = &doc.headings[idx];
    let title = Pattern::parse(&h.title);
    let fields = doc
        .body_fields(idx)
        .iter()
        .map(|f| field_spec(&f.name, &f.value))
        .collect();
    let subs = doc
        .children(idx)
        .into_iter()
        .map(|c| section_spec(doc, c))
        .collect();
    let mut id_prefix = None;
    for i in h.body_start..h.body_end {
        if let Some(c) = BULLET_ID_RE.captures(doc.src.text(i)) {
            id_prefix = Some(c[1].to_string());
            break;
        }
    }
    SectionSpec {
        repeating: title.has_placeholder(),
        title,
        fields,
        subs,
        id_prefix,
    }
}

fn derive(text: &str) -> Option<FamilySchema> {
    let doc = Doc::parse(Source::from_text(text));
    let t = doc.title()?;
    let header = doc
        .header_fields()
        .iter()
        .map(|f| field_spec(&f.name, &f.value))
        .collect();
    let sections = doc
        .headings_at(2)
        .map(|(i, _)| section_spec(&doc, i))
        .collect();
    Some(FamilySchema {
        title: Pattern::parse(&t.title),
        header,
        sections,
    })
}

/// Schemas of every template that has a fixed structure, keyed by template name.
pub struct Schemas {
    by_name: BTreeMap<&'static str, FamilySchema>,
}

impl Schemas {
    /// The schema of template `name` (`rfc`, `adr`, `backlog`, ...).
    pub fn get(&self, name: &str) -> Option<&FamilySchema> {
        self.by_name.get(name)
    }
}

static SCHEMAS: LazyLock<Schemas> = LazyLock::new(|| {
    let mut by_name = BTreeMap::new();
    for (name, text) in TEMPLATES {
        if matches!(*name, "changeset" | "house-style" | "rubrics") {
            continue;
        }
        if let Some(s) = derive(text) {
            by_name.insert(*name, s);
        }
    }
    Schemas { by_name }
});

/// The schemas derived from the embedded templates.
pub fn schemas() -> &'static Schemas {
    &SCHEMAS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_yields_a_schema_except_free_form_ones() {
        for name in [
            "adr",
            "backlog",
            "contributing",
            "deliverable",
            "design",
            "glossary",
            "layout",
            "phase",
            "principles",
            "rfc",
            "spec",
            "state",
        ] {
            let s = schemas()
                .get(name)
                .unwrap_or_else(|| panic!("no schema for {name}"));
            assert!(!s.sections.is_empty() || !s.header.is_empty(), "{name}");
        }
    }

    #[test]
    fn rfc_schema_shape() {
        let s = schemas().get("rfc").expect("rfc");
        assert!(s.title.is_match("RFC-0004: palette MCP server"));
        assert!(!s.title.is_match("RFC-4: x"));
        let names: Vec<&str> = s.header.iter().filter_map(|f| f.literal_name()).collect();
        assert_eq!(names[0], "Status");
        assert_eq!(names.len(), 15);
        assert_eq!(s.sections.len(), 12);
        assert_eq!(s.sections[0].title.literal(), Some("Summary"));
    }

    #[test]
    fn implementation_suffix_attaches_to_every_token() {
        let v = ValueSpec::parse(
            "not-started | partial | complete | not-applicable — <scope in one line>",
        );
        assert!(v.check("not-started — the server", false).is_ok());
        assert!(v.check("complete — all of it", false).is_ok());
        assert!(v.check("not-started", false).is_err());
        assert!(v.check("started — x", false).is_err());
    }

    #[test]
    fn revised_none_stands_bare() {
        let v = ValueSpec::parse("none | <YYYY-MM-DD> — <what changed, in one line>");
        assert!(v.check("none", true).is_ok());
        assert!(v.check("2026-09-30 — fixed a typo", true).is_ok());
        assert!(v.check("2026-09-30 — a; 2026-10-01 — b", true).is_ok());
        assert!(v.check("yesterday — a", true).is_err());
        assert!(v.check("none; 2026-09-30 — a", true).is_err());
    }

    #[test]
    fn tokens_after_dash() {
        let v = ValueSpec::parse("<link to the phase file> — active | closed");
        assert!(v.check("`p <phase-1/phase.rst>`_ — active", false).is_ok());
        assert!(v.check("Phase 1 — closed", false).is_ok());
        assert!(v.check("Phase 1 — open", false).is_err());
    }

    #[test]
    fn changes_entry_takes_one_or_more_sections() {
        let v = ValueSpec::parse("none | <maintained document path> (<section>; <section>)");
        assert!(v.check("spec/x.rst (created)", true).is_ok());
        assert!(
            v.check("spec/x.rst (A; B / C); design/y.rst (D)", true)
                .is_ok()
        );
        assert!(v.check("spec/x.rst", true).is_err());
    }

    #[test]
    fn depends_entries() {
        let v = ValueSpec::parse("none | RFC-<NNNN> (<the contract used>)");
        assert!(v.check("RFC-0003 (rules; more, text)", true).is_ok());
        assert!(v.check("RFC-0003 (a); RFC-0002 (b)", true).is_ok());
        assert!(v.check("RFC-3 (a)", true).is_err());
        assert!(v.check("none", true).is_ok());
    }

    #[test]
    fn state_sections_carry_entry_prefixes() {
        let s = schemas().get("state").expect("state");
        let prefixes: Vec<_> = s.sections.iter().map(|x| x.id_prefix.clone()).collect();
        assert_eq!(
            prefixes,
            vec![Some("D-".into()), Some("Q-".into()), Some("X-".into())]
        );
    }

    #[test]
    fn backlog_items_and_phase_entries() {
        let s = schemas().get("backlog").expect("backlog");
        assert_eq!(s.sections.len(), 2);
        assert_eq!(s.sections[0].fields.len(), 1);
        assert!(s.sections[0].fields[0].name.is_match("phase-2"));
        let item = &s.sections[1].subs[0];
        assert!(item.repeating);
        assert!(item.title.is_match("B-12 Something"));
        assert_eq!(item.fields.len(), 7);
    }

    #[test]
    fn layout_lists_thirteen_families_and_checker() {
        let s = schemas().get("layout").expect("layout");
        assert_eq!(s.sections[0].fields.len(), 14);
    }

    #[test]
    fn split_entries_respects_parentheses_and_literals() {
        assert_eq!(
            split_entries("a (x; y); ``p;q``; b"),
            vec!["a (x; y)", "``p;q``", "b"]
        );
    }
}
