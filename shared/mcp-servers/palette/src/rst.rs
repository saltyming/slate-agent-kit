//! Line-oriented scanner for the house-style RST subset.
//!
//! Owns classifying each line (blank, text, literal block, explicit markup, with
//! list-table cells classified as the text they hold), finding section headings with
//! their extents, field lists, hyperlinks and the docutils-compatible anchor
//! normalization. Does not check house-style rules
//! (that is `lint`) and does not know any document family.
//! Entry point: [`Doc::parse`].

use std::sync::LazyLock;

use regex::Regex;

use crate::directives::{Directive, masked_cells};
use crate::text::{Eol, Source};
use crate::util::re;

/// Adornment characters in depth order: level 1 is `=`, level 5 is `"`.
pub const HOUSE_CHARS: [char; 5] = ['=', '-', '~', '^', '"'];

/// Depth (1..=5) of a house adornment character.
pub fn level_of(c: char) -> Option<usize> {
    HOUSE_CHARS.iter().position(|h| *h == c).map(|i| i + 1)
}

/// What a line is, as far as the palette checks care.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Empty or whitespace only.
    Blank,
    /// Ordinary text (paragraph, list, field, definition).
    Text,
    /// Inside a literal block.
    Literal,
    /// A comment line (`.. text`).
    Comment,
    /// A directive (`.. name::`).
    Directive,
    /// A substitution definition (`.. |name| ...`).
    SubstDef,
    /// A footnote or citation definition (`.. [x] ...`).
    NoteDef,
    /// A hyperlink target (`.. _name:`).
    Target,
    /// An indented continuation of explicit markup. The cell content of a
    /// `list-table` is not: it is classified as the text it is.
    ExplicitBody,
    /// A section title text line.
    Title,
    /// An underline or overline of a title.
    Adornment,
}

/// A section heading.
#[derive(Clone, Debug)]
pub struct Heading {
    /// Depth 1..=5 for a house adornment; 0 for any other character.
    pub level: usize,
    /// The adornment character.
    pub adorn: char,
    /// Title text, trimmed.
    pub title: String,
    /// Title text line (0-based).
    pub line: usize,
    /// First line of the heading (the overline when present, else the title).
    pub start: usize,
    /// Underline line.
    pub underline: usize,
    /// Underline length in characters.
    pub underline_len: usize,
    /// Whether the heading has an overline.
    pub overlined: bool,
    /// Exclusive end of the section including subsections.
    pub end: usize,
    /// First body line (after the underline).
    pub body_start: usize,
    /// Exclusive end of the body before the first subsection.
    pub body_end: usize,
}

impl Heading {
    fn effective_level(&self) -> usize {
        if self.level == 0 { 6 } else { self.level }
    }
}

/// A field-list entry (`:Name: value`) at column 0.
#[derive(Clone, Debug)]
pub struct Field {
    /// Field name as written.
    pub name: String,
    /// Value with continuation lines joined by single spaces.
    pub value: String,
    /// First line (0-based).
    pub start: usize,
    /// Exclusive end line.
    pub end: usize,
}

/// A position in a document: a line (0-based) and a byte offset inside its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pos {
    /// Line (0-based).
    pub line: usize,
    /// Byte offset inside the line text.
    pub col: usize,
}

/// A hyperlink `` `text <target>`_ ``, possibly wrapped over several lines of one
/// block of prose.
#[derive(Clone, Debug)]
pub struct Link {
    /// The line that holds the target (its first non-blank character); for a link on
    /// one line, that line.
    pub line: usize,
    /// Link text, with each line break and the indentation around it read as one space.
    pub text: String,
    /// Link target, trimmed, with each line break and the indentation around it
    /// removed, as docutils reads a wrapped URI.
    pub target: String,
    /// Start of the target as written between the angle brackets.
    pub target_start: Pos,
    /// Exclusive end of the target as written; on a later line than `target_start`
    /// when the target itself wraps.
    pub target_end: Pos,
}

/// A parsed document: the source lines plus per-line kinds and headings.
#[derive(Clone, Debug)]
pub struct Doc {
    /// The underlying lines.
    pub src: Source,
    /// Kind of each line.
    pub kinds: Vec<Kind>,
    /// Section headings in document order.
    pub headings: Vec<Heading>,
}

