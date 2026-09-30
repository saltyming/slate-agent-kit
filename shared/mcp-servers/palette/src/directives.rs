//! The two directives the house style admits in record, changeset, staging and
//! maintained documents: `list-table` and `code-block`.
//!
//! Owns reading a directive into its argument, options and body, the list-table row
//! and cell grammar, the masking the scanner uses to scan cell content as text, and
//! the structure problems P017 reports. Works on source lines only; which family
//! admits a directive is the lint's decision.
//! Entry points: [`name_at`], [`Directive::read`], [`problems`], [`masked_cells`].

use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

use crate::text::Source;
use crate::util::re;

/// The directives some families admit.
pub const ADMITTED: [&str; 2] = ["list-table", "code-block"];

static NAME_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r"^\.\.[ \t]+([A-Za-z0-9][\w+:.-]*?)::(?:[ \t]+(.*?))?[ \t]*$"));
static OPTION_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r"^:([A-Za-z][\w-]*):(?:[ \t]+(.*?))?[ \t]*$"));
static LANGUAGE_RE: LazyLock<Regex> = LazyLock::new(|| re(r"^[A-Za-z0-9][A-Za-z0-9_+.#-]*$"));

fn indent_of(s: &str) -> usize {
    s.chars().take_while(|c| *c == ' ' || *c == '\t').count()
}

fn is_blank(s: &str) -> bool {
    s.trim().is_empty()
}

/// Whether `rest` starts with `marker` alone or followed by whitespace.
fn has_marker(rest: &str, marker: &str) -> bool {
    rest.strip_prefix(marker)
        .is_some_and(|r| r.is_empty() || r.starts_with([' ', '\t']))
}

/// The directive on `line`, if any: the column of its `..`, its name and the argument
/// text after `::`. A leading table row or cell marker (`* -`, `-`) is skipped, so a
/// directive that opens a cell is found too.
pub fn name_at(line: &str) -> Option<(usize, String, String)> {
    let mut col = indent_of(line);
    let mut rest = &line[col..];
    for marker in ["* -", "-"] {
        if has_marker(rest, marker) {
            let after = &rest[marker.len()..];
            let skip = marker.len() + indent_of(after);
            rest = &rest[skip..];
            col += skip;
            break;
        }
    }
    let c = NAME_RE.captures(rest)?;
    let argument = c.get(2).map(|m| m.as_str().to_string()).unwrap_or_default();
    Some((col, c[1].to_string(), argument))
}

/// One directive split into its parts.
#[derive(Clone, Debug)]
pub struct Directive {
    /// 0-based line of `.. name::`.
    pub line: usize,
    /// Column of the `..`.
    pub col: usize,
    /// Directive name.
    pub name: String,
    /// Argument text on the directive line, trimmed.
    pub argument: String,
    /// Option lines directly after the directive line: line, name, value.
    pub options: Vec<(usize, String, String)>,
    /// The first line after the directive line that is neither an option nor
    /// separated by a blank line: the body starts without its blank line.
    pub unseparated: Option<usize>,
    /// The body: from its first line to its last non-blank line; empty when absent.
    pub body: Range<usize>,
}

impl Directive {
    /// Reads the directive on line `line` of `src`; `None` when there is none.
    pub fn read(src: &Source, line: usize) -> Option<Directive> {
        let (col, name, argument) = name_at(src.text(line))?;
        let n = src.len();
        let mut last = line;
        let mut j = line + 1;
        while j < n {
            let t = src.text(j);
            if !is_blank(t) {
                if indent_of(t) <= col {
                    break;
                }
                last = j;
            }
            j += 1;
        }
        let end = last + 1;
        let mut options = Vec::new();
        let mut unseparated = None;
        let mut i = line + 1;
        while i < end && !is_blank(src.text(i)) {
            let t = src.text(i).trim();
            match OPTION_RE.captures(t) {
                Some(c) if unseparated.is_none() => options.push((
                    i,
                    c[1].to_string(),
                    c.get(2).map(|m| m.as_str().to_string()).unwrap_or_default(),
                )),
                _ => {
                    unseparated.get_or_insert(i);
                }
            }
            i += 1;
        }
        while i < end && is_blank(src.text(i)) {
            i += 1;
        }
        let start = unseparated.unwrap_or(i);
        Some(Directive {
            line,
            col,
            name,
            argument: argument.trim().to_string(),
            options,
            unseparated,
            body: start..end.max(start),
        })
    }
}

