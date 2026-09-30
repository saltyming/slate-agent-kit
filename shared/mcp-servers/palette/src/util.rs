//! Small shared helpers: built-in regexes, path text, dates, kebab-case, wrapping.
//!
//! Owns nothing domain-specific. Entry points are the free functions below.

use std::path::{Component, Path, PathBuf};

use regex::Regex;

/// Compiles a built-in pattern. The patterns are constants covered by unit tests,
/// so a failure here is a programming error rather than a runtime condition.
pub fn re(pattern: &str) -> Regex {
    match Regex::new(pattern) {
        Ok(r) => r,
        Err(e) => panic!("invalid built-in pattern {pattern:?}: {e}"),
    }
}

/// Whether `name` is a lowercase ASCII kebab-case file name ending in `.rst`.
pub fn is_kebab_rst(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".rst") else {
        return false;
    };
    is_kebab(stem)
}

/// Whether `s` is lowercase ASCII words joined by single hyphens.
pub fn is_kebab(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && !s.ends_with('-')
        && !s.contains("--")
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Lowercase kebab-case slug of `s`; empty when `s` has no ASCII letters or digits.
pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            if dash && !out.is_empty() {
                out.push('-');
            }
            dash = false;
            out.push(c.to_ascii_lowercase());
        } else {
            dash = true;
        }
    }
    out
}

/// Project-relative path text with `/` separators.
pub fn rel_slash(root: &Path, p: &Path) -> String {
    let rel = p.strip_prefix(root).unwrap_or(p);
    let parts: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    parts.join("/")
}

/// The relative path text (with `/`) that leads from directory `from_dir` to `to`.
/// Both are absolute and lexically normalized.
pub fn relative_link(from_dir: &Path, to: &Path) -> String {
    let a: Vec<Component> = from_dir.components().collect();
    let b: Vec<Component> = to.components().collect();
    let mut i = 0;
    while i < a.len() && i < b.len() && a[i] == b[i] {
        i += 1;
    }
    let mut parts: Vec<String> = Vec::new();
    for _ in i..a.len() {
        parts.push("..".to_string());
    }
    for c in &b[i..] {
        parts.push(c.as_os_str().to_string_lossy().into_owned());
    }
    parts.join("/")
}

/// Lexically resolves `rel` (with `/` separators, possibly `..`) against `base`,
/// without touching the file system. `None` when the result would climb above the
/// root of `base`.
pub fn join_lexical(base: &Path, rel: &str) -> Option<PathBuf> {
    let mut out = base.to_path_buf();
    for part in rel.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if !out.pop() {
                    return None;
                }
            }
            p => out.push(p),
        }
    }
    Some(out)
}

/// Today's date (UTC) as `YYYY-MM-DD`.
pub fn today_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    civil_from_days((secs / 86_400) as i64)
}

/// Civil date for a day count since 1970-01-01 (Howard Hinnant's algorithm).
pub fn civil_from_days(z: i64) -> String {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Whether `s` is an ISO date `YYYY-MM-DD` with a plausible month and day.
pub fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| s[r].chars().all(|c| c.is_ascii_digit());
    if !(digits(0..4) && digits(5..7) && digits(8..10)) {
        return false;
    }
    let m: u32 = s[5..7].parse().unwrap_or(0);
    let d: u32 = s[8..10].parse().unwrap_or(0);
    (1..=12).contains(&m) && (1..=31).contains(&d)
}

/// Splits `text` into words at whitespace, except inside backtick spans (`` `a b` `` and
/// ``` ``a b`` ```), so an inline literal or a hyperlink is never broken across lines.
pub fn split_words(text: &str) -> Vec<String> {
    #[derive(PartialEq)]
    enum Span {
        None,
        Single,
        Double,
    }
    let chars: Vec<char> = text.chars().collect();
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut span = Span::None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            let double = chars.get(i + 1) == Some(&'`');
            match (&span, double) {
                (Span::None, true) => {
                    span = Span::Double;
                    cur.push_str("``");
                    i += 2;
                    continue;
                }
                (Span::Double, true) => {
                    span = Span::None;
                    cur.push_str("``");
                    i += 2;
                    continue;
                }
                (Span::None, false) => span = Span::Single,
                (Span::Single, _) => span = Span::None,
                (Span::Double, false) => {}
            }
        }
        if c.is_whitespace() && span == Span::None {
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
        i += 1;
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

/// Wraps `text` at word boundaries to at most `width` columns per line; the first line
/// starts after `first_prefix_len` characters already present, continuation lines are
/// indented by `indent` spaces.
pub fn wrap_words(text: &str, first_prefix_len: usize, indent: usize, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_len = first_prefix_len;
    for word in split_words(text) {
        let wl = word.chars().count();
        if !cur.is_empty() && cur_len + 1 + wl > width {
            lines.push(std::mem::take(&mut cur));
            cur_len = indent;
        }
        if cur.is_empty() {
            if !lines.is_empty() {
                cur.push_str(&" ".repeat(indent));
            }
            cur.push_str(&word);
            cur_len += wl;
        } else {
            cur.push(' ');
            cur.push_str(&word);
            cur_len += 1 + wl;
        }
    }
    lines.push(cur);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kebab_names() {
        assert!(is_kebab_rst("rfc-0001-foo.rst"));
        assert!(!is_kebab_rst("RFC-0001.rst"));
        assert!(!is_kebab_rst("a--b.rst"));
        assert!(!is_kebab_rst("a_b.rst"));
        assert!(!is_kebab_rst("a.md"));
    }

    #[test]
    fn dates() {
        assert_eq!(civil_from_days(0), "1970-01-01");
        assert_eq!(civil_from_days(20_725), "2026-09-29");
        assert!(is_iso_date("2026-09-29"));
        assert!(!is_iso_date("2026-13-01"));
        assert!(!is_iso_date("26-09-29"));
    }

    #[test]
    fn relative_links() {
        let from = Path::new("/p/_palette");
        let to = Path::new("/p/docs/rfc/a.rst");
        assert_eq!(relative_link(from, to), "../docs/rfc/a.rst");
        assert_eq!(
            relative_link(Path::new("/p/docs"), Path::new("/p/docs/x.rst")),
            "x.rst"
        );
    }

    #[test]
    fn wrapping() {
        let w = wrap_words("aaa bbb ccc ddd", 0, 2, 8);
        assert_eq!(w, vec!["aaa bbb", "  ccc", "  ddd"]);
        assert_eq!(
            split_words("a ``b c`` d `e f <g>`_ h"),
            vec!["a", "``b c``", "d", "`e f <g>`_", "h"]
        );
        assert_eq!(split_words("``a `b c` d`` e"), vec!["``a `b c` d``", "e"]);
        let w = wrap_words("x `a <b.rst>`_ yyyyyyyy", 0, 2, 12);
        assert_eq!(w, vec!["x", "  `a <b.rst>`_", "  yyyyyyyy"]);
    }
}