static FIELD_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r"^:([^:\s](?:[^:]*[^:\s])?):(?:[ \t]+(.*?))?[ \t]*$"));
static LINK_RE: LazyLock<Regex> = LazyLock::new(|| re(r"`([^`<>]*?)\s*<([^<>`]+)>`__?"));
static INLINE_LITERAL_RE: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)``.+?``"));
/// A line break with the indentation around it, inside a wrapped link.
static LINE_BREAK_RE: LazyLock<Regex> = LazyLock::new(|| re(r"[ \t]*\n[ \t]*"));
/// A bullet, enumerator or field marker opening a list item or a field.
static ITEM_MARKER_RE: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^(?:[-*+•‣⁃]|(?:\d+|#|[A-Za-z]|[ivxlcdmIVXLCDM]+)[.)]|\((?:\d+|#|[A-Za-z]|[ivxlcdmIVXLCDM]+)\))(?:[ \t]|$)|^:[^:\s](?:[^:]*[^:\s])?:(?:[ \t]|$)",
    )
});
static DIRECTIVE_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r"^\.\.[ \t]+[A-Za-z0-9][\w+:.-]*::(\s|$)"));

fn indent_of(s: &str) -> usize {
    s.chars().take_while(|c| *c == ' ' || *c == '\t').count()
}

/// Returns the adornment character when `s` is an underline/overline candidate.
fn adornment_char(s: &str) -> Option<char> {
    let t = s.trim_end();
    let mut chars = t.chars();
    let c = chars.next()?;
    if t.chars().count() < 3 || c.is_alphanumeric() || c.is_whitespace() {
        return None;
    }
    if !c.is_ascii_punctuation() {
        return None;
    }
    if chars.all(|x| x == c) { Some(c) } else { None }
}

/// Replaces inline literals with spaces of the same byte length so offsets stay
/// valid. A literal may span lines of `s`; its line breaks are kept.
pub fn mask_inline_literals(s: &str) -> String {
    INLINE_LITERAL_RE
        .replace_all(s, |c: &regex::Captures| {
            c[0].chars()
                .map(|ch| {
                    if ch == '\n' {
                        "\n".to_string()
                    } else {
                        " ".repeat(ch.len_utf8())
                    }
                })
                .collect::<String>()
        })
        .into_owned()
}

