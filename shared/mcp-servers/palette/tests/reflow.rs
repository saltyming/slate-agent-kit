//! Rewrapping prose at 79 columns (RFC-0024, B-13): the `reflow` module on the
//! house-style constructs, the write tools wrapping the text they receive, and the
//! `palette reflow` operation over a project.

mod common;

use common::*;
use palette_server::edit::WRAP_COLUMN;
use palette_server::ops::{self, records};
use palette_server::reflow::{WIDTH, reflow_source, shape, wrap_fragment};
use palette_server::rst::{Doc, Kind};
use palette_server::text::Source;
use serde_json::json;

const LONG: &str = "far longer than the width of seventy-nine columns";

/// A document with every construct the house style has, around prose that must wrap.
fn house() -> String {
    let mut s = String::new();
    let mut add = |block: &str| {
        s.push_str(block);
        s.push('\n');
    };
    add(
        "Wide title that is much longer than the width so that nobody could wrap it by accident\n===========================================================================================\n",
    );
    add(&format!(
        ":Status: Maintained\n:Description: A header value {LONG} that continues\n  on an indented line of the same value.\n:Date: 2026-10-14\n"
    ));
    add("Prose\n-----\n");
    add(&format!(
        "A paragraph on one line that is {LONG} and has to be wrapped by the pass, with ``an inline literal with spaces`` kept whole and `a link <../spec/some/long/path/to-a-document.rst#a-section>`_ as one token.\n"
    ));
    add(&format!(
        "Escaped \\*markup\\* and a trailing escaped space ``code``\\ s stay as written, in a sentence that is {LONG} and wraps in the middle.\n"
    ));
    add(
        "AVeryLongTokenThatHasNoSpacesAndIsLongerThanTheSeventyNineColumnsOfTheWidthSoItStandsAloneOnItsLine and then some words.\n",
    );
    add(&format!(
        "A paragraph whose link is split over lines, `the alpha\ncontract <../spec/a-very/long-\npath/to-the-alpha-contract.rst#section>`_ and more text that is {LONG}.\n"
    ));
    add(&format!(
        "- A bullet that is {LONG} and continues over several lines when wrapped, indented to its text.\n  - A nested tight bullet that is also {LONG} and is its own item.\n\n  Parent continuation after nested content: a paragraph of the first bullet, {LONG}.\n"
    ));
    add(&format!(
        "1. An enumerated item that is {LONG} and continues over several lines when wrapped.\n   Second line of the same item.\n10. A two-digit marker item that is {LONG} and continues over several lines.\n"
    ));
    add(&format!(
        "Term kept as written even though it is very long indeed, {LONG}\n  The body of the definition, which is {LONG} and is wrapped at its own indent.\n\nshort term\n  Short body.\n"
    ));
    add(&format!(
        "Example introduced inside a paragraph that is {LONG}::\n\n  literal block kept\n     exactly as written, even when its lines are {LONG}\n"
    ));
    add(&format!(
        "- Item that introduces a literal block and is {LONG}::\n\n      literal in a list kept as written, {LONG} in a line\n"
    ));
    add(&format!(
        ".. a comment that is {LONG} and is kept as written\n   with a body that is also kept as written, {LONG}.\n"
    ));
    add("..\n");
    add(&format!(
        "Another paragraph after a bare comment that is {LONG} and wraps.\n"
    ));
    add(&format!(
        ".. code-block:: text\n\n   code {LONG} is kept as written inside the directive extent\n"
    ));
    add(&format!(
        ".. list-table:: A title that is {LONG} and kept as written too\n   :header-rows: 1\n   :widths: 30 70\n\n   * - Key\n     - Meaning that is {LONG} and kept as written in the cell\n   * - ``a``\n     - A cell with a link `x <../a.rst>`_ and a nested\n\n       .. code-block:: text\n\n          cell code {LONG} kept as written\n"
    ));
    add(&format!(
        "+---------------+-----------------------------------------------------------------+\n| grid table    | cell {LONG} kept as written                   |\n+---------------+-----------------------------------------------------------------+\n"
    ));
    add(&format!(
        "=====  ======\nName   Value {LONG} kept as written because it is a table\n=====  ======\n"
    ));
    add("----\n");
    add("Last paragraph.");
    s
}

