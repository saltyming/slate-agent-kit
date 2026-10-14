//! Re-wrapping of RST prose at 79 columns.
//!
//! Owns refilling the paragraphs, list items and definition bodies of a document (or of
//! a fragment a write tool received) at [`WIDTH`] columns, by the scanner's own
//! classification, and the structure check that proves a pass changed line breaks only.
//! Everything else is left byte for byte: titles and underlines, field lists,
//! definition terms, literal blocks (a paragraph keeps its `::` at its end, attached to
//! the block), directives with their whole extent (list-table cells included), tables,
//! comments with their bodies and blank lines. A backtick-delimited span is one token, a
//! link target split over lines is rejoined without a space, an escaped space stays with
//! its word and a token longer than the width stands alone on its line.
//! Does not decide which documents to rewrap (that is `ops::reflow`).
//! Entry points: [`reflow_source`], [`wrap_fragment`], [`shape`].

use std::collections::BTreeSet;
use std::ops::Range;

use crate::directives::Directive;
use crate::rst::{
    Doc, Kind, indent_of, is_field_start, is_item_start, is_table_line, list_marker_width,
};
use crate::text::{Eol, Source};
use crate::util::{glue_words, split_words, wrap_tokens};

/// The column prose is wrapped at: a line is at most this many characters. The width the
/// write tools' own helpers wrap at ([`crate::edit::WRAP_COLUMN`]).
pub const WIDTH: usize = crate::edit::WRAP_COLUMN;

/// Lines inside the extent of a directive, which a pass never touches: the directive
/// line, its options and its body. A `list-table`'s cell text is classified as prose by
/// the scanner; it is held here with the rest of the table.
fn held_lines(doc: &Doc) -> Vec<bool> {
    let mut held = vec![false; doc.src.len()];
    for i in 0..doc.src.len() {
        if doc.kinds[i] != Kind::Directive {
            continue;
        }
        let end = Directive::read(&doc.src, i).map_or(i + 1, |d| d.body.end.max(i + 1));
        for slot in &mut held[i..end] {
            *slot = true;
        }
    }
    held
}

/// For each line, whether it belongs to a run of prose lines (consecutive, non-blank,
/// not held) that holds a table line or a line block, which no pass touches.
fn table_runs(doc: &Doc, held: &[bool]) -> Vec<bool> {
    let n = doc.src.len();
    let mut out = vec![false; n];
    let mut i = 0;
    while i < n {
        if doc.kinds[i] != Kind::Text || held[i] {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < n && doc.kinds[j] == Kind::Text && !held[j] {
            j += 1;
        }
        let table = (i..j).any(|l| is_table_line(doc.src.text(l)));
        for slot in &mut out[i..j] {
            *slot = table;
        }
        i = j;
    }
    out
}

/// The first non-blank line at or after `from`.
fn next_non_blank(doc: &Doc, from: usize) -> Option<usize> {
    (from..doc.src.len()).find(|l| !doc.src.text(*l).trim().is_empty())
}

/// The rewrapped lines of the prose block `block` of `doc`, or `None` when the block is
/// left as it is: it is a field, a definition term, a table, uses tabs, or has nothing to
/// wrap. `tight` holds the lines whose break to the next line lies inside a link target
/// and is rejoined without a space.
fn rewrap_block(doc: &Doc, block: &Range<usize>, tight: &BTreeSet<usize>) -> Option<Vec<String>> {
    let first = doc.src.text(block.start);
    let ind = indent_of(first);
    if (block.start..block.end).any(|l| {
        let t = doc.src.text(l);
        t[..indent_of(t)].contains('\t')
    }) {
        return None;
    }
    let rest = &first[ind..];
    if is_field_start(rest) {
        return None;
    }
    let item = is_item_start(rest);
    // A line followed by a deeper one is a definition term; its body is a block of its
    // own. A term is not wrapped.
    if !item
        && block.end < doc.src.len()
        && doc.kinds[block.end] == Kind::Text
        && indent_of(doc.src.text(block.end)) > ind
    {
        return None;
    }
    let (prefix, first_text) = if item {
        let width = list_marker_width(rest)?;
        let prefix: String = first.chars().take(ind + width).collect();
        let text: String = first.chars().skip(ind + width).collect();
        if is_item_start(&text) || is_field_start(&text) {
            return None;
        }
        (prefix, text)
    } else {
        (first[..ind].to_string(), rest.to_string())
    };
    let mut text = first_text.trim().to_string();
    for l in block.start + 1..block.end {
        text.push_str(if tight.contains(&(l - 1)) { "" } else { " " });
        text.push_str(doc.src.text(l).trim());
    }
    let tokens = glue_words(&split_words(&text));
    if tokens.is_empty() {
        return None;
    }
    let hang = prefix.chars().count();
    let mut lines = wrap_tokens(&tokens, hang, hang, WIDTH);
    if let Some(head) = lines.first_mut() {
        *head = format!("{prefix}{head}");
    }
    // The scanner reads the first non-blank line after a line ending in `::` as literal
    // when it is indented deeper than that line. The block's last line may move to
    // another indent, so the line after it must read as it did before.
    if let Some(next) = next_non_blank(doc, block.end) {
        let literal_now = lines.last().is_some_and(|l| l.ends_with("::"))
            && indent_of(doc.src.text(next)) > lines.last().map_or(0, |l| indent_of(l));
        if literal_now != (doc.kinds[next] == Kind::Literal) {
            return None;
        }
    }
    let old: Vec<&str> = (block.start..block.end).map(|l| doc.src.text(l)).collect();
    (old != lines.iter().map(String::as_str).collect::<Vec<_>>()).then_some(lines)
}

/// Every block of `doc` whose lines change, as `(lines replaced, new lines)` in document
/// order.
fn plan(doc: &Doc) -> Vec<(Range<usize>, Vec<String>)> {
    let held = held_lines(doc);
    let tables = table_runs(doc, &held);
    // A break between two lines inside a link target joins them without a space.
    let mut tight: BTreeSet<usize> = BTreeSet::new();
    for link in doc.links() {
        tight.extend(link.target_start.line..link.target_end.line);
    }
    let mut out = Vec::new();
    for block in doc.prose_blocks() {
        if doc.kinds[block.start] != Kind::Text
            || block.clone().any(|l| held[l] || tables[l])
            || block.clone().any(|l| doc.kinds[l] != Kind::Text)
        {
            continue;
        }
        if let Some(lines) = rewrap_block(doc, &block, &tight) {
            out.push((block, lines));
        }
    }
    out
}

/// What a pass must keep: the lines it never touches, in order, with one marker for each
/// run of prose; the titles; the field lists, links and directives as the scanner reads
/// them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shape {
    fixed: Vec<(Kind, String)>,
    headings: Vec<(usize, String)>,
    fields: Vec<(String, String)>,
    links: Vec<(String, String)>,
    directives: Vec<String>,
}

