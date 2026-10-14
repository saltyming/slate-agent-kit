//! Changeset targets beyond design and spec (RFC-0019): the principles, glossary and
//! contributing documents and other `.rst` documents under the document root are
//! edited, staged and promoted like design and spec documents; every other document is
//! refused with the rule it fails; and `Replace body:` keeps a section's subsections.

mod common;

use common::*;
use palette_server::docs::{Role, Snapshot};
use palette_server::errors::ErrCode;
use palette_server::layout::{Family, Placement};
use palette_server::ops::{self, init, records};
use palette_server::vfs::DiskSource;
use serde_json::json;

const GUIDE: &str = "docs/guide/coding-style.rst";
const GUIDE_MIRROR: &str = "docs/staging/guide/coding-style.rst";

/// A document outside every family, with a grid table that the lint would refuse in a
/// palette document.
fn write_guide(p: &Proj, eol: &str) {
    let text = "Coding style\n============\n\nNaming\n------\n\nNames are short.\n\nTypes\n~~~~~\n\nTypes are nouns.\n\nFunctions\n~~~~~~~~~\n\nFunctions are verbs.\n\n+---+---+\n| a | b |\n+---+---+\n\nLayout\n------\n\nOne item per line.\n";
    p.write(GUIDE, &text.replace('\n', eol));
}

fn edit(
    p: &Proj,
    record: &str,
    kind: &str,
    document: &str,
    target: &str,
    body: &str,
) -> ToolResult {
    records::changeset_edit(
        &p.ctx,
        params(
            p,
            json!({"record": record, "action": "add", "kind": kind, "document": document, "target": target, "body": body}),
        ),
    )
}

fn promote(p: &Proj, record: &str) -> ToolResult {
    records::changeset_promote(
        &p.ctx,
        params(
            p,
            json!({"record": record, "implementation": "complete", "implementers": "S", "verification": "static", "verification_note": "n"}),
        ),
    )
}

fn assert_clean(p: &Proj) {
    let errors = p.errors();
    assert!(errors.is_empty(), "{}", show(&errors));
}

fn eol_of(p: &Proj) -> &'static str {
    if p.read("_palette/layout.rst").contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// Adds, stages and promotes edits to `principles.rst`, `contributing.rst` and a
/// document outside every family through the tools, on `p` (LF or CRLF).
fn edit_stage_and_promote(p: &Proj) {
    let eol = eol_of(p);
    write_guide(p, eol);
    edit(
        p,
        "RFC-0002",
        "insert_after",
        "principles.rst",
        "Keep the thing small",
        "State the limits\n^^^^^^^^^^^^^^^^\n\nEach limit is written down.",
    )
    .expect("principles edit");
    edit(
        p,
        "RFC-0002",
        "replace_body",
        "contributing.rst",
        "Commits",
        "Commits\n^^^^^^^\n\nConventional Commits, signed off; one change per commit.",
    )
    .expect("contributing edit");
    edit(
        p,
        "RFC-0002",
        "replace_body",
        GUIDE,
        "Naming",
        "Naming\n^^^^^^\n\nNames are short and say what they hold; see `the thing <../spec/thing.rst>`_.",
    )
    .expect("guide edit");
    assert_clean(p);
    let staged = p.read("docs/staging/principles.rst");
    assert!(staged.contains(&format!("State the limits{eol}----------------{eol}")));
    assert!(
        p.read("docs/staging/contributing.rst")
            .contains("Conventional Commits, signed off")
    );
    let mirror = p.read(GUIDE_MIRROR);
    assert!(mirror.contains(&format!(
        "Names are short and say what they hold; see `the thing <../spec/thing.rst>`_.{eol}{eol}Types{eol}~~~~~{eol}"
    )));
    assert!(mirror.contains(&format!("Functions{eol}~~~~~~~~~{eol}")));
    // The document itself is untouched until promotion.
    assert!(!p.read(GUIDE).contains("say what they hold"));

    promote(p, "RFC-0002").expect("promote");
    let guide = p.read(GUIDE);
    assert!(
        guide.contains("say what they hold")
            && guide.contains("Types")
            && guide.contains("Functions")
    );
    if eol == "\r\n" {
        assert!(!guide.replace("\r\n", "").contains('\n'), "{guide:?}");
    } else {
        assert!(!guide.contains('\r'));
    }
    assert!(p.read("docs/principles.rst").contains("State the limits"));
    assert!(p.read("docs/contributing.rst").contains("signed off"));
    for gone in [
        GUIDE_MIRROR,
        "docs/staging/principles.rst",
        "docs/staging/contributing.rst",
        "docs/changeset/rfc-0002.rst",
    ] {
        assert!(!p.exists(gone), "{gone}");
    }
    assert_clean(p);
}