/// The structure problems of `d` as `(line, message)`; none for a directive this
/// module does not check.
pub fn problems(src: &Source, d: &Directive) -> Vec<(usize, String)> {
    let mut out = match d.name.as_str() {
        "list-table" => list_table(src, d),
        "code-block" => code_block(d),
        _ => return Vec::new(),
    };
    // docutils expands a tab to the next multiple of eight columns, so a tab in the
    // indentation moves a line in or out of the directive without showing it.
    // The leading whitespace of a line; on the directive line, everything before `..`.
    let lead = |l: usize| {
        let t = src.text(l);
        let end = if l == d.line { d.col } else { indent_of(t) };
        &t[..end]
    };
    let mut lines = std::iter::once(d.line)
        .chain(d.options.iter().map(|o| o.0))
        .chain(d.body.clone());
    if let Some(l) = lines.find(|l| lead(*l).contains('\t')) {
        out.push((l, format!("indent the {} with spaces, not tabs", d.name)));
    }
    out
}

fn list_table(src: &Source, d: &Directive) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut header_rows: Option<(usize, usize)> = None;
    let mut widths: Option<(usize, Vec<u32>)> = None;
    let mut seen = BTreeSet::new();
    for (l, name, value) in &d.options {
        if !seen.insert(name.as_str()) {
            out.push((*l, format!("the option `:{name}:` is given twice")));
            continue;
        }
        match name.as_str() {
            "header-rows" => match value.parse::<usize>() {
                Ok(h) => header_rows = Some((*l, h)),
                Err(_) => out.push((
                    *l,
                    format!("`:header-rows:` takes a non-negative integer, not `{value}`"),
                )),
            },
            "widths" if value == "auto" => {}
            "widths" => {
                let parsed: Option<Vec<u32>> = value
                    .split_whitespace()
                    .map(|w| w.parse::<u32>().ok().filter(|v| *v > 0))
                    .collect();
                match parsed {
                    Some(v) if !v.is_empty() => widths = Some((*l, v)),
                    _ => out.push((
                        *l,
                        format!(
                            "`:widths:` takes `auto` or positive integers separated by spaces, not `{value}`"
                        ),
                    )),
                }
            }
            _ => out.push((
                *l,
                format!(
                    "the option `:{name}:` is not allowed on a list-table; only `:header-rows:` and `:widths:` are"
                ),
            )),
        }
    }
    if let Some(u) = d.unseparated {
        out.push((
            u,
            "a blank line must separate the list-table's title and options from its rows"
                .to_string(),
        ));
    }
    if d.body.is_empty() {
        out.push((d.line, "the list-table has no rows".to_string()));
        return out;
    }
    let b = indent_of(src.text(d.body.start));
    if let Some((l, _, _)) = d.options.iter().find(|o| indent_of(src.text(o.0)) != b) {
        out.push((
            *l,
            format!(
                "the options and the rows start at the same column; the rows start at column {}",
                b + 1
            ),
        ));
    }
    let mut rows: Vec<(usize, usize)> = Vec::new();
    for l in d.body.clone() {
        let t = src.text(l);
        if is_blank(t) {
            continue;
        }
        let ind = indent_of(t);
        let rest = &t[ind..];
        for marker in ["* -", "-"] {
            if (ind == b && marker == "* -" || ind == b + 2 && marker == "-")
                && has_marker(rest, marker)
                && rest[marker.len()..].starts_with([' ', '\t'])
                && rest[marker.len() + 1..].starts_with([' ', '\t'])
            {
                out.push((
                    l,
                    format!(
                        "one space separates `{marker}` from the cell text, which starts at column {}",
                        b + 5
                    ),
                ));
            }
        }
        if ind == b {
            if has_marker(rest, "* -") {
                rows.push((l, 1));
            } else {
                out.push((
                    l,
                    format!("a table row starts with `* -` at column {}", b + 1),
                ));
            }
        } else if ind == b + 2 {
            match rows.last_mut() {
                Some(row) if has_marker(rest, "-") => row.1 += 1,
                Some(_) => out.push((
                    l,
                    format!(
                        "a further cell starts with `-` at column {}; cell text continues at column {} or deeper",
                        b + 3,
                        b + 5
                    ),
                )),
                None => out.push((l, format!("a table row starts with `* -` at column {}", b + 1))),
            }
        } else if ind >= b + 4 {
            if rows.is_empty() {
                out.push((
                    l,
                    format!("a table row starts with `* -` at column {}", b + 1),
                ));
            }
        } else {
            out.push((
                l,
                format!(
                    "a table line starts at column {} (a row), {} (a further cell) or {} and deeper (cell text)",
                    b + 1,
                    b + 3,
                    b + 5
                ),
            ));
        }
    }
    let Some(&(_, columns)) = rows.first() else {
        return out;
    };
    for &(l, cells) in &rows[1..] {
        if cells != columns {
            out.push((
                l,
                format!("this row has {cells} cells; the first row has {columns}"),
            ));
        }
    }
    if let Some((l, h)) = header_rows
        && h > 0
        && h >= rows.len()
    {
        out.push((
            l,
            format!(
                "`:header-rows: {h}` leaves no body row; the table has {} row(s)",
                rows.len()
            ),
        ));
    }
    if let Some((l, w)) = widths
        && w.len() != columns
    {
        out.push((
            l,
            format!(
                "`:widths:` gives {} value(s) for {columns} column(s)",
                w.len()
            ),
        ));
    }
    out
}