/// The lines whose bytes the pass must keep: everything that is not prose text.
fn untouched(text: &str) -> Vec<String> {
    let doc = Doc::parse(Source::from_text(text));
    (0..doc.src.len())
        .filter(|l| doc.kinds[*l] != Kind::Text)
        .map(|l| doc.src.text(l).to_string())
        .collect()
}

fn flat(text: &str) -> String {
    text.split_whitespace().collect()
}

fn flow_text(text: &str) -> String {
    reflow_source(&Source::from_text(text))
        .unwrap_or_else(|e| panic!("{e}"))
        .to_text()
}

#[test]
fn the_house_style_constructs_keep_their_bytes_and_structure() {
    let text = house();
    let src = Source::from_text(&text);
    let out = reflow_source(&src).expect("structure kept");
    let wrapped = out.to_text();
    assert_ne!(wrapped, text);

    // Untouched bytes outside the prose: every other line, in order.
    assert_eq!(untouched(&text), untouched(&wrapped));
    // The scanned structure is the same, and a second pass finds nothing to do.
    assert_eq!(shape(&src), shape(&out));
    assert_eq!(flow_text(&wrapped), wrapped, "idempotent");
    // Only whitespace between words moved.
    assert_eq!(flat(&text), flat(&wrapped));

    // Kept as written, though far over the width: the title, the field list, a
    // definition term, the literal blocks, the comment, the directives and the tables.
    for kept in [
        "Wide title that is much longer than the width so that nobody could wrap it by accident",
        &format!(":Description: A header value {LONG} that continues"),
        "  on an indented line of the same value.",
        &format!("Term kept as written even though it is very long indeed, {LONG}"),
        "     exactly as written, even when its lines are far longer than the width of seventy-nine columns",
        &format!("      literal in a list kept as written, {LONG} in a line"),
        &format!(".. a comment that is {LONG} and is kept as written"),
        &format!("   with a body that is also kept as written, {LONG}."),
        &format!("   code {LONG} is kept as written inside the directive extent"),
        &format!("     - Meaning that is {LONG} and kept as written in the cell"),
        &format!("          cell code {LONG} kept as written"),
        &format!("| grid table    | cell {LONG} kept as written                   |"),
        &format!("Name   Value {LONG} kept as written because it is a table"),
        "..\n",
        "----\n",
    ] {
        assert!(wrapped.contains(kept), "{kept:?} missing from\n{wrapped}");
    }

    // Prose is at most 79 columns, except a token that stands alone.
    let long_token = "AVeryLongTokenThatHasNoSpacesAndIsLongerThanTheSeventyNineColumnsOfTheWidthSoItStandsAloneOnItsLine";
    assert!(wrapped.lines().any(|l| l == long_token), "{wrapped}");
    let doc = Doc::parse(out.clone());
    let kept_long = untouched(&wrapped);
    for (i, l) in wrapped.lines().enumerate() {
        if doc.kinds[i] == Kind::Text && l != long_token && !kept_long.contains(&l.to_string()) {
            let standalone =
                !l.trim().contains(' ') || (l.trim().starts_with('`') && l.ends_with("`_"));
            let in_table = standalone
                || l.starts_with('|')
                || l.starts_with('+')
                || l.starts_with("Name ")
                || l.starts_with("     - Meaning")
                || l.starts_with("Term kept")
                || l.starts_with("  on an indented")
                || l.starts_with(":Description:");
            assert!(in_table || l.chars().count() <= WIDTH, "line {i}: {l:?}");
        }
    }

    // A term's body is wrapped at its own indent, a nested item at its own, a parent's
    // continuation after nested content stays in the parent.
    assert!(
        wrapped.contains("\n  The body of the definition, which is"),
        "{wrapped}"
    );
    assert!(
        wrapped.contains("\n  - A nested tight bullet that is also"),
        "{wrapped}"
    );
    assert!(
        wrapped.contains("\n  Parent continuation after nested content: a paragraph"),
        "{wrapped}"
    );
    // The enumerated item is refilled to its text column (three and four spaces).
    assert!(
        wrapped.contains("\n1. An enumerated item that is"),
        "{wrapped}"
    );
    assert!(
        wrapped.contains("\n10. A two-digit marker item"),
        "{wrapped}"
    );
    // The introducer sentence keeps its `::` at its end, attached to the block.
    assert!(
        wrapped.lines().any(|l| l.ends_with("columns::")
            || l.ends_with("wide::")
            || l.ends_with("it::")
            || l.ends_with("digits::")),
        "{wrapped}"
    );
}

