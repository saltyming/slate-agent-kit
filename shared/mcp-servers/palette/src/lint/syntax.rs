//! P001: the RST subset of the house style.
//!
//! Flags tables, directives, overlined titles, short underlines, heading levels out of
//! order, substitutions, footnotes and citations. Checks lines lexically; literal
//! blocks, inline literals and comments are skipped.

use std::sync::LazyLock;

use regex::Regex;

use super::{Cx, Finding};
use crate::rst::{Kind, mask_inline_literals};
use crate::util::re;

static GRID_BORDER: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*\+(?:[-=]+\+)+\s*$"));
static SIMPLE_BORDER: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*=+(?:\s+=+)+\s*$"));
static PIPE_SEPARATOR: LazyLock<Regex> =
    LazyLock::new(|| re(r"^\s*\|?\s*:?-{3,}:?\s*(?:\|\s*:?-{3,}:?\s*)+\|?\s*$"));
static SUBST_REF: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?:^|[\s(\[])\|[A-Za-z0-9][^|\s]*(?: [^|\s]+)*\|_{0,2}(?:$|[\s.,;:)\]])")
});
static NOTE_REF: LazyLock<Regex> = LazyLock::new(|| re(r"\[(?:\d+|#[\w-]*|\*|[A-Za-z][\w.-]*)\]_"));
static DIRECTIVE_NAME: LazyLock<Regex> = LazyLock::new(|| re(r"^\.\.\s+([A-Za-z0-9][\w+:.-]*)::"));

pub(super) fn check(cx: &Cx, out: &mut Vec<Finding>) {
    for f in &cx.snap.files {
        let Some(doc) = &f.doc else {
            if let Some(line) = f.bad_utf8 {
                out.push(Finding::error(
                    "P001",
                    f,
                    line - 1,
                    "the file is not valid UTF-8",
                ));
            }
            continue;
        };
        let mut prev_level = 0usize;
        for h in &doc.headings {
            if h.overlined {
                out.push(Finding::error(
                    "P001",
                    f,
                    h.start,
                    format!("`{}` has an overline; titles are underlined only", h.title),
                ));
            }
            if h.level == 0 {
                out.push(Finding::error(
                    "P001",
                    f,
                    h.line,
                    format!(
                        "`{}` is underlined with `{}`; the house-style order is `=`, `-`, `~`, `^`, `\"`",
                        h.title, h.adorn
                    ),
                ));
            } else {
                if h.level > prev_level + 1 {
                    let msg = if prev_level == 0 {
                        format!(
                            "the first heading `{}` must be the title, underlined with `=`",
                            h.title
                        )
                    } else {
                        format!(
                            "`{}` skips a heading level (after level {prev_level} comes level {})",
                            h.title,
                            prev_level + 1
                        )
                    };
                    out.push(Finding::error("P001", f, h.line, msg));
                }
                prev_level = h.level;
            }
            let width = h.title.chars().count();
            if h.underline_len < width {
                out.push(Finding::error(
                    "P001",
                    f,
                    h.underline,
                    format!(
                        "the underline of `{}` is shorter than its title ({} < {width})",
                        h.title, h.underline_len
                    ),
                ));
            }
        }
        let mut last_table_line: Option<usize> = None;
        for i in 0..doc.src.len() {
            let text = doc.src.text(i);
            match doc.kinds[i] {
                Kind::Directive => {
                    let name = DIRECTIVE_NAME
                        .captures(text.trim_start())
                        .map(|c| c[1].to_string())
                        .unwrap_or_default();
                    out.push(Finding::error("P001", f, i, format!("directive `{name}::` is not allowed; only comment lines may start with `..`")));
                }
                Kind::SubstDef => out.push(Finding::error(
                    "P001",
                    f,
                    i,
                    "substitution definitions are not allowed",
                )),
                Kind::NoteDef => out.push(Finding::error(
                    "P001",
                    f,
                    i,
                    "footnotes and citations are not allowed",
                )),
                Kind::Text => {
                    if GRID_BORDER.is_match(text)
                        || SIMPLE_BORDER.is_match(text)
                        || PIPE_SEPARATOR.is_match(text)
                    {
                        if last_table_line.is_none_or(|l| l + 3 < i) {
                            out.push(Finding::error(
                                "P001",
                                f,
                                i,
                                "tables are not allowed; use a list or a field list",
                            ));
                        }
                        last_table_line = Some(i);
                        continue;
                    }
                    let masked = mask_inline_literals(text);
                    if SUBST_REF.is_match(&masked) {
                        out.push(Finding::error(
                            "P001",
                            f,
                            i,
                            "substitution references (`|name|`) are not allowed",
                        ));
                    }
                    if NOTE_REF.is_match(&masked) {
                        out.push(Finding::error(
                            "P001",
                            f,
                            i,
                            "footnote and citation references (`[1]_`) are not allowed",
                        ));
                    }
                }
                _ => {}
            }
        }
    }
}
