//! Rewrapping generated documents: paragraphs, lists, definition lists and literal
//! blocks built from words, with arbitrary line breaks. Every pass must keep the
//! scanned structure (the pass checks it and fails otherwise), move only whitespace and
//! be idempotent; with words that wrap freely, no line is over the width except a token
//! that does not fit.

use palette_server::reflow::{WIDTH, reflow_source};
use palette_server::rst::{Doc, Kind};
use palette_server::text::Source;

/// A small deterministic generator, so a failure repeats.
struct Rng(u64);

impl Rng {
    fn next(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) as usize) % n
    }
}

/// Words that wrap freely.
const PLAIN: &[&str] = &[
    "word",
    "the",
    "of",
    "and",
    "a",
    "to",
    "in",
    "§",
    "→",
    "—",
    "``lit eral``",
    "\\*star\\*",
    "`a link <../x/y.rst#z>`_",
    "`two words`",
    "`a b c <p/q.rst>`__",
    "한국어",
    "문장",
    "foo:",
    "e.g.",
    "verylongtokenverylongtokenverylongtokenverylongtokenverylongtokenverylongtoken12345",
];

/// Words that would read as other markup at the start of a line, end a line with a
/// literal introducer or an escape, or open a span that never closes.
const NASTY: &[&str] = &[
    "-",
    "*",
    "+",
    "1.",
    "10.",
    "a)",
    "(i)",
    "I.",
    "mild.",
    "civil.",
    "..",
    "::",
    "|",
    ":x:",
    ":ref:`a b`",
    "----",
    "====",
    "...",
    "i.e.",
    "``code``\\",
    "+---+",
    "|x|",
    "foo::",
    "bar:",
    ">>>",
    "__",
    "_",
    "`",
    "\\",
];

fn words(rng: &mut Rng, count: usize, nasty: bool) -> Vec<&'static str> {
    (0..count)
        .map(|_| {
            if nasty && rng.next(3) == 0 {
                NASTY[rng.next(NASTY.len())]
            } else {
                PLAIN[rng.next(PLAIN.len())]
            }
        })
        .collect()
}

/// Lines of prose, the first starting with `first` (a marker) and the others with
/// `rest`, broken at random places.
fn prose(rng: &mut Rng, nasty: bool, first: &str, rest: &str) -> Vec<String> {
    let count = 1 + rng.next(30);
    let mut lines = Vec::new();
    let mut cur = first.to_string();
    let mut on_line = 0;
    for w in words(rng, count, nasty) {
        if on_line > 0 && rng.next(6) == 0 {
            lines.push(std::mem::replace(&mut cur, rest.to_string()));
            on_line = 0;
        }
        if on_line > 0 {
            cur.push(' ');
        }
        cur.push_str(w);
        on_line += 1;
    }
    lines.push(cur);
    lines
}

fn document(rng: &mut Rng, nasty: bool) -> String {
    let mut out: Vec<String> = vec!["Title".into(), "=====".into(), String::new()];
    for _ in 0..1 + rng.next(8) {
        match rng.next(8) {
            0 => out.extend(prose(rng, nasty, "", "")),
            1 => out.extend(prose(rng, nasty, "- ", "  ")),
            2 => {
                out.extend(prose(rng, nasty, "1. ", "   "));
                if rng.next(2) == 0 {
                    out.extend(prose(rng, nasty, "  - ", "    "));
                }
            }
            3 => {
                out.push("a term".into());
                out.extend(prose(rng, nasty, "  ", "  "));
            }
            4 => {
                let mut p = prose(rng, nasty, "", "");
                if let Some(last) = p.last_mut() {
                    last.push_str(" example::");
                }
                out.extend(p);
                out.push(String::new());
                out.push("  literal text that is kept".into());
                out.push("     as written".into());
            }
            5 => {
                out.push(".. comment".into());
                out.push("   with a body".into());
            }
            6 => {
                out.push("- item".into());
                out.push(String::new());
                out.extend(prose(rng, nasty, "  ", "  "));
                out.push(String::new());
                out.extend(prose(rng, nasty, "  - ", "    "));
            }
            _ => {
                out.push(":Field: a value".into());
                out.push("  continued".into());
            }
        }
        out.push(String::new());
    }
    out.join("\n")
}

fn flat(s: &str) -> String {
    s.split_whitespace().collect()
}

fn check(seed: u64, documents: usize, nasty: bool) {
    let mut rng = Rng(seed);
    let mut wrapped_any = 0;
    for n in 0..documents {
        let text = document(&mut rng, nasty);
        let src = Source::from_text(&text);
        let out = match reflow_source(&src) {
            Ok(o) => o,
            Err(e) => panic!("document {n}: {e}\n{text}"),
        };
        let t = out.to_text();
        assert_eq!(
            flat(&text),
            flat(&t),
            "document {n} changed content\n{text}\n--\n{t}"
        );
        let again = reflow_source(&out).unwrap_or_else(|e| panic!("document {n} again: {e}\n{t}"));
        assert_eq!(
            again.to_text(),
            t,
            "document {n} is not idempotent\n{text}\n--\n{t}"
        );
        if !nasty {
            let doc = Doc::parse(out);
            for (i, l) in t.lines().enumerate() {
                // A line over the width holds a token that does not fit, or is a line
                // the pass leaves alone (a field, a term).
                if doc.kinds[i] == Kind::Text && l.chars().count() > WIDTH {
                    assert!(
                        l.starts_with(':')
                            || l.starts_with("  continued")
                            || l.starts_with("a term")
                            || l.contains('`')
                            || l.split(' ').any(|w| w.chars().count() >= 40),
                        "document {n}, line {i}: {l:?}\n{t}"
                    );
                }
            }
        }
        if t != text {
            wrapped_any += 1;
        }
    }
    assert!(wrapped_any > documents / 30, "{wrapped_any}");
}

#[test]
fn documents_of_free_words_wrap_within_the_width_and_idempotently() {
    check(0x5eed, 3000, false);
}

#[test]
fn documents_with_words_that_read_as_markup_keep_their_structure_and_content() {
    check(0x1234_abcd, 3000, true);
}