#[test]
fn a_link_split_over_lines_is_rejoined_without_a_space_and_escapes_are_kept() {
    let wrapped = flow_text(&house());
    assert!(
        wrapped.contains(
            "`the alpha contract <../spec/a-very/long-path/to-the-alpha-contract.rst#section>`_"
        ),
        "{wrapped}"
    );
    assert!(wrapped.contains("\\*markup\\*"), "{wrapped}");
    assert!(wrapped.contains("``code``\\ s"), "{wrapped}");
    assert!(
        wrapped.contains("``an inline literal with spaces``"),
        "{wrapped}"
    );
    assert!(
        wrapped.contains("`a link <../spec/some/long/path/to-a-document.rst#a-section>`_"),
        "{wrapped}"
    );
}

#[test]
fn crlf_documents_keep_crlf() {
    let lf = house();
    let crlf = lf.replace('\n', "\r\n");
    let out = reflow_source(&Source::from_text(&crlf))
        .expect("crlf")
        .to_text();
    assert!(
        !out.replace("\r\n", "").contains('\n'),
        "a bare LF crept in"
    );
    assert_eq!(out.replace("\r\n", "\n"), flow_text(&lf));
    // A file without a final newline keeps none.
    let text = format!("{}\nno newline at the end", "word ".repeat(30).trim_end());
    let out = flow_text(&text);
    assert!(
        !out.ends_with('\n') && out.ends_with("at the end"),
        "{out:?}"
    );
}

#[test]
fn exact_wrapping_of_items_paragraphs_and_definition_bodies() {
    let words = "word ".repeat(30);
    let words = words.trim();
    let fifteen = vec!["word"; 15].join(" ");
    // A bullet: fifteen words fit after `- ` at 79 columns, and the text column is two.
    assert_eq!(
        flow_text(&format!("- {words}\n")),
        format!("- {fifteen}\n  {fifteen}\n")
    );
    // A paragraph: fifteen words fit in 79 columns, the sixteenth would be 80.
    let sixteen = vec!["word"; 16].join(" ");
    assert_eq!(sixteen.len(), 79);
    assert_eq!(
        flow_text(&format!("{words}\n")),
        format!("{sixteen}\n{}\n", vec!["word"; 14].join(" "))
    );
    // A definition body at its indent.
    let body = vec!["word"; 15].join(" ");
    assert_eq!(
        flow_text(&format!("term\n  {words}\n")),
        format!("term\n  {body}\n  {body}\n")
    );
    assert_eq!(WIDTH, 79);
    assert_eq!(WRAP_COLUMN, WIDTH);
}