impl Doc {
    /// Scans `src`.
    pub fn parse(src: Source) -> Doc {
        let n = src.len();
        let mut kinds = vec![Kind::Text; n];
        let mut headings: Vec<Heading> = Vec::new();

        let mut explicit: Option<usize> = None;
        let mut literal_pending: Option<usize> = None;
        let mut literal_base: Option<usize> = None;

        let mut i = 0;
        while i < n {
            let text = src.text(i);
            let blank = text.trim().is_empty();
            let ind = indent_of(text);

            if let Some(base) = literal_base {
                if blank || ind > base {
                    kinds[i] = if blank { Kind::Blank } else { Kind::Literal };
                    i += 1;
                    continue;
                }
                literal_base = None;
            }
            if let Some(base) = explicit {
                if blank {
                    kinds[i] = Kind::Blank;
                    i += 1;
                    continue;
                }
                if ind > base {
                    kinds[i] = Kind::ExplicitBody;
                    i += 1;
                    continue;
                }
                explicit = None;
            }
            if blank {
                kinds[i] = Kind::Blank;
                i += 1;
                continue;
            }
            if let Some(base) = literal_pending {
                // The block starts at the first non-blank line after `::`.
                if ind > base {
                    literal_base = Some(base);
                    literal_pending = None;
                    kinds[i] = Kind::Literal;
                    i += 1;
                    continue;
                }
                literal_pending = None;
            }

            let trimmed = text.trim_start();
            if trimmed.starts_with("..")
                && (trimmed.len() == 2 || trimmed[2..].starts_with([' ', '\t']))
            {
                explicit = Some(ind);
                kinds[i] = classify_explicit(trimmed);
                i += 1;
                continue;
            }

            // Overlined heading: adornment, title, adornment of the same character.
            if ind == 0
                && let Some(c) = adornment_char(text)
                && i + 2 < n
                && adornment_char(src.text(i + 2)) == Some(c)
                && indent_of(src.text(i + 1)) == 0
                && !src.text(i + 1).trim().is_empty()
                && adornment_char(src.text(i + 1)).is_none()
            {
                push_heading(&mut headings, &src, c, i + 1, i, i + 2, true);
                kinds[i] = Kind::Adornment;
                kinds[i + 1] = Kind::Title;
                kinds[i + 2] = Kind::Adornment;
                i += 3;
                continue;
            }
            // Underlined heading: a text line followed by an adornment line.
            if ind == 0
                && adornment_char(text).is_none()
                && i + 1 < n
                && let Some(c) = adornment_char(src.text(i + 1))
                && indent_of(src.text(i + 1)) == 0
            {
                push_heading(&mut headings, &src, c, i, i, i + 1, false);
                kinds[i] = Kind::Title;
                kinds[i + 1] = Kind::Adornment;
                i += 2;
                continue;
            }

            kinds[i] = Kind::Text;
            if text.trim_end().ends_with("::") {
                literal_pending = Some(ind);
            }
            i += 1;
        }

        // A list-table's rows are explicit markup to the scan above; its cell content
        // is scanned again as text, so links, references and nested blocks are seen.
        let mut i = 0;
        while i < n {
            if kinds[i] == Kind::Directive
                && let Some(d) = Directive::read(&src, i)
                && d.name == "list-table"
                && !d.body.is_empty()
            {
                // Each cell on its own: a block left open in one cell ends at the next.
                for (start, lines) in masked_cells(&src, &d) {
                    let cell = Doc::parse(Source::from_lines(&lines, Eol::Lf));
                    for (k, kind) in cell.kinds.into_iter().enumerate() {
                        kinds[start + k] = kind;
                    }
                }
                i = d.body.end;
                continue;
            }
            i += 1;
        }

        // Section extents.
        let count = headings.len();
        for h in 0..count {
            let lvl = headings[h].effective_level();
            let end = headings[h + 1..]
                .iter()
                .find(|o| o.effective_level() <= lvl)
                .map(|o| o.start)
                .unwrap_or(n);
            let body_end = headings.get(h + 1).map(|o| o.start).unwrap_or(n);
            headings[h].end = end;
            headings[h].body_end = body_end;
        }
        Doc {
            src,
            kinds,
            headings,
        }
    }

    /// Parses `bytes`; `Err(line)` names the first line holding invalid UTF-8.
    pub fn parse_bytes(bytes: &[u8]) -> Result<Doc, usize> {
        Source::from_bytes(bytes).map(Doc::parse)
    }

    /// The first level-1 heading.
    pub fn title(&self) -> Option<&Heading> {
        self.headings.iter().find(|h| h.level == 1)
    }

    /// Headings at exactly `level`.
    pub fn headings_at(&self, level: usize) -> impl Iterator<Item = (usize, &Heading)> {
        self.headings
            .iter()
            .enumerate()
            .filter(move |(_, h)| h.level == level)
    }

    /// Indexes of the direct children of heading `idx` (headings one level deeper
    /// inside its extent).
    pub fn children(&self, idx: usize) -> Vec<usize> {
        let h = &self.headings[idx];
        let mut out = Vec::new();
        for (j, o) in self.headings.iter().enumerate().skip(idx + 1) {
            if o.start >= h.end {
                break;
            }
            if o.level == h.level + 1 {
                out.push(j);
            }
        }
        out
    }

    /// Titles of the ancestors of heading `idx`, outermost first, excluding the
    /// document title (level 1).
    pub fn ancestors(&self, idx: usize) -> Vec<String> {
        let target = &self.headings[idx];
        let mut chain: Vec<&Heading> = Vec::new();
        for h in &self.headings[..idx] {
            while chain
                .last()
                .is_some_and(|l| l.effective_level() >= h.effective_level())
            {
                chain.pop();
            }
            chain.push(h);
        }
        chain
            .into_iter()
            .filter(|h| h.level >= 2 && h.effective_level() < target.effective_level())
            .map(|h| h.title.clone())
            .collect()
    }

    /// Whether line `i` is ordinary prose: not literal, not explicit markup.
    pub fn is_prose(&self, i: usize) -> bool {
        matches!(self.kinds[i], Kind::Text | Kind::Title)
    }