/// The scanned structure of `src` that a rewrap must leave as it is: block kinds, titles,
/// fields, links and directives.
pub fn shape(src: &Source) -> Shape {
    let doc = Doc::parse(src.clone());
    let held = held_lines(&doc);
    let mut fixed = Vec::new();
    let mut in_run = false;
    for (l, kind) in doc.kinds.iter().enumerate() {
        if *kind == Kind::Text && !held[l] {
            if !in_run {
                fixed.push((Kind::Text, String::new()));
            }
            in_run = true;
        } else {
            in_run = false;
            fixed.push((*kind, doc.src.text(l).to_string()));
        }
    }
    let directives = (0..doc.src.len())
        .filter(|l| doc.kinds[*l] == Kind::Directive)
        .filter_map(|l| Directive::read(&doc.src, l))
        .map(|d| {
            let options: Vec<(String, String)> =
                d.options.into_iter().map(|(_, n, v)| (n, v)).collect();
            format!("{}:: {} {options:?}", d.name, d.argument)
        })
        .collect();
    Shape {
        fixed,
        headings: doc
            .headings
            .iter()
            .map(|h| (h.level, h.title.clone()))
            .collect(),
        fields: doc
            .fields(0, doc.src.len())
            .into_iter()
            .map(|f| (f.name, f.value))
            .collect(),
        links: doc
            .links()
            .into_iter()
            .map(|l| (l.text, l.target))
            .collect(),
        directives,
    }
}

impl Shape {
    /// What differs first between two shapes, in words; `None` when they are equal.
    fn difference(&self, other: &Shape) -> Option<String> {
        if let Some(i) = (0..self.fixed.len().max(other.fixed.len()))
            .find(|i| self.fixed.get(*i) != other.fixed.get(*i))
        {
            let show = |s: &Shape| {
                s.fixed
                    .get(i)
                    .map_or("(none)".to_string(), |(k, t)| format!("{k:?} `{t}`"))
            };
            return Some(format!(
                "line kinds: item {i} is {} and would become {}",
                show(self),
                show(other)
            ));
        }
        if self.headings != other.headings {
            return Some("a title".to_string());
        }
        if self.fields != other.fields {
            return Some("a field".to_string());
        }
        if self.links != other.links {
            return Some("a link".to_string());
        }
        if self.directives != other.directives {
            return Some("a directive".to_string());
        }
        None
    }
}