#[test]
fn a_line_never_begins_where_it_would_read_as_other_markup() {
    let pad = "w".repeat(70);
    for lead in [
        "-", "1.", "(a)", "..", "mild.", "----", "|", ":x:", "+--", "::",
    ] {
        let text = format!("{pad} {lead} tail\n");
        let out = flow_text(&text);
        assert_eq!(flat(&out), flat(&text));
        for (i, l) in out.lines().enumerate().skip(1) {
            let first = l.split(' ').next().unwrap_or("");
            assert_ne!(first, lead, "{lead}: line {i} of {out:?}");
        }
        // The pass kept the structure: still one block of text, no new title or list.
        let doc = Doc::parse(Source::from_text(&out));
        assert!(doc.headings.is_empty(), "{lead}: {out}");
    }
}

#[test]
fn tabs_tables_and_fields_in_prose_runs_are_left_alone() {
    let tabbed = format!("\t{}\n", "word ".repeat(30));
    assert_eq!(flow_text(&tabbed), tabbed);
    let table = format!("+----+\n| {LONG} |\n+----+\ntext {LONG} {LONG}\n");
    assert_eq!(flow_text(&table), table);
    let field = format!(":Name: {LONG} {LONG} {LONG}\n");
    assert_eq!(flow_text(&field), field);
}

#[test]
fn a_literal_block_stays_literal_when_its_introducer_moves() {
    // The introducer is the item's first line; wrapped, it ends the second line, whose
    // indent is the text column. The block is indented deeper than that, so it stays.
    let text = format!("- Item that introduces a literal block and is {LONG}::\n\n      code\n");
    let out = flow_text(&text);
    let doc = Doc::parse(Source::from_text(&out));
    assert!(doc.kinds.contains(&Kind::Literal), "{out}");
    // A block at the item's own column would stop being literal: the item is left as is.
    let text = format!("- Item that introduces a literal block and is {LONG}::\n\n  code\n");
    assert_eq!(flow_text(&text), text);
}

#[test]
fn a_fragment_is_wrapped_before_a_write_tool_splices_it() {
    let lines: Vec<String> =
        format!("A paragraph {LONG} and more words, again {LONG} and again {LONG}.")
            .split('\n')
            .map(String::from)
            .collect();
    let out = wrap_fragment(&lines);
    assert!(
        out.len() > 1 && out.iter().all(|l| l.chars().count() <= WIDTH),
        "{out:?}"
    );
    assert_eq!(wrap_fragment(&out), out, "idempotent");
    let titled: Vec<String> = ["Title", "^^^^^", "", "text"].map(String::from).to_vec();
    assert_eq!(wrap_fragment(&titled), titled);
}

// ── the write tools ─────────────────────────────────────────────────────

/// Every prose line of `text` is at most 79 columns; header and item fields, which a
/// pass never touches, may be longer.
fn assert_wrapped(text: &str) {
    for (i, l) in text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.starts_with(':'))
    {
        assert!(
            l.chars().count() <= WIDTH,
            "line {} is {} columns: {l}",
            i + 1,
            l.chars().count()
        );
    }
}

#[test]
fn record_sections_are_stored_wrapped() {
    let p = Proj::valid();
    let long = format!(
        "A section paragraph {LONG}, stated as one line the way a caller supplies it, and then continued with {LONG}."
    );
    records::record_create(
        &p.ctx,
        params(
            &p,
            json!({
                "kind": "rfc", "title": "Epsilon", "authors": "A", "areas": "thing",
                "description": format!("One sentence that is {LONG} and is stored as a header value {LONG}."),
                "sections": {
                    "Summary": long,
                    "Design": format!("- A bullet {LONG} {LONG}\n\nExample::\n\n  literal {LONG} {LONG} {LONG}\n"),
                }
            }),
        ),
    )
    .expect("create");
    let text = p.read("docs/rfc/rfc-0004-epsilon.rst");
    let summary = text.split("Summary\n-------\n\n").nth(1).expect("summary");
    let summary = summary.split("\n\n").next().expect("paragraph");
    assert!(summary.lines().count() > 1, "{summary}");
    assert_wrapped(summary);
    assert!(text.contains("\n  literal "), "the literal block is kept");
    assert!(
        text.contains(&format!("  literal {LONG} {LONG} {LONG}\n")),
        "{text}"
    );
    for l in text.lines().filter(|l| !l.starts_with("  literal")) {
        assert!(l.chars().count() <= WIDTH, "{l}");
    }
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));

    // An update wraps the same way.
    records::record_update(
        &p.ctx,
        params(
            &p,
            json!({"record": "RFC-0004", "sections": {"Summary": format!("{long} {long}")}}),
        ),
    )
    .expect("update");
    let text = p.read("docs/rfc/rfc-0004-epsilon.rst");
    let summary = text.split("Summary\n-------\n\n").nth(1).expect("summary");
    assert_wrapped(summary.split("\n\n").next().expect("paragraph"));
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
}

