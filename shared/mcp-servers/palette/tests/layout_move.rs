//! `palette_layout_set` refuses a move that strands a changeset target (RFC-0024, B-8):
//! a target that a changeset may edit today but not under the new placement, and a move
//! while a changeset cannot be read. The refusal comes before any file moves, so the
//! layout, the sources and the mirrors stay byte for byte as they were. Every case is
//! covered for the changesets of an Accepted (RFC-0002), a Draft (RFC-0003) and a
//! Proposed (RFC-0004) record.

mod common;

use common::*;
use palette_server::errors::ErrCode;
use palette_server::ops::{init, records};
use serde_json::json;

/// The records whose changesets the tests strand: Accepted, Draft, Proposed.
const RECORDS: [&str; 3] = ["RFC-0002", "RFC-0003", "RFC-0004"];

fn move_family(p: &Proj, family: &str, placement: &str) -> ToolResult {
    init::layout_set(
        &p.ctx,
        params(
            p,
            json!({"family": family, "placement": placement, "confirmed_by_user": true}),
        ),
    )
}

fn edit(p: &Proj, record: &str, document: &str, target: &str) {
    records::changeset_edit(
        &p.ctx,
        params(
            p,
            json!({
                "record": record, "action": "add", "kind": "replace_body", "document": document,
                "target": target, "body": format!("{target}\n{}\n\nNew text.", "^".repeat(target.len()))
            }),
        ),
    )
    .unwrap_or_else(|e| panic!("edit {record} {document}: {}", e.message));
}

fn doc(title: &str, section: &str) -> String {
    format!(
        "{title}\n{}\n\n{section}\n{}\n\nText.\n",
        "=".repeat(title.len()),
        "-".repeat(section.len())
    )
}

/// RFC-0004, a Proposed record (RFC-0002 is Accepted and RFC-0003 a Draft in the fixture).
fn propose_rfc4(p: &Proj) {
    records::record_create(
        &p.ctx,
        params(
            p,
            json!({"kind": "rfc", "title": "Epsilon", "authors": "A", "areas": "thing", "description": "Plans."}),
        ),
    )
    .expect("create RFC-0004");
    records::record_update(
        &p.ctx,
        params(p, json!({"record": "RFC-0004", "status": "Proposed"})),
    )
    .expect("propose RFC-0004");
}

/// The valid fixture with one document under `docs/` for each record, written at
/// `paths`, and a changeset on each; plain `.rst` documents the families do not hold.
fn project_with(paths: [&str; 3]) -> Proj {
    let p = Proj::valid();
    propose_rfc4(&p);
    for (record, path) in RECORDS.iter().zip(paths) {
        p.write(path, &doc(path, "Scope"));
        edit(&p, record, path, "Scope");
    }
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
    p
}

/// Moves `family` to `placement`, which must be refused, and returns the message; the
/// refusal leaves every file as it was.
fn refused(p: &Proj, family: &str, placement: &str) -> String {
    let before = p.snapshot();
    let err = move_family(p, family, placement).expect_err("the move strands a target");
    assert_eq!(err.code, ErrCode::InvariantViolation, "{}", err.message);
    assert_eq!(
        p.snapshot(),
        before,
        "a refusal leaves every file as it was"
    );
    err.message
}

/// `msg` names `<record>: <path>` for each record and path.
fn names_all(msg: &str, paths: [&str; 3]) {
    for (record, path) in RECORDS.iter().zip(paths) {
        assert!(
            msg.contains(&format!("{record}: {path}")),
            "{record}: {path}\n{msg}"
        );
    }
}

const GUIDES: [&str; 3] = [
    "docs/guide/coding-style.rst",
    "docs/guide/other.rst",
    "docs/guide/third.rst",
];

#[test]
fn a_move_of_the_document_root_names_every_stranded_target_of_every_status() {
    let p = project_with(GUIDES);
    // Staging internal: the document root becomes `_palette/`, outside which no project
    // document is a target.
    let msg = refused(&p, "staging", "internal");
    names_all(&msg, GUIDES);
    assert!(
        msg.contains("outside the document root `_palette`"),
        "{msg}"
    );
    assert!(
        p.read("_palette/layout.rst")
            .contains(":staging: docs/staging")
    );
    assert!(p.exists("docs/staging/spec/thing.rst"));
}

#[test]
fn a_target_that_would_lie_inside_the_new_staging_folder_is_stranded() {
    // Every document under the staging folder is a generated mirror.
    let p = project_with(GUIDES);
    let msg = refused(&p, "staging", "docs/guide");
    names_all(&msg, GUIDES);
    assert!(
        msg.contains("is a staging document in `docs/guide`"),
        "{msg}"
    );
}

#[test]
fn only_the_targets_the_move_strands_are_named() {
    let p = project_with(GUIDES);
    p.write("docs/notes/plan.rst", &doc("Plan", "Steps"));
    edit(&p, "RFC-0002", "docs/notes/plan.rst", "Steps");
    let msg = refused(&p, "staging", "docs/guide");
    assert!(!msg.contains("docs/notes/plan.rst"), "{msg}");
    assert!(
        msg.contains("RFC-0002: docs/guide/coding-style.rst"),
        "{msg}"
    );
    // Several targets of one changeset are each named.
    let msg = refused(&p, "staging", "internal");
    assert!(
        msg.contains("RFC-0002: docs/guide/coding-style.rst")
            && msg.contains("RFC-0002: docs/notes/plan.rst"),
        "{msg}"
    );
}