    /// Field-list entries at column 0 within the lines `[start, end)`.
    pub fn fields(&self, start: usize, end: usize) -> Vec<Field> {
        let end = end.min(self.src.len());
        let mut out = Vec::new();
        let mut i = start;
        while i < end {
            if self.kinds[i] == Kind::Text
                && let Some(c) = FIELD_RE.captures(self.src.text(i))
            {
                let name = c[1].to_string();
                let mut value = c.get(2).map(|m| m.as_str().to_string()).unwrap_or_default();
                let mut j = i + 1;
                while j < end
                    && self.kinds[j] == Kind::Text
                    && indent_of(self.src.text(j)) > 0
                    && !self.src.text(j).trim().is_empty()
                {
                    let piece = self.src.text(j).trim();
                    if !value.is_empty() {
                        value.push(' ');
                    }
                    value.push_str(piece);
                    j += 1;
                }
                out.push(Field {
                    name,
                    value,
                    start: i,
                    end: j,
                });
                i = j;
                continue;
            }
            i += 1;
        }
        out
    }

    /// Header fields: the field list between the title and the first later heading.
    pub fn header_fields(&self) -> Vec<Field> {
        let Some(t) = self.title() else {
            return Vec::new();
        };
        let after = self
            .headings
            .iter()
            .find(|h| h.start > t.start)
            .map(|h| h.start)
            .unwrap_or(self.src.len());
        self.fields(t.body_start, after)
    }

    /// Fields inside the body of heading `idx` (before its first subsection).
    pub fn body_fields(&self, idx: usize) -> Vec<Field> {
        let h = &self.headings[idx];
        self.fields(h.body_start, h.body_end)
    }

    /// Hyperlinks in prose, each block of prose read as one text so a link may wrap
    /// over its lines.
    pub fn links(&self) -> Vec<Link> {
        let mut out = Vec::new();
        for block in self.prose_blocks() {
            // Line starts inside the joined text; each line keeps its indentation.
            let mut starts = Vec::with_capacity(block.len());
            let mut joined = String::new();
            for i in block.clone() {
                if i > block.start {
                    joined.push('\n');
                }
                starts.push(joined.len());
                joined.push_str(self.src.text(i));
            }
            let pos = |off: usize| {
                let k = starts.partition_point(|s| *s <= off) - 1;
                Pos {
                    line: block.start + k,
                    col: off - starts[k],
                }
            };
            let masked = mask_inline_literals(&joined);
            for c in LINK_RE.captures_iter(&masked) {
                if let (Some(t), Some(target)) = (c.get(1), c.get(2)) {
                    let raw = &joined[target.range()];
                    let lead = raw.len() - raw.trim_start().len();
                    let first = if lead < raw.len() { lead } else { 0 };
                    out.push(Link {
                        line: pos(target.start() + first).line,
                        text: LINE_BREAK_RE
                            .replace_all(&joined[t.range()], " ")
                            .into_owned(),
                        target: LINE_BREAK_RE.replace_all(raw.trim(), "").into_owned(),
                        target_start: pos(target.start()),
                        target_end: pos(target.end()),
                    });
                }
            }
        }
        out
    }

    /// The blocks of prose inline markup may wrap in: runs of consecutive text lines.
    /// A block ends at any other line (blank, title, literal, explicit markup) and
    /// before a line that opens a list item or a field. A block opened by a list item
    /// or a field continues on lines indented past its marker; any other block (a
    /// paragraph, a definition body) continues on lines at its first line's indent, so
    /// a definition term and its body, or a list item and the next, stay apart.
    /// List-table cells are text lines whose markers open blocks of their own.
    fn prose_blocks(&self) -> Vec<std::ops::Range<usize>> {
        let mut out = Vec::new();
        let n = self.src.len();
        let mut i = 0;
        while i < n {
            match self.kinds[i] {
                Kind::Title => {
                    out.push(i..i + 1);
                    i += 1;
                }
                Kind::Text => {
                    let text = self.src.text(i);
                    let ind = indent_of(text);
                    let item = ITEM_MARKER_RE.is_match(&text[ind..]);
                    let mut j = i + 1;
                    while j < n && self.kinds[j] == Kind::Text {
                        let t = self.src.text(j);
                        let k = indent_of(t);
                        let continues = if item { k > ind } else { k == ind };
                        if !continues || ITEM_MARKER_RE.is_match(&t[k..]) {
                            break;
                        }
                        j += 1;
                    }
                    out.push(i..j);
                    i = j;
                }
                _ => i += 1,
            }
        }
        out
    }