#[test]
fn single_file_families_and_other_documents_are_edited_staged_and_promoted() {
    edit_stage_and_promote(&Proj::valid());
}

#[test]
fn single_file_families_and_other_documents_are_edited_staged_and_promoted_with_crlf() {
    edit_stage_and_promote(&Proj::valid_crlf());
}

#[test]
fn an_internal_design_document_is_edited_staged_and_promoted() {
    for p in [Proj::valid(), Proj::valid_crlf()] {
        init::layout_set(
            &p.ctx,
            params(
                &p,
                json!({"family": "design", "placement": "internal", "confirmed_by_user": true}),
            ),
        )
        .expect("move design");
        edit(
            &p,
            "RFC-0002",
            "replace_body",
            "design/overview.rst",
            "Architecture",
            "Architecture\n^^^^^^^^^^^^\n\nThe sample has two layers.",
        )
        .expect("design edit");
        assert_clean(&p);
        assert!(
            p.read("docs/staging/design/overview.rst")
                .contains("two layers")
        );
        promote(&p, "RFC-0002").expect("promote");
        assert!(
            p.read("_palette/design/overview.rst")
                .contains("two layers")
        );
        assert!(!p.exists("docs/staging/design/overview.rst"));
        assert_clean(&p);
    }
}

#[test]
fn documents_a_changeset_may_not_edit_are_refused_with_the_rule() {
    let p = Proj::valid();
    p.write("AGENTS.md", "# Agents\n\n## Build\n\nWith make.\n");
    p.write(
        "README.rst",
        "Read me\n=======\n\nBuild\n-----\n\nWith make.\n",
    );
    let before = p.snapshot();
    for (document, needle) in [
        (
            "AGENTS.md",
            "`.md` documents are named in a record's Changes",
        ),
        ("_palette/backlog.rst", "inside `_palette/`"),
        ("docs/rfc/rfc-0001-alpha.rst", "is a record or index"),
        ("docs/rfc/index.rst", "is a record or index"),
        ("docs/changeset/rfc-0002.rst", "is a changeset or index"),
        ("docs/staging/spec/thing.rst", "is a staging document"),
        ("README.rst", "outside the document root `docs`"),
        (
            "docs/spec/thing.rst",
            "write its logical name `spec/thing.rst`",
        ),
        (
            "docs/principles.rst",
            "write its logical name `principles.rst`",
        ),
        ("docs/missing.rst", "does not exist"),
    ] {
        let err = edit(&p, "RFC-0002", "delete", document, "Build", "").expect_err(document);
        assert_eq!(
            err.code,
            ErrCode::InvalidParams,
            "{document}: {}",
            err.message
        );
        assert!(
            err.message
                .starts_with(&format!("`{document}` cannot be edited by a changeset"))
                && err.message.contains(needle),
            "{document}: {}",
            err.message
        );
    }
    assert_eq!(p.snapshot(), before);
}