#[test]
fn a_mirror_that_would_take_a_reserved_mirrors_place_is_stranded() {
    let design = [
        "docs/design/a.rst",
        "docs/design/b.rst",
        "docs/design/c.rst",
    ];
    // With the staging folder at the top the document root is the project folder, and
    // with the design family elsewhere `docs/design/` is an ordinary folder. Under
    // `docs/` as the document root the mirrors would be `design/a.rst` and so on, in the
    // design mirror folder of the new staging folder.
    let p = Proj::valid();
    move_family(&p, "staging", "staging").expect("staging to the top");
    move_family(&p, "design", "docs/architecture").expect("design elsewhere");
    propose_rfc4(&p);
    for (record, path) in RECORDS.iter().zip(design) {
        p.write(path, &doc(path, "Scope"));
        edit(&p, record, path, "Scope");
    }
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
    let msg = refused(&p, "staging", "docs/staging");
    names_all(&msg, design);
    assert!(msg.contains("which mirrors the design family"), "{msg}");

    // A single-file mirror: `docs/glossary.rst` would mirror at `glossary.rst`, the
    // glossary's own mirror.
    let single = [
        "docs/glossary.rst",
        "docs/principles.rst",
        "docs/contributing.rst",
    ];
    let p = Proj::valid();
    move_family(&p, "staging", "staging").expect("staging to the top");
    for family in ["glossary", "principles", "contributing"] {
        move_family(&p, family, "internal").expect("internal");
    }
    propose_rfc4(&p);
    for (record, path) in RECORDS.iter().zip(single) {
        p.write(path, &doc(path, "Scope"));
        edit(&p, record, path, "Scope");
    }
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
    let msg = refused(&p, "staging", "docs/staging");
    names_all(&msg, single);
    assert!(msg.contains("the mirror of the glossary document"), "{msg}");
    assert!(
        msg.contains("the mirror of the principles document"),
        "{msg}"
    );
    assert!(
        msg.contains("the mirror of the contributing document"),
        "{msg}"
    );
}

#[test]
fn a_move_that_puts_a_family_over_a_target_is_stranded_too() {
    // The check is the parser's own rules against the new placement, whichever family
    // moves: `docs/guide/` would hold the records.
    let p = project_with(GUIDES);
    let msg = refused(&p, "rfc", "docs/guide");
    names_all(&msg, GUIDES);
    assert!(msg.contains("is a record or index"), "{msg}");
}

#[test]
fn a_changeset_that_cannot_be_read_stops_every_move() {
    // One unreadable changeset per record, whatever the record's status.
    for record in RECORDS {
        let p = project_with(GUIDES);
        let file = format!("docs/changeset/{}.rst", record.to_lowercase());
        std::fs::write(p.path(&file), b"Changeset: x\n\xff\xfe\n").expect("write");
        // Moving the changeset family itself used to reach the file's `parse_error`; the
        // preflight names the changeset first.
        for (family, placement) in [
            ("staging", "docs/stage"),
            ("glossary", "internal"),
            ("changeset", "docs/changesets"),
        ] {
            let msg = refused(&p, family, placement);
            assert!(
                msg.contains(&format!("{record}: unreadable")),
                "{record}: {msg}"
            );
        }
    }
}

#[test]
fn a_move_that_strands_nothing_proceeds() {
    let p = project_with(GUIDES);
    // The document root stays `docs/`, so every target stays valid; the mirrors move.
    move_family(&p, "staging", "docs/stage").expect("staging next door");
    assert!(p.exists("docs/stage/spec/thing.rst") && !p.exists("docs/staging"));
    assert!(
        p.read("_palette/layout.rst")
            .contains(":staging: docs/stage")
    );
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
    // An unrelated family moves with changesets on non-family targets present.
    move_family(&p, "glossary", "internal").expect("glossary");
    assert!(p.exists("_palette/glossary.rst"));
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
    // And back: the check passes while every target is still valid.
    move_family(&p, "staging", "docs/staging").expect("staging back");
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
}

#[test]
fn a_dry_run_is_refused_too() {
    let p = project_with(GUIDES);
    let before = p.snapshot();
    let err = init::layout_set(
        &p.ctx,
        params(
            &p,
            json!({"family": "staging", "placement": "internal", "confirmed_by_user": true, "dry_run": true}),
        ),
    )
    .expect_err("dry run");
    assert_eq!(err.code, ErrCode::InvariantViolation);
    assert_eq!(p.snapshot(), before);
}

#[test]
fn a_target_that_is_already_refused_does_not_block_a_move() {
    // The move is not what strands it: the lint already reports it.
    let p = project_with(GUIDES);
    p.mutate("docs/changeset/rfc-0002.rst", |s| {
        format!("{s}\nREADME.rst\n----------\n\nDelete: Build\n~~~~~~~~~~~~~\n")
    });
    move_family(&p, "staging", "docs/stage").expect("staging next door");
    assert!(p.exists("docs/stage/spec/thing.rst"));
}

#[test]
fn crlf_projects_are_refused_the_same_way() {
    let p = Proj::valid_crlf();
    p.write(
        GUIDES[0],
        &doc("Coding style", "Scope").replace('\n', "\r\n"),
    );
    edit(&p, "RFC-0002", GUIDES[0], "Scope");
    let msg = refused(&p, "staging", "internal");
    assert!(msg.contains(&format!("RFC-0002: {}", GUIDES[0])), "{msg}");
}