#[test]
fn a_changeset_edit_body_is_stored_and_staged_wrapped_and_still_promotes() {
    let p = Proj::valid();
    let body = format!(
        "Rules\n^^^^^\n\nThe beta rules apply to every operation, which is {LONG}, and to every caller {LONG}."
    );
    records::changeset_edit(
        &p.ctx,
        params(
            &p,
            json!({"record": "RFC-0002", "action": "replace", "kind": "insert_into", "document": "spec/thing.rst", "target": "Contract", "body": body}),
        ),
    )
    .expect("edit");
    let cs = p.read("docs/changeset/rfc-0002.rst");
    assert_wrapped(&cs);
    assert!(
        cs.contains("The beta rules apply to every operation, which is"),
        "{cs}"
    );
    assert_wrapped(&p.read("docs/staging/spec/thing.rst"));
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
    records::changeset_promote(
        &p.ctx,
        params(
            &p,
            json!({"record": "RFC-0002", "implementation": "complete", "implementers": "S", "verification": "static", "verification_note": "n"}),
        ),
    )
    .expect("promote");
    assert!(
        flat(&p.read("docs/spec/thing.rst")).contains("Thebetarulesapplytoeveryoperation,whichis")
    );
    assert_wrapped(&p.read("docs/spec/thing.rst"));
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
}

#[test]
fn work_documents_wrap_at_79() {
    let p = Proj::valid();
    let added = |diff: &str| -> String {
        diff.lines()
            .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
            .map(|l| &l[1..])
            .collect::<Vec<_>>()
            .join("\n")
    };
    let b = ops::backlog::backlog_add(
        &p.ctx,
        params(
            &p,
            json!({"title": "Wrapped", "type": "chore", "source": "user", "body": format!("A body that is {LONG} and is written the way a caller supplies it, in one line {LONG}")}),
        ),
    )
    .expect("add");
    let s = ops::state::state_record(
        &p.ctx,
        params(
            &p,
            json!({"kind": "decision", "text": format!("A decision that is {LONG} and more {LONG}"), "source": "User", "target": "RFC-0004"}),
        ),
    )
    .expect("record");
    let (b, s) = (added(&b.diff), added(&s.diff));
    assert!(b.lines().count() > 8 && s.lines().count() > 2, "{b}\n{s}");
    assert_wrapped(&b);
    assert_wrapped(&s);
}

// ── palette reflow ──────────────────────────────────────────────────────