#[test]
fn a_refused_target_in_a_changeset_is_reported_and_never_applied() {
    let p = Proj::valid();
    p.write(
        "README.rst",
        "Read me\n=======\n\nBuild\n-----\n\nWith make.\n",
    );
    p.mutate("docs/changeset/rfc-0002.rst", |s| {
        format!("{s}\nREADME.rst\n----------\n\nDelete: Build\n~~~~~~~~~~~~~\n")
    });
    let f = p.lint();
    assert!(
        has(
            &f,
            "P007",
            "changeset/rfc-0002.rst",
            "`README.rst` cannot be edited by a changeset: it lies outside the document root"
        ),
        "{}",
        show(&f)
    );
    ops::generate(&p.ctx, &p.arg()).expect("generate");
    assert!(!p.exists("docs/staging/README.rst"));
    assert!(p.read("README.rst").contains("With make."));
}

#[test]
fn create_is_refused_outside_design_and_spec_in_both_paths() {
    let p = Proj::valid();
    write_guide(&p, "\n");
    let err = edit(
        &p,
        "RFC-0002",
        "create",
        "principles.rst",
        "More principles",
        ":Status: Maintained",
    )
    .expect_err("create principles");
    assert_eq!(err.code, ErrCode::InvalidParams);
    assert!(
        err.message
            .contains("`Create:` makes design and spec documents only"),
        "{}",
        err.message
    );

    // A Create edit written by hand: staging and lint skip it with P007, promotion refuses it.
    p.mutate("docs/changeset/rfc-0002.rst", |s| {
        format!(
            "{s}\n{GUIDE}\n{}\n\nCreate: Guide\n~~~~~~~~~~~~~\n\n:Status: Maintained\n",
            "-".repeat(GUIDE.len())
        )
    });
    let f = p.lint();
    assert!(
        has(
            &f,
            "P007",
            "changeset/rfc-0002.rst",
            "`Create:` makes design and spec documents only"
        ),
        "{}",
        show(&f)
    );
    let before = p.snapshot();
    let err = promote(&p, "RFC-0002").expect_err("promote create");
    assert_eq!(err.code, ErrCode::InvariantViolation);
    assert!(
        err.message
            .contains("`Create:` makes design and spec documents only"),
        "{}",
        err.message
    );
    assert_eq!(p.snapshot(), before);
}

#[test]
fn replace_body_through_the_tool_keeps_subsections_and_conflicts_like_replace() {
    let p = Proj::valid();
    write_guide(&p, "\n");
    edit(
        &p,
        "RFC-0002",
        "replace_body",
        GUIDE,
        "Naming",
        "Names\n^^^^^\n\nRenamed.",
    )
    .expect("rename");
    edit(
        &p,
        "RFC-0002",
        "insert_into",
        GUIDE,
        "Names",
        "Modules\n^^^^^^^\n\nModules are nouns.",
    )
    .expect("insert into the new name");
    assert_clean(&p);
    let mirror = p.read(GUIDE_MIRROR);
    assert!(
        mirror.contains("Names\n-----\n\nRenamed.\n\nTypes\n~~~~~\n\nTypes are nouns.\n\nFunctions\n~~~~~~~~~\n\nFunctions are verbs.\n\n+---+---+\n| a | b |\n+---+---+\n\nModules\n~~~~~~~\n\nModules are nouns.\n\nLayout\n"),
        "{mirror}"
    );
    // A second heading in a Replace body: edit is refused.
    let err = edit(
        &p,
        "RFC-0002",
        "replace_body",
        GUIDE,
        "Layout",
        "Layout\n^^^^^^\n\nText.\n\nInner\n\"\"\"\"\"\n\nMore.",
    )
    .expect_err("second heading");
    assert!(
        err.message.contains("no further heading"),
        "{}",
        err.message
    );
    // An independent record editing the same section conflicts, whatever the kinds.
    records::record_create(&p.ctx, params(&p, json!({
        "kind": "rfc", "title": "Epsilon", "authors": "A", "areas": "thing", "description": "Edits the guide."
    }))).expect("create");
    edit(
        &p,
        "RFC-0004",
        "replace",
        GUIDE,
        "Layout",
        "Layout\n^^^^^^\n\nTwo items per line.",
    )
    .expect("independent edit of another section");
    let err = edit(
        &p,
        "RFC-0004",
        "replace",
        GUIDE,
        "Naming",
        "Naming\n^^^^^^\n\nOther.",
    )
    .expect_err("conflict");
    assert!(
        err.message
            .contains("RFC-0004 and RFC-0002 both edit `Naming`")
            && err.message.contains("neither depends on the other"),
        "{}",
        err.message
    );
}