fn code_block(d: &Directive) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let arg = d.argument.as_str();
    if arg.is_empty() {
        out.push((
            d.line,
            "a code-block names its language (`.. code-block:: text`); a plain block uses `::`"
                .to_string(),
        ));
    } else if arg.split_whitespace().count() > 1 {
        out.push((
            d.line,
            format!("a code-block takes one language argument, not `{arg}`"),
        ));
    } else if !LANGUAGE_RE.is_match(arg) {
        out.push((
            d.line,
            format!("`{arg}` is not a language name: letters, digits and `_ + . # -`"),
        ));
    }
    for (l, name, _) in &d.options {
        out.push((
            *l,
            format!("a code-block takes no options; `:{name}:` is not allowed"),
        ));
    }
    if let Some(u) = d.unseparated {
        out.push((
            u,
            "a blank line must separate `.. code-block::` from its content".to_string(),
        ));
    } else if d.body.is_empty() {
        out.push((d.line, "the code-block has no content".to_string()));
    }
    out
}

/// The cells of list-table `d`, each as its first line and its lines with the row or
/// cell marker blanked out, so each cell's content can be scanned as ordinary text at
/// the same columns. Lines before the first marker form a cell of their own.
pub fn masked_cells(src: &Source, d: &Directive) -> Vec<(usize, Vec<String>)> {
    let Some(first) = d.body.clone().find(|l| !is_blank(src.text(*l))) else {
        return Vec::new();
    };
    let b = indent_of(src.text(first));
    let mut cells: Vec<(usize, Vec<String>)> = Vec::new();
    for l in d.body.clone() {
        let t = src.text(l);
        let ind = indent_of(t);
        let rest = &t[ind..];
        let marker = if ind == b && has_marker(rest, "* -") {
            3
        } else if ind == b + 2 && has_marker(rest, "-") {
            1
        } else {
            0
        };
        let masked = format!("{}{}{}", &t[..ind], " ".repeat(marker), &rest[marker..]);
        match cells.last_mut() {
            Some(cell) if marker == 0 => cell.1.push(masked),
            _ => cells.push((l, vec![masked])),
        }
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(text: &str) -> Vec<(usize, String)> {
        let src = Source::from_text(text);
        let line = (0..src.len())
            .find(|i| name_at(src.text(*i)).is_some())
            .expect("a directive");
        let d = Directive::read(&src, line).expect("a directive");
        problems(&src, &d)
    }

    fn ok(text: &str) {
        let p = check(text);
        assert!(p.is_empty(), "unexpected problems in {text:?}: {p:?}");
    }

    fn bad(text: &str, needle: &str) {
        let p = check(text);
        assert!(
            p.iter().any(|(_, m)| m.contains(needle)),
            "expected {needle:?} in {text:?}; got {p:?}"
        );
    }

    const TABLE: &str = ".. list-table:: Keys
   :header-rows: 1
   :widths: 30 70

   * - Key
     - Meaning
   * - ``a``
     - The first key, described over
       two lines, with `a link <x.rst>`_.
   * - ``b``
     -
";

    #[test]
    fn admitted_list_tables() {
        ok(TABLE);
        ok(".. list-table::\n   :widths: auto\n\n   * - a\n     - b\n");
        ok(".. list-table::\n\n   * - a\n     - b\n       *Emphasis* opens this line.\n");
        ok("- item\n\n  .. list-table::\n\n     * - a\n");
    }

    #[test]
    fn list_table_options() {
        bad(
            ".. list-table::\n   :stub-columns: 1\n\n   * - a\n",
            "not allowed on a list-table",
        );
        bad(
            ".. list-table::\n   :header-rows: 1\n   :header-rows: 1\n\n   * - a\n   * - b\n",
            "given twice",
        );
        bad(
            ".. list-table::\n   :header-rows: one\n\n   * - a\n",
            "non-negative integer",
        );
        bad(
            ".. list-table::\n   :widths: grid\n\n   * - a\n",
            "`auto` or positive integers",
        );
        bad(
            ".. list-table::\n   :widths: 0 1\n\n   * - a\n     - b\n",
            "`auto` or positive integers",
        );
    }

    #[test]
    fn list_table_structure() {
        bad(
            ".. list-table::\n   * - a\n     - b\n",
            "blank line must separate",
        );
        bad(".. list-table::\n", "has no rows");
        bad(
            ".. list-table::\n\n   * - a\n     - b\n   * - c\n",
            "this row has 1 cells; the first row has 2",
        );
        bad(
            ".. list-table::\n   :header-rows: 2\n\n   * - a\n   * - b\n",
            "leaves no body row",
        );
        bad(
            ".. list-table::\n   :header-rows: 3\n\n   * - a\n   * - b\n",
            "leaves no body row",
        );
        bad(
            ".. list-table::\n   :widths: 1 2 3\n\n   * - a\n     - b\n",
            "3 value(s) for 2 column(s)",
        );
        bad(".. list-table::\n\n   - - a\n", "starts with `* -`");
        bad(
            ".. list-table::\n\n   * - a\n     + b\n",
            "further cell starts with `-`",
        );
        bad(
            ".. list-table::\n\n   * - a\n    - b\n",
            "a table line starts at column",
        );
        bad(
            ".. list-table::\n   :header-rows: 0\n\n    * - a\n",
            "the options and the rows start at the same column",
        );
        bad(
            ".. list-table::\n\n   * -  a\n       b\n",
            "one space separates `* -`",
        );
        bad(
            ".. list-table::\n\n   * - a\n     -  b\n",
            "one space separates `-`",
        );
    }

    #[test]
    fn code_blocks() {
        ok(".. code-block:: rust\n\n   fn main() { let f = |x| x; }\n");
        ok("Term\n  .. code-block:: c++\n\n     int x;\n\n  after\n");
        bad(".. code-block::\n\n   x\n", "names its language");
        bad(".. code-block:: rust c\n\n   x\n", "one language argument");
        bad(".. code-block:: r/s\n\n   x\n", "is not a language name");
        bad(
            ".. code-block:: rust\n   :linenos:\n\n   x\n",
            "takes no options",
        );
        bad(
            ".. code-block:: rust\n   fn main() {}\n",
            "blank line must separate",
        );
        bad(".. code-block:: rust\n\nafter\n", "has no content");
        bad(
            "Term\n\t.. code-block:: text\n\n   x\n",
            "with spaces, not tabs",
        );
        bad(".. list-table::\n\n\t* - a\n", "with spaces, not tabs");
    }

    #[test]
    fn name_at_skips_cell_markers() {
        assert_eq!(
            name_at("   * - .. code-block:: text"),
            Some((7, "code-block".to_string(), "text".to_string()))
        );
        assert_eq!(
            name_at("     - .. list-table::"),
            Some((7, "list-table".to_string(), String::new()))
        );
        assert_eq!(name_at("   * - text"), None);
    }

    #[test]
    fn masking_keeps_columns() {
        let src = Source::from_text(TABLE);
        let d = Directive::read(&src, 0).expect("a directive");
        let m = masked_cells(&src, &d);
        assert_eq!(m.len(), 6);
        assert_eq!((m[0].0, m[0].1[0].as_str()), (4, "       Key"));
        assert_eq!(m[1].1, vec!["       Meaning".to_string()]);
        assert_eq!(m[3].1[1], "       two lines, with `a link <x.rst>`_.");
    }
}