/// Rewraps the prose of `src` at [`WIDTH`] columns and returns the new text. Only the
/// lines of a changed block are replaced, with the document's own line ending; every
/// other byte is kept. Fails, returning nothing, when the scanned structure of the result
/// would differ from the input's ([`shape`]).
pub fn reflow_source(src: &Source) -> Result<Source, String> {
    let doc = Doc::parse(src.clone());
    let edits = plan(&doc);
    if edits.is_empty() {
        return Ok(src.clone());
    }
    let mut out = src.clone();
    for (range, lines) in edits.into_iter().rev() {
        out.splice(range, &lines);
    }
    match shape(src).difference(&shape(&out)) {
        None => Ok(out),
        Some(what) => Err(format!(
            "rewrapping would change the scanned structure of the document ({what}), so it was left as it is"
        )),
    }
}

/// Rewraps the prose of a fragment of text a write tool received: the lines are parsed as
/// a document of their own and the prose blocks refilled before the fragment is spliced
/// into a file. A fragment whose structure the pass would change is returned as given.
pub fn wrap_fragment(lines: &[String]) -> Vec<String> {
    let src = Source::from_lines(lines, Eol::Lf);
    match reflow_source(&src) {
        Ok(out) => out.lines.into_iter().map(|l| l.text).collect(),
        Err(_) => lines.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flow(text: &str) -> String {
        let src = Source::from_text(text);
        let out = reflow_source(&src).expect("structure kept");
        assert_eq!(
            reflow_source(&out).expect("again").to_text(),
            out.to_text(),
            "idempotent"
        );
        out.to_text()
    }

    #[test]
    fn a_long_paragraph_is_refilled_at_the_width() {
        let words = "word ".repeat(40);
        let out = flow(&format!("Title\n=====\n\n{}\n\nnext\n", words.trim()));
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines.len() > 6, "{out}");
        assert!(lines.iter().all(|l| l.chars().count() <= WIDTH), "{out}");
        assert!(out.starts_with("Title\n=====\n\nword word"));
        assert!(out.ends_with("\n\nnext\n"));
    }

    #[test]
    fn a_paragraph_that_fits_is_joined_and_left_alone_when_already_filled() {
        assert_eq!(flow("a b\nc d\n"), "a b c d\n");
        assert_eq!(flow("a b c d\n"), "a b c d\n");
    }

    #[test]
    fn items_continue_at_their_text_column() {
        let long = "word ".repeat(30);
        let out = flow(&format!("- {}\n\n10. {}\n", long.trim(), long.trim()));
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[1].starts_with("  word"), "{out}");
        assert!(lines.iter().any(|l| l.starts_with("    word")), "{out}");
        assert!(lines.iter().all(|l| l.chars().count() <= WIDTH), "{out}");
    }

    #[test]
    fn a_term_is_kept_and_its_body_is_wrapped_at_its_indent() {
        let long = "word ".repeat(30);
        let out = flow(&format!("a long term that is kept as written\n  {long}\n"));
        let mut lines = out.lines();
        assert_eq!(lines.next(), Some("a long term that is kept as written"));
        assert!(lines.all(|l| l.starts_with("  word") && l.chars().count() <= WIDTH));
    }

    #[test]
    fn a_link_target_split_over_lines_is_rejoined_without_a_space() {
        let text = "see `the alpha\ncontract <../spec/a-very/long-\npath.rst#section>`_ now\n";
        let out = flow(text);
        assert_eq!(
            out,
            "see `the alpha contract <../spec/a-very/long-path.rst#section>`_ now\n"
        );
    }

    #[test]
    fn a_line_never_starts_with_markup() {
        let pad = "w".repeat(70);
        for lead in ["-", "1.", "(a)", "..", "mild.", "----", "|", ":x:", "+--"] {
            let out = flow(&format!("{pad} {lead} tail\n"));
            assert!(
                out.lines()
                    .all(|l| !is_item_start(l) && !l.starts_with("..")),
                "{lead}: {out}"
            );
            assert!(out.contains(&format!("{pad} {lead} tail")) || out.contains(lead));
        }
    }

    #[test]
    fn an_escaped_space_stays_with_its_word() {
        let pad = "w".repeat(75);
        let out = flow(&format!("{pad} ``a``\\ s tail\n"));
        assert!(out.contains("``a``\\ s"), "{out}");
    }
}
