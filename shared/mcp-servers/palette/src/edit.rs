//! Line-level edit helpers shared by the write tools.
//!
//! Owns formatting fields, headings, paragraphs and bullets the way the templates show
//! them, and replacing or inserting only the lines an operation concerns. Every helper
//! works on a [`Source`], so untouched lines keep their bytes and line endings.
//! Does not decide what to write (that is `ops`).

use crate::rst::{Doc, Field, HOUSE_CHARS};
use crate::text::Source;
use crate::util::wrap_words;

/// Column at which generated prose wraps.
pub const WRAP_COLUMN: usize = 80;

/// A collapsed single-line form of user text: whitespace runs become one space.
pub fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Lines of `:Name: value`, wrapped with a two-space continuation indent.
pub fn field_lines(name: &str, value: &str) -> Vec<String> {
    let prefix = format!(":{name}: ");
    let mut lines = wrap_words(&one_line(value), prefix.chars().count(), 2, WRAP_COLUMN);
    if let Some(first) = lines.first_mut() {
        *first = format!("{prefix}{first}");
    }
    lines
}

/// A title line and its underline for `level` (1..=5).
pub fn heading_lines(level: usize, title: &str) -> Vec<String> {
    let c = HOUSE_CHARS[level.clamp(1, 5) - 1];
    vec![
        title.to_string(),
        c.to_string().repeat(title.chars().count().max(3)),
    ]
}

/// A wrapped paragraph.
pub fn paragraph_lines(text: &str) -> Vec<String> {
    wrap_words(&one_line(text), 0, 0, WRAP_COLUMN)
}

/// A wrapped bullet: `- ` first line, two-space continuation.
pub fn bullet_lines(text: &str) -> Vec<String> {
    let mut lines = wrap_words(&one_line(text), 2, 2, WRAP_COLUMN);
    if let Some(first) = lines.first_mut() {
        *first = format!("- {first}");
    }
    lines
}

/// Keeps user text as written except for surrounding blank lines and trailing spaces.
pub fn verbatim_lines(text: &str) -> Vec<String> {
    let mut v: Vec<String> = text.lines().map(|l| l.trim_end().to_string()).collect();
    while v.first().is_some_and(|l| l.is_empty()) {
        v.remove(0);
    }
    while v.last().is_some_and(|l| l.is_empty()) {
        v.pop();
    }
    v
}

/// Replaces the lines of `field` with `:Name: value`. Returns whether anything changed.
pub fn replace_field(src: &mut Source, field: &Field, value: &str) -> bool {
    if field.value == one_line(value) {
        return false;
    }
    let new = field_lines(&field.name, value);
    src.splice(field.start..field.end, &new);
    true
}

/// Sets header field `name` (creating nothing: the field must exist).
pub fn set_header_field(src: &mut Source, name: &str, value: &str) -> Result<bool, String> {
    let doc = Doc::parse(src.clone());
    let field = doc
        .header_fields()
        .into_iter()
        .find(|f| f.name == name)
        .ok_or_else(|| format!("the document has no `:{name}:` field"))?;
    Ok(replace_field(src, &field, value))
}

/// Replaces the text of a heading's title line and lengthens its underline when needed.
pub fn retitle(src: &mut Source, doc: &Doc, heading: usize, new_title: &str) {
    let h = &doc.headings[heading];
    if h.title == new_title {
        return;
    }
    src.splice(h.line..h.line + 1, &[new_title.to_string()]);
    let need = new_title.chars().count();
    if h.underline_len < need {
        let c = h.adorn;
        src.splice(h.underline..h.underline + 1, &[c.to_string().repeat(need)]);
    }
}

/// Where and how to add lines at the end of a section body: the line after the last
/// non-blank line, replacing a lone `None.` paragraph.
pub fn append_to_section(
    src: &mut Source,
    doc: &Doc,
    heading: usize,
    block: &[String],
    blank_before: bool,
) {
    let h = &doc.headings[heading];
    let end = h.body_end;
    let mut last = None;
    let mut none_line = None;
    for i in h.body_start..end {
        let t = doc.src.text(i).trim();
        if !t.is_empty() {
            last = Some(i);
            none_line = if t == "None." { Some(i) } else { none_line };
        }
    }
    let only_none = none_line.is_some()
        && (h.body_start..end)
            .filter(|i| !doc.src.text(*i).trim().is_empty())
            .count()
            == 1;
    if only_none && let Some(n) = none_line {
        src.splice(n..n + 1, block);
        return;
    }
    match last {
        Some(l) => {
            let mut new = Vec::new();
            if blank_before {
                new.push(String::new());
            }
            new.extend(block.iter().cloned());
            src.splice(l + 1..l + 1, &new);
        }
        None => {
            let mut new = vec![String::new()];
            new.extend(block.iter().cloned());
            src.splice(h.body_start..h.body_start, &new);
        }
    }
}

/// Replaces the body of a level-2 section (everything up to the next section of the
/// same or a higher level) with `body`, keeping the heading and one blank line before
/// the next section.
pub fn replace_section_body(src: &mut Source, doc: &Doc, heading: usize, body: &[String]) {
    let h = &doc.headings[heading];
    let mut new = vec![String::new()];
    new.extend(body.iter().cloned());
    if h.end < doc.src.len() {
        new.push(String::new());
    }
    src.splice(h.body_start..h.end, &new);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_lines_wrap_and_prefix() {
        let l = field_lines("Areas", "a; b");
        assert_eq!(l, vec![":Areas: a; b"]);
        let long = "word ".repeat(30);
        let l = field_lines("Description", &long);
        assert!(l.len() > 1);
        assert!(l[1].starts_with("  "));
        assert!(l.iter().all(|x| x.chars().count() <= WRAP_COLUMN));
    }

    #[test]
    fn set_header_field_touches_only_that_field() {
        let mut s = Source::from_text("Title\r\n=====\r\n\r\n:A: one\r\n:B: two\r\n\r\nBody\r\n");
        let before = s.clone();
        assert!(set_header_field(&mut s, "B", "three").expect("set"));
        assert_eq!(s.lines[3], before.lines[3]);
        assert_eq!(
            s.to_text(),
            "Title\r\n=====\r\n\r\n:A: one\r\n:B: three\r\n\r\nBody\r\n"
        );
        assert!(!set_header_field(&mut s, "B", "three").expect("noop"));
    }

    #[test]
    fn append_replaces_none() {
        let mut s = Source::from_text("Title\n=====\n\nItems\n-----\n\nNone.\n");
        let doc = Doc::parse(s.clone());
        append_to_section(&mut s, &doc, 1, &["- x".to_string()], true);
        assert_eq!(s.to_text(), "Title\n=====\n\nItems\n-----\n\n- x\n");
        let doc = Doc::parse(s.clone());
        append_to_section(&mut s, &doc, 1, &["- y".to_string()], false);
        assert_eq!(s.to_text(), "Title\n=====\n\nItems\n-----\n\n- x\n- y\n");
    }

    #[test]
    fn replace_section_keeps_neighbours() {
        let mut s = Source::from_text("Title\n=====\n\nA\n-\n\nold\n\nB\n-\n\nkeep\n");
        let s2 = Source::from_text("Title\n=====\n\nAaa\n---\n\nold\n\nBbb\n---\n\nkeep\n");
        let _ = &mut s;
        let doc = Doc::parse(s2.clone());
        let mut s2 = s2;
        replace_section_body(&mut s2, &doc, 1, &["new".to_string()]);
        assert_eq!(
            s2.to_text(),
            "Title\n=====\n\nAaa\n---\n\nnew\n\nBbb\n---\n\nkeep\n"
        );
    }
}