/// The valid fixture with long one-line paragraphs in a maintained document, a record,
/// and an accepted changeset that holds every edit kind.
fn long_project(p: &Proj) {
    let long = format!(
        "This paragraph is {LONG}, and it is written on one line the way a caller supplies it, {LONG}."
    );
    let eol = if p.read("docs/changeset/rfc-0002.rst").contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    p.replace("docs/spec/thing.rst", "The thing has one operation.", &long);
    p.replace(
        "docs/spec/thing.rst",
        "The compatibility of the thing.",
        &long,
    );
    p.replace(
        "docs/spec/thing.rst",
        "The definitions and model of the thing.",
        &format!("{long}\n\nOld notes\n~~~~~~~~~\n\n{long}").replace('\n', eol),
    );
    p.replace(
        "docs/rfc/rfc-0002-beta.rst",
        "Beta rules apply to every operation of the thing.",
        &long,
    );
    // A bullet list with a continuation, a definition list and a literal block.
    p.mutate("docs/glossary.rst", |s| {
        format!("{s}{eol}long term{eol}  {long} {long}{eol}")
    });
    // The accepted changeset of RFC-0002, written by hand with one-line paragraphs: an
    // edit of every kind.
    let heading =
        |text: &str, rule: char| format!("{text}\n{}\n\n", rule.to_string().repeat(text.len()));
    let section =
        |title: &str, text: &str| format!("{title}\n{}\n\n{text}\n\n", "^".repeat(title.len()));
    let mut cs = heading("Changeset: RFC-0002", '=') + &heading("spec/thing.rst", '-');
    cs += &(heading("Replace: Contract", '~') + &section("Contract", &long));
    cs += &(heading("Insert into: Contract", '~') + &section("Rules", &long));
    cs += &(heading("Insert after: Rules", '~') + &section("Extra rules", &long));
    cs += &(heading("Replace body: Compatibility", '~')
        + &section("Compatibility", &format!("{long} {long}")));
    cs += &heading("Delete: Old notes", '~');
    cs += &(heading("spec/extra.rst", '-')
        + &heading("Create: Extra", '~')
        + ":Status: Contract\n\n");
    for t in [
        "Scope and authority",
        "Definitions and model",
        "Contract",
        "Errors and edge cases",
        "Ownership and ordering",
        "Compatibility",
        "Conformance",
        "References",
    ] {
        cs += &section(t, &long);
    }
    p.write(
        "docs/changeset/rfc-0002.rst",
        &format!("{}\n", cs.trim_end()).replace('\n', eol),
    );
    ops::generate(&p.ctx, &p.arg()).expect("generate staging");
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
}

fn reflow(p: &Proj, paths: &[&str]) -> ToolResult {
    let paths: Vec<String> = paths.iter().map(|s| s.to_string()).collect();
    ops::reflow::reflow(&p.ctx, &p.arg(), &paths)
}

#[test]
fn reflowing_accepted_changesets_and_documents_with_pending_edits_still_promotes() {
    for p in [Proj::valid(), Proj::valid_crlf()] {
        long_project(&p);
        let crlf = p.read("docs/spec/thing.rst").contains("\r\n");
        let before = p.snapshot();

        let r = reflow(&p, &[]).expect("reflow");
        let files: Vec<&str> = r.files.iter().map(|(f, _)| f.as_str()).collect();
        for expected in [
            "docs/changeset/rfc-0002.rst",
            "docs/spec/thing.rst",
            "docs/rfc/rfc-0002-beta.rst",
            "docs/glossary.rst",
            "docs/staging/spec/thing.rst",
            "docs/staging/spec/extra.rst",
        ] {
            assert!(files.contains(&expected), "{expected} not in {files:?}");
        }
        assert!(
            r.files
                .iter()
                .all(|(_, a)| *a == "modified" || *a == "created")
        );
        assert!(!files.iter().any(|f| f.ends_with("index.rst")), "{files:?}");
        assert!(r.message.is_none());
        assert_ne!(p.snapshot(), before);
        for f in &files {
            if f.contains("/staging/") || f.ends_with("extra.rst") {
                continue;
            }
            let text = p.read(f);
            for l in text.lines().filter(|l| !l.starts_with(':')) {
                assert!(l.chars().count() <= WIDTH, "{f}: {l}");
            }
            if crlf {
                assert!(!text.replace("\r\n", "").contains('\n'), "{f} kept CRLF");
            }
        }
        let errors = p.errors();
        assert!(errors.is_empty(), "{}", show(&errors));

        // Generation has nothing to do: the reflow regenerated staging in its transaction.
        let g = ops::generate(&p.ctx, &p.arg()).expect("generate");
        assert!(g.files.is_empty(), "{:?}", g.files);

        // A second reflow finds nothing to do.
        let again = p.snapshot();
        let second = reflow(&p, &[]).expect("reflow again");
        assert!(second.files.is_empty());
        assert_eq!(
            second.message.as_deref(),
            Some(ops::reflow::NOTHING_TO_REFLOW)
        );
        assert_eq!(p.snapshot(), again);

        // Every edit kind still resolves and promotes.
        records::changeset_promote(
            &p.ctx,
            params(
                &p,
                json!({"record": "RFC-0002", "implementation": "complete", "implementers": "S", "verification": "static", "verification_note": "n"}),
            ),
        )
        .expect("promote");
        let thing = p.read("docs/spec/thing.rst").replace("\r\n", "\n");
        assert!(thing.contains("Extra rules\n~~~~~~~~~~~"), "{thing}");
        assert!(!thing.contains("Old notes"), "{thing}");
        assert!(p.exists("docs/spec/extra.rst") && !p.exists("docs/changeset/rfc-0002.rst"));
        let errors = p.errors();
        assert!(errors.is_empty(), "{}", show(&errors));
        let g = ops::generate(&p.ctx, &p.arg()).expect("generate");
        assert!(g.files.is_empty(), "{:?}", g.files);
    }
}