    /// The normalized anchor names this document defines: one per heading plus
    /// explicit `.. _name:` targets.
    pub fn anchors(&self) -> Vec<String> {
        let mut out: Vec<String> = self.headings.iter().map(|h| make_id(&h.title)).collect();
        for i in 0..self.src.len() {
            if self.kinds[i] == Kind::Target {
                let t = self.src.text(i).trim_start();
                let name = t[2..].trim_start().trim_start_matches('_');
                let name = name
                    .split(':')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .trim_matches('`');
                out.push(make_id(name));
            }
        }
        out
    }

    /// Non-blank text lines of `[start, end)` that are prose, joined with newlines.
    pub fn prose_between(&self, start: usize, end: usize) -> String {
        let mut out = Vec::new();
        for i in start..end.min(self.src.len()) {
            if self.kinds[i] != Kind::Blank {
                out.push(self.src.text(i).trim());
            }
        }
        out.join("\n")
    }
}

fn classify_explicit(trimmed: &str) -> Kind {
    let rest = trimmed[2..].trim_start();
    if DIRECTIVE_RE.is_match(trimmed) {
        Kind::Directive
    } else if rest.starts_with('|') {
        Kind::SubstDef
    } else if rest.starts_with('[') {
        Kind::NoteDef
    } else if rest.starts_with('_') {
        Kind::Target
    } else {
        Kind::Comment
    }
}

fn push_heading(
    headings: &mut Vec<Heading>,
    src: &Source,
    c: char,
    title_line: usize,
    start: usize,
    underline: usize,
    overlined: bool,
) {
    headings.push(Heading {
        level: level_of(c).unwrap_or(0),
        adorn: c,
        title: src.text(title_line).trim().to_string(),
        line: title_line,
        start,
        underline,
        underline_len: src.text(underline).trim_end().chars().count(),
        overlined,
        end: 0,
        body_start: underline + 1,
        body_end: 0,
    });
}

/// Docutils identifier normalization: lowercase, runs of non-alphanumerics become a
/// single hyphen, leading non-letters and trailing hyphens are dropped.
pub fn make_id(s: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for ch in s.chars().flat_map(|c| c.to_lowercase()) {
        if ch.is_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(ch);
        } else {
            pending_dash = true;
        }
    }
    let trimmed = out.trim_start_matches(|c: char| !c.is_alphabetic());
    trimmed.to_string()
}

/// Splits a link target into `(path, anchor)`.
pub fn split_anchor(target: &str) -> (&str, Option<&str>) {
    match target.split_once('#') {
        Some((p, a)) => (p, Some(a)),
        None => (target, None),
    }
}