#[test]
fn edit_body_links_are_read_from_the_document_they_change() {
    let p = Proj::valid();
    write_guide(&p, "\n");
    // `../spec/thing.rst` is right from docs/guide/; read from the changeset folder it
    // would be right too, so use a link only the target's folder resolves.
    edit(&p, "RFC-0002", "insert_after", GUIDE, "Layout", "Records\n^^^^^^^\n\nSee `alpha <../rfc/rfc-0001-alpha.rst>`_ and `the guide <coding-style.rst#naming>`_.")
        .expect("link from the guide's folder");
    assert_clean(&p);
    let err = edit(
        &p,
        "RFC-0002",
        "insert_after",
        GUIDE,
        "Records",
        "More\n^^^^\n\nSee `x <rfc-0002.rst>`_.",
    )
    .expect_err("a link read from the changeset folder");
    assert!(
        err.message.contains("P004") && err.message.contains("does not exist"),
        "{}",
        err.message
    );
    // Moving the record family rewrites the link relative to the guide.
    init::layout_set(
        &p.ctx,
        params(
            &p,
            json!({"family": "rfc", "placement": "docs/records", "confirmed_by_user": true}),
        ),
    )
    .expect("move rfc");
    // The write tool wrapped the edit body at 79 columns, so the sentence spans lines.
    let changeset = p.read("docs/changeset/rfc-0002.rst");
    let flat = changeset.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(flat.contains(
        "`alpha <../records/rfc-0001-alpha.rst>`_ and `the guide <coding-style.rst#naming>`_"
    ));
    assert_clean(&p);
}

#[test]
fn every_document_under_staging_is_a_staging_document() {
    let p = Proj::valid();
    p.write(
        "docs/staging/guide/orphan.rst",
        "Orphan\n======\n\n+---+\n| x |\n+---+\n",
    );
    p.write(
        "docs/staging/principles.rst",
        "Principles — Sample\n===================\n",
    );
    let snap = Snapshot::load(&DiskSource, &p.root).expect("load");
    let role = |rel: &str| snap.files.iter().find(|f| f.rel == rel).map(|f| f.role);
    assert_eq!(
        role("docs/staging/guide/orphan.rst"),
        Some(Role::Staging(None))
    );
    assert_eq!(
        role("docs/staging/principles.rst"),
        Some(Role::Staging(Some(Family::Principles)))
    );
    assert_eq!(
        role("docs/staging/spec/thing.rst"),
        Some(Role::Staging(Some(Family::Spec)))
    );
    let f = p.lint();
    for rel in [
        "docs/staging/guide/orphan.rst",
        "docs/staging/principles.rst",
    ] {
        assert!(
            has(&f, "P008", rel, "no accepted changeset produces"),
            "{}",
            show(&f)
        );
        assert!(!has(&f, "P015", rel, ""), "{}", show(&f));
    }
    // The mirror of a document outside the families is not linted as a palette document.
    assert!(!has(&f, "P001", "orphan.rst", ""), "{}", show(&f));
    ops::generate(&p.ctx, &p.arg()).expect("generate");
    assert!(!p.exists("docs/staging/guide/orphan.rst") && !p.exists("docs/staging/principles.rst"));
    assert!(p.exists("docs/staging/spec/thing.rst"));
    assert_clean(&p);
}