#[test]
fn paths_select_documents_and_unknown_paths_are_refused() {
    let p = Proj::valid();
    long_project(&p);
    let before = p.snapshot();
    let r = reflow(&p, &["docs/glossary.rst"]).expect("one file");
    let files: Vec<&str> = r.files.iter().map(|(f, _)| f.as_str()).collect();
    assert_eq!(files, vec!["docs/glossary.rst"]);
    let after_one = p.snapshot();
    assert_ne!(after_one, before);

    // A folder selects the documents under it.
    let r = reflow(&p, &["docs/spec/"]).expect("folder");
    let files: Vec<&str> = r.files.iter().map(|(f, _)| f.as_str()).collect();
    assert!(
        files.contains(&"docs/spec/thing.rst") && !files.contains(&"docs/rfc/rfc-0002-beta.rst"),
        "{files:?}"
    );

    let snap = p.snapshot();
    for (path, needle) in [
        ("docs/missing.rst", "names no document the layout places"),
        (
            "docs/staging/spec/thing.rst",
            "names no document the layout places",
        ),
        ("docs/rfc/index.rst", "names no document the layout places"),
        ("../x.rst", "project-relative"),
        ("/etc/x.rst", "project-relative"),
        ("", "project-relative"),
    ] {
        let err = reflow(&p, &[path]).expect_err(path);
        assert!(err.message.contains(needle), "{path}: {}", err.message);
        assert_eq!(p.snapshot(), snap, "{path}");
    }
}

#[test]
fn a_document_that_does_not_lint_clean_after_the_pass_blocks_the_reflow() {
    // The transaction lints what it writes: a result with an error writes nothing.
    let p = Proj::valid();
    let long = format!("This paragraph is {LONG}, {LONG}, {LONG}.");
    p.replace("docs/spec/thing.rst", "The thing has one operation.", &long);
    // An existing error in a file the reflow would change blocks it, as for any tool.
    p.mutate("docs/spec/thing.rst", |s| {
        format!("{s}\n.. note:: a directive the house style forbids\n")
    });
    let before = p.snapshot();
    let err = reflow(&p, &["docs/spec/thing.rst"]).expect_err("lint error");
    assert_eq!(
        err.code,
        palette_server::errors::ErrCode::InvariantViolation
    );
    assert_eq!(p.snapshot(), before);
}

#[test]
fn the_server_does_not_expose_reflow_as_a_tool() {
    assert!(
        palette_server::server::tool_annotations()
            .iter()
            .all(|(n, _, _)| !n.contains("reflow"))
    );
}