/// Whether `target` points outside the project (a URL or mail address).
pub fn is_external(target: &str) -> bool {
    let lower = target.to_ascii_lowercase();
    lower.contains("://") || lower.starts_with("mailto:")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(s: &str) -> Doc {
        Doc::parse(Source::from_text(s))
    }

    #[test]
    fn finds_headings_and_extents() {
        let d = doc("Title\n=====\n\n:A: b\n\nOne\n---\n\ntext\n\nSub\n~~~\n\nx\n\nTwo\n---\n");
        assert_eq!(d.headings.len(), 4);
        assert_eq!(d.headings[1].title, "One");
        assert_eq!(d.headings[1].end, 15);
        assert_eq!(d.children(1), vec![2]);
        assert_eq!(d.ancestors(2), vec!["One".to_string()]);
    }

    #[test]
    fn overline_and_short_underline_are_recorded() {
        let d = doc("=====\nTitle\n=====\n\nAbcdef\n--\n");
        assert!(d.headings[0].overlined);
        // A two-character rule is not an adornment, so the second heading is not found.
        assert_eq!(d.headings.len(), 1);
        let d = doc("Abcdef\n---\n");
        assert_eq!(d.headings[0].underline_len, 3);
    }

    #[test]
    fn fields_join_continuations() {
        let d = doc("Title\n=====\n\n:Impl: not-started — one\n  two\n:Next: x\n\nBody\n");
        let f = d.header_fields();
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].value, "not-started — one two");
        assert_eq!((f[0].start, f[0].end), (3, 5));
    }

    #[test]
    fn list_table_cells_are_scanned_one_by_one() {
        // The second cell's text sits deeper than the code block that closes the first
        // cell; it is still the second cell's prose, not the block's content.
        let d = doc(
            ".. list-table::\n\n   * - .. code-block:: text\n\n          x\n     -  `gone <missing.rst>`_\n",
        );
        assert_eq!(d.kinds[4], Kind::ExplicitBody);
        assert_eq!(d.kinds[5], Kind::Text);
        assert_eq!(d.links().len(), 1);
    }

    #[test]
    fn literal_blocks_are_not_scanned() {
        let d = doc("Title\n=====\n\nExample::\n\n  Fake\n  ----\n\nafter\n");
        assert_eq!(d.headings.len(), 1);
        assert_eq!(d.kinds[5], Kind::Literal);
    }

    #[test]
    fn links_skip_inline_literals() {
        let d = doc("Title\n=====\n\nsee `a <x.rst#s>`_ and ``not `b <y.rst>`_ here``\n");
        let l = d.links();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].target, "x.rst#s");
        // A literal that wraps hides the link inside it too.
        let d = doc("Title\n=====\n\nsee ``not `b\n<y.rst>`_ here`` and `a\n<x.rst>`_\n");
        let l = d.links();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].target, "x.rst");
    }

    #[test]
    fn links_wrap_within_a_block() {
        let d = doc("Title\n=====\n\nsee `the\nx` or `the alpha\ncontract <a.rst#s>`_ now\n");
        let l = d.links();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].text, "the alpha contract");
        assert_eq!(l[0].target, "a.rst#s");
        assert_eq!(l[0].line, 5);
        assert_eq!(l[0].target_start, Pos { line: 5, col: 10 });
        assert_eq!(l[0].target_end, Pos { line: 5, col: 17 });

        // A wrapped target loses the line break and indentation; the link is reported on
        // the line where the target starts.
        let d = doc("- item `text <long/\n  path.rst>`_ end\n");
        let l = d.links();
        assert_eq!(l[0].target, "long/path.rst");
        assert_eq!(l[0].line, 0);
        assert_eq!(l[0].target_start, Pos { line: 0, col: 14 });
        assert_eq!(l[0].target_end, Pos { line: 1, col: 10 });

        // One-line links keep their positions.
        let d = doc("a `b <c.rst>`_ and `d < e.rst >`_\n");
        let l = d.links();
        assert_eq!(l.len(), 2);
        assert_eq!((l[0].target_start.col, l[0].target_end.col), (6, 11));
        assert_eq!(l[1].target, "e.rst");
        assert_eq!((l[1].target_start.col, l[1].target_end.col), (23, 30));
    }

    #[test]
    fn links_do_not_join_separate_blocks() {
        for text in [
            // Paragraphs split by a blank line.
            "a `b\n\nc <x.rst>`_\n",
            // Two list items, bullet and enumerated.
            "- a `b\n- c <x.rst>`_\n",
            "1. a `b\n2. c <x.rst>`_\n",
            // Two fields.
            ":A: `b\n:B: c <x.rst>`_\n",
            // A definition term and its body.
            "term `b\n  body <x.rst>`_\n",
            // A paragraph and a shallower line.
            "  a `b\nc <x.rst>`_\n",
            // A heading and its body.
            "Title `b\n=======\n\nc <x.rst>`_\n",
            // A paragraph and a literal block.
            "a `b::\n\n  c <x.rst>`_\n",
            // A paragraph and a directive.
            "a `b\n.. note:: c <x.rst>`_\n",
        ] {
            assert!(doc(text).links().is_empty(), "{text:?}");
        }
        // A list item continues on lines indented past its marker.
        let d = doc("- a `b\n  c <x.rst>`_\n:A: `b\n   c <y.rst>`_\n");
        let l = d.links();
        assert_eq!(l.len(), 2);
        assert_eq!((l[0].line, l[1].line), (1, 3));
    }

    #[test]
    fn links_wrap_within_one_list_table_cell() {
        let d = doc(
            ".. list-table::\n\n   * - `a\n       b <x.rst>`_\n     - `c\n     - d <y.rst>`_\n",
        );
        let l = d.links();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].target, "x.rst");
        assert_eq!(l[0].line, 3);
    }

    #[test]
    fn make_id_matches_docutils() {
        assert_eq!(make_id("Scope and authority"), "scope-and-authority");
        assert_eq!(make_id("1. Lint rules (P001)"), "lint-rules-p001");
    }
}