#[test]
fn with_staging_internal_only_family_documents_are_targets() {
    let p = Proj::valid();
    write_guide(&p, "\n");
    init::layout_set(
        &p.ctx,
        params(
            &p,
            json!({"family": "staging", "placement": "internal", "confirmed_by_user": true}),
        ),
    )
    .expect("move staging");
    let err = edit(&p, "RFC-0002", "delete", GUIDE, "Layout", "").expect_err("outside the root");
    assert!(
        err.message.contains("outside the document root `_palette`"),
        "{}",
        err.message
    );
    edit(&p, "RFC-0002", "delete", "glossary.rst", "Terms", "").expect("glossary edit");
    assert!(p.exists("_palette/staging/glossary.rst"));
    assert_clean(&p);
}

#[test]
fn a_staging_placement_that_holds_a_family_is_a_layout_error() {
    let p = Proj::valid();
    p.replace(
        "_palette/layout.rst",
        ":staging: docs/staging",
        ":staging: docs",
    );
    let f = p.lint();
    assert!(
        has(
            &f,
            "P012",
            "layout.rst",
            "`staging` at `docs` contains the placement of `rfc`"
        ),
        "{}",
        show(&f)
    );
}

/// The valid fixture without `_palette/`, whose only changeset edits a document outside
/// the families: the staging folder holds no design or spec folder, so it is found
/// because it lies beside the record folders.
fn checkout_without_layout(p: &Proj) {
    std::fs::remove_dir_all(p.path("_palette")).expect("rm _palette");
    write_guide(p, "\n");
    p.write(
        "docs/changeset/rfc-0002.rst",
        &format!(
            "Changeset: RFC-0002\n===================\n\n{GUIDE}\n{}\n\nReplace body: Layout\n~~~~~~~~~~~~~~~~~~~~\n\nLayout\n^^^^^^\n\nTwo items per line.\n",
            "-".repeat(GUIDE.len())
        ),
    );
    p.replace(
        "docs/rfc/rfc-0002-beta.rst",
        ":Changes: spec/thing.rst (Contract; Rules)",
        &format!(":Changes: {GUIDE} (the layout rule)"),
    );
    std::fs::remove_dir_all(p.path("docs/staging/spec")).expect("rm staging/spec");
}

#[test]
fn without_a_layout_staging_beside_the_records_is_found_and_regenerated() {
    let p = Proj::valid();
    checkout_without_layout(&p);
    p.write(GUIDE_MIRROR, "stale\n");
    let snap = Snapshot::load(&DiskSource, &p.root).expect("load");
    assert!(snap.inferred);
    assert_eq!(
        snap.layout.placement(Family::Staging),
        Placement::Path("docs/staging".into())
    );
    assert_eq!(
        snap.files
            .iter()
            .find(|f| f.rel == GUIDE_MIRROR)
            .map(|f| f.role),
        Some(Role::Staging(None))
    );
    assert!(
        has(&p.lint(), "P008", GUIDE_MIRROR, "differs"),
        "{}",
        show(&p.lint())
    );
    ops::generate(&p.ctx, &p.arg()).expect("generate");
    let mirror = p.read(GUIDE_MIRROR);
    assert!(
        mirror.contains("Layout\n------\n\nTwo items per line.\n"),
        "{mirror}"
    );
    assert!(mirror.contains("Types\n~~~~~"));
    let again = p.snapshot();
    let second = ops::generate(&p.ctx, &p.arg()).expect("generate again");
    assert!(second.files.is_empty(), "{:?}", second.files);
    assert_eq!(p.snapshot(), again);
    assert_clean(&p);
}

#[test]
fn without_a_layout_or_a_staging_folder_no_project_document_is_a_target() {
    let p = Proj::valid();
    checkout_without_layout(&p);
    std::fs::remove_dir_all(p.path("docs/staging")).expect("rm staging");
    let f = p.lint();
    assert!(
        has(
            &f,
            "P007",
            "changeset/rfc-0002.rst",
            "outside the document root `_palette`"
        ),
        "{}",
        show(&f)
    );
}
