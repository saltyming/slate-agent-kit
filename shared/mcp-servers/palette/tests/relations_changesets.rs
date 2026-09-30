//! Record relations (closure, incoming links, supersession, cycles) and changesets
//! (application, staging generation in dependency order, conflicts, promotion).

mod common;

use std::collections::BTreeSet;

use common::*;
use palette_server::changeset;
use palette_server::docs::Snapshot;
use palette_server::errors::ErrCode;
use palette_server::lint::Analysis;
use palette_server::ops::records;
use palette_server::records::{EdgeKind, RecId, Records};
use palette_server::vfs::DiskSource;
use serde_json::json;

fn analyse(p: &Proj) -> (Snapshot, Analysis) {
    let snap = Snapshot::load(&DiskSource, &p.root).expect("load");
    let an = Analysis::build(&snap);
    (snap, an)
}

fn id(s: &str) -> RecId {
    RecId::parse(s).expect("id")
}

#[test]
fn closure_incoming_and_supersession_are_computed() {
    let p = Proj::valid();
    let (_, an) = analyse(&p);
    let r = &an.records;
    assert_eq!(
        r.closure(id("RFC-0003")),
        BTreeSet::from([id("RFC-0002"), id("RFC-0001")])
    );
    assert_eq!(r.closure(id("RFC-0001")), BTreeSet::new());
    let incoming = r.incoming(id("RFC-0001"));
    assert!(incoming.contains(&(id("RFC-0002"), EdgeKind::Depends)));
    assert!(incoming.contains(&(id("ADR-0001"), EdgeKind::Within)));
    // A body link is an incoming link too.
    assert!(
        r.incoming(id("ADR-0001"))
            .contains(&(id("RFC-0003"), EdgeKind::Link))
    );
    assert!(r.superseded_by(id("RFC-0001")).is_empty());

    p.replace(
        "docs/rfc/rfc-0003-gamma.rst",
        ":Supersedes: none",
        ":Supersedes: RFC-0001 (the alpha contract)",
    );
    let (_, an) = analyse(&p);
    assert_eq!(
        an.records.superseded_by(id("RFC-0001")),
        vec![(id("RFC-0003"), None)]
    );
    assert_eq!(
        an.records.wholly_superseded_by(id("RFC-0001")),
        vec![id("RFC-0003")]
    );
}

#[test]
fn cycles_are_found_and_the_closure_terminates() {
    let p = Proj::valid();
    p.replace(
        "docs/rfc/rfc-0001-alpha.rst",
        ":Depends: none",
        ":Depends: RFC-0003 (a newer record)",
    );
    let (_, an) = analyse(&p);
    let cycles = an.records.cycles();
    // RFC-0003 links to ADR-0001 in its body and ADR-0001 depends on RFC-0001: one group.
    assert_eq!(
        cycles,
        vec![vec![
            id("RFC-0001"),
            id("RFC-0002"),
            id("RFC-0003"),
            id("ADR-0001")
        ]]
    );
    assert_eq!(
        an.records.closure(id("RFC-0001")),
        BTreeSet::from([id("RFC-0002"), id("RFC-0003")])
    );
}

#[test]
fn forward_and_cross_kind_direction_rules() {
    let p = Proj::valid();
    // Same kind: a lower number is older. Cross kind: an earlier or equal date is older.
    p.replace(
        "docs/adr/adr-0001-naming.rst",
        ":Date: 2026-02-01",
        ":Date: 2026-01-10",
    );
    assert!(
        p.lint().iter().all(|f| f.rule != "P005"),
        "equal dates are allowed"
    );
    p.replace(
        "docs/adr/adr-0001-naming.rst",
        ":Date: 2026-01-10",
        ":Date: 2026-01-09",
    );
    assert!(has(&p.lint(), "P005", "adr-0001", "later than"));
}

#[test]
fn dependency_order_beats_date_order() {
    let p = Proj::valid();
    // RFC-0002 depends on RFC-0001, but RFC-0001 carries a later date: the dependency still comes first.
    p.replace(
        "docs/rfc/rfc-0001-alpha.rst",
        ":Date: 2026-01-10",
        ":Date: 2026-12-01",
    );
    let (_, an) = analyse(&p);
    let order = an
        .records
        .dependency_order(&[id("RFC-0002"), id("RFC-0001"), id("RFC-0003")]);
    assert_eq!(order, vec![id("RFC-0001"), id("RFC-0002"), id("RFC-0003")]);
}

fn accept_rfc4_depending_on_rfc2(p: &Proj, depends: bool) {
    let dep = if depends {
        "RFC-0002 (extends the beta rules directly)"
    } else {
        "none"
    };
    let text = p
        .read("docs/rfc/rfc-0003-gamma.rst")
        .replace("RFC-0003: Gamma proposal", "RFC-0004: Delta rules")
        .replace(":Status: Draft", ":Status: Accepted")
        .replace(
            ":Accepted: none",
            ":Accepted: Sample Owner (2026-04-01T00:00Z)",
        )
        .replace(":Date: 2026-03-10", ":Date: 2026-04-01")
        .replace(
            ":Depends: RFC-0002 (the beta rules it builds on)",
            &format!(":Depends: {dep}"),
        )
        .replace("=========================", "=====================")
        .replace("Gamma proposal", "Delta rules")
        .replace(
            "Proposes a third operation for the thing.",
            "Adds delta rules.",
        )
        .replace(
            "See `ADR-0001 <../adr/adr-0001-naming.rst>`_ for the naming choice.",
            "No references.",
        );
    p.write(
        "docs/rfc/rfc-0004-delta.rst",
        &text.replace("======================\n", "=====================\n"),
    );
    p.write(
        "docs/changeset/rfc-0004.rst",
        "Changeset: RFC-0004\n===================\n\nspec/thing.rst\n--------------\n\nInsert into: Contract\n~~~~~~~~~~~~~~~~~~~~~\n\nDelta\n^^^^^\n\nThe delta rules.\n",
    );
}

#[test]
fn staging_applies_accepted_changesets_in_dependency_order() {
    let p = Proj::valid();
    accept_rfc4_depending_on_rfc2(&p, true);
    let (snap, an) = analyse(&p);
    let staged = an.plan.docs.get("spec/thing.rst").expect("staged");
    assert_eq!(staged.records, vec![id("RFC-0002"), id("RFC-0004")]);
    let text = staged.source.to_text();
    let rules = text.find("Rules\n~~~~~").expect("rules");
    let delta = text.find("Delta\n~~~~~").expect("delta");
    assert!(rules < delta, "RFC-0002's subsection comes first:\n{text}");
    assert!(an.plan.failures.is_empty());
    let _ = snap;
}

#[test]
fn draft_and_proposed_changesets_are_checked_not_applied() {
    let p = Proj::valid();
    accept_rfc4_depending_on_rfc2(&p, true);
    p.replace(
        "docs/rfc/rfc-0004-delta.rst",
        ":Status: Accepted",
        ":Status: Proposed",
    );
    p.replace(
        "docs/rfc/rfc-0004-delta.rst",
        ":Accepted: Sample Owner (2026-04-01T00:00Z)",
        ":Accepted: none",
    );
    let (_, an) = analyse(&p);
    let staged = an.plan.docs.get("spec/thing.rst").expect("staged");
    assert_eq!(
        staged.records,
        vec![id("RFC-0002")],
        "a proposed record's edits are not applied"
    );
    // But they are checked: an edit that does not resolve is an error.
    p.replace(
        "docs/changeset/rfc-0004.rst",
        "Insert into: Contract",
        "Insert into: Nowhere",
    );
    assert!(has(&p.lint(), "P007", "rfc-0004.rst", "does not resolve"));
}

#[test]
fn edits_resolve_against_the_dependency_closure_only() {
    let p = Proj::valid();
    accept_rfc4_depending_on_rfc2(&p, false);
    // "Rules" exists only after RFC-0002's edit; RFC-0004 does not depend on RFC-0002.
    p.replace(
        "docs/changeset/rfc-0004.rst",
        "Insert into: Contract",
        "Insert into: Rules",
    );
    let f = p.lint();
    assert!(
        has(&f, "P007", "rfc-0004.rst", "section `Rules` does not exist"),
        "{}",
        show(&f)
    );
    // Adding the dependency makes the same edit resolve.
    p.replace(
        "docs/rfc/rfc-0004-delta.rst",
        ":Depends: none",
        ":Depends: RFC-0002 (extends the beta rules directly)",
    );
    let f = p.lint();
    assert!(!f.iter().any(|x| x.rule == "P007"), "{}", show(&f));
}

#[test]
fn staging_of_a_created_document_and_its_promotion() {
    let p = Proj::valid();
    p.mutate("docs/changeset/rfc-0002.rst", |s| {
        format!("{s}\nspec/extra.rst\n--------------\n\nCreate: Extra\n~~~~~~~~~~~~~\n\n:Status: Contract\n\nScope and authority\n^^^^^^^^^^^^^^^^^^^\n\nText.\n")
    });
    p.replace(
        "docs/rfc/rfc-0002-beta.rst",
        ":Changes: spec/thing.rst (Contract; Rules)",
        ":Changes: spec/thing.rst (Contract; Rules); spec/extra.rst (created)",
    );
    let (_, an) = analyse(&p);
    let staged = an
        .plan
        .docs
        .get("spec/extra.rst")
        .expect("created document is staged");
    assert!(!staged.from_existing);
    assert!(
        staged.source.to_text().starts_with(
            "Extra\n=====\n\n:Status: Contract\n\nScope and authority\n---------------------\n"
        ) || staged.source.to_text().starts_with(
            "Extra\n=====\n\n:Status: Contract\n\nScope and authority\n-------------------\n"
        )
    );
}

#[test]
fn an_independent_conflicting_edit_is_refused_by_the_tool() {
    let p = Proj::valid();
    records::record_create(&p.ctx, params(&p, json!({
        "kind": "rfc", "title": "Epsilon", "authors": "A", "areas": "thing", "description": "Edits the contract."
    }))).expect("create");
    let before = p.snapshot();
    let err = records::changeset_edit(&p.ctx, params(&p, json!({
        "record": "RFC-0004", "action": "add", "kind": "insert_into", "document": "spec/thing.rst", "target": "Contract",
        "body": "Eps\n^^^\n\nText."
    }))).expect_err("conflict");
    assert_eq!(err.code, ErrCode::InvariantViolation);
    assert!(
        err.message.contains("neither depends on the other"),
        "{}",
        err.message
    );
    assert_eq!(p.snapshot(), before);
    // Declaring the dependency first makes the edit acceptable.
    records::record_update(&p.ctx, params(&p, json!({
        "record": "RFC-0004", "depends": [{"record": "RFC-0002", "note": "directly edits the section RFC-0002 edits"}]
    }))).expect("depends");
    records::changeset_edit(&p.ctx, params(&p, json!({
        "record": "RFC-0004", "action": "add", "kind": "insert_into", "document": "spec/thing.rst", "target": "Contract",
        "body": "Eps\n^^^\n\nText."
    }))).expect("edit");
}

#[test]
fn promotion_waits_for_the_edits_of_dependencies() {
    let p = Proj::valid();
    accept_rfc4_depending_on_rfc2(&p, true);
    let before = p.snapshot();
    let err = records::changeset_promote(&p.ctx, params(&p, json!({
        "record": "RFC-0004", "implementation": "complete", "implementers": "S", "verification": "static", "verification_note": "n"
    }))).expect_err("order");
    assert_eq!(err.code, ErrCode::InvariantViolation);
    assert!(
        err.message.contains("RFC-0002") && err.message.contains("promote those first"),
        "{}",
        err.message
    );
    assert_eq!(p.snapshot(), before);
    records::changeset_promote(&p.ctx, params(&p, json!({
        "record": "RFC-0002", "implementation": "complete", "implementers": "S", "verification": "static", "verification_note": "n"
    }))).expect("first");
    records::changeset_promote(&p.ctx, params(&p, json!({
        "record": "RFC-0004", "implementation": "complete", "implementers": "S", "verification": "static", "verification_note": "n"
    }))).expect("second");
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
    assert!(
        !p.exists("docs/staging/spec/thing.rst"),
        "nothing pending, so no staging document"
    );
}

#[test]
fn promotion_sets_the_time_varying_fields_and_creates_documents() {
    let p = Proj::valid();
    p.mutate("docs/changeset/rfc-0002.rst", |s| {
        format!("{s}\nspec/extra.rst\n--------------\n\nCreate: Extra\n~~~~~~~~~~~~~\n\n:Status: Contract\n\nScope and authority\n^^^^^^^^^^^^^^^^^^^\n\nText.\n\nDefinitions and model\n^^^^^^^^^^^^^^^^^^^^^\n\nText.\n\nContract\n^^^^^^^^\n\nText.\n\nErrors and edge cases\n^^^^^^^^^^^^^^^^^^^^^\n\nText.\n\nOwnership and ordering\n^^^^^^^^^^^^^^^^^^^^^^\n\nText.\n\nCompatibility\n^^^^^^^^^^^^^\n\nText.\n\nConformance\n^^^^^^^^^^^\n\nText.\n\nReferences\n^^^^^^^^^^\n\nText.\n")
    });
    p.replace(
        "docs/rfc/rfc-0002-beta.rst",
        ":Changes: spec/thing.rst (Contract; Rules)",
        ":Changes: spec/thing.rst (Contract; Rules); spec/extra.rst (created)",
    );
    records::changeset_promote(&p.ctx, params(&p, json!({
        "record": "RFC-0002", "implementation": "complete", "implementation_scope": "the whole beta rules",
        "implementers": "Sample Implementer", "verification": "runtime", "verification_note": "unit tests only"
    }))).expect("promote");
    let rec = p.read("docs/rfc/rfc-0002-beta.rst");
    assert!(rec.contains(":Implementation: complete — the whole beta rules"));
    assert!(rec.contains(":Implementers: Sample Implementer"));
    assert!(rec.contains(":Verification: runtime — 2026-09-30; unit tests only"));
    let extra = p.read("docs/spec/extra.rst");
    assert!(
        extra.starts_with("Extra\n=====\n\n:Status: Contract\n:Date: 2026-09-30\n"),
        "{extra}"
    );
    assert!(!p.exists("docs/changeset/rfc-0002.rst"));
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
}

const CELL_TABLE: &str = ".. list-table:: Operations
   :header-rows: 1
   :widths: 30 70

   * - Operation
     - Effect
   * - ``open``
     - Opens the thing, as `the contract <#contract>`_ says;
       *emphasis* may open a continuation line.
   * - ``close``
     -";

const RUST_BLOCK: &str = ".. code-block:: rust

   fn close(t: Thing) -> Option<u8> {
       t.map(|x| x)
   }";

/// Edits RFC-0002's changeset through the tool so that a replace, an insert-after and
/// a create each carry a list-table and a code block, then checks that staging and
/// promote keep those lines unchanged, with the project's line endings.
fn tables_survive_staging_and_promote(p: &Proj, crlf: bool) {
    let eol = if crlf { "\r\n" } else { "\n" };
    let holds = |rel: &str, block: &str| {
        let text = p.read(rel);
        assert!(
            text.contains(&block.replace('\n', eol)),
            "{rel} lost the block {block:?}:\n{text}"
        );
    };
    let blocks = format!("{CELL_TABLE}\n\n{RUST_BLOCK}");
    p.replace(
        "docs/rfc/rfc-0002-beta.rst",
        ":Changes: spec/thing.rst (Contract; Rules)",
        ":Changes: spec/thing.rst (Contract; Rules; Limits); spec/extra.rst (created)",
    );
    records::changeset_edit(&p.ctx, params(p, json!({
        "record": "RFC-0002", "action": "replace", "kind": "replace", "document": "spec/thing.rst",
        "target": "Contract", "body": format!("Contract\n^^^^^^^^\n\nThe thing has two operations.\n\n{blocks}\n")
    }))).expect("replace");
    records::changeset_edit(&p.ctx, params(p, json!({
        "record": "RFC-0002", "action": "add", "kind": "insert_after", "document": "spec/thing.rst",
        "target": "Rules", "body": format!("Limits\n^^^^^^\n\n{blocks}\n")
    }))).expect("insert after");
    let sections = [
        "Scope and authority",
        "Definitions and model",
        "Contract",
        "Errors and edge cases",
        "Ownership and ordering",
        "Compatibility",
        "Conformance",
        "References",
    ];
    let body: String = sections
        .iter()
        .map(|t| {
            let text = if *t == "Contract" {
                blocks.as_str()
            } else {
                "Text."
            };
            format!("{t}\n{}\n\n{text}\n\n", "^".repeat(t.len()))
        })
        .collect();
    records::changeset_edit(&p.ctx, params(p, json!({
        "record": "RFC-0002", "action": "add", "kind": "create", "document": "spec/extra.rst",
        "target": "Extra", "body": format!(":Status: Contract\n\n{body}")
    }))).expect("create");
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
    for rel in [
        "docs/changeset/rfc-0002.rst",
        "docs/staging/spec/thing.rst",
        "docs/staging/spec/extra.rst",
    ] {
        holds(rel, CELL_TABLE);
        holds(rel, RUST_BLOCK);
    }
    let staged = p.read("docs/staging/spec/thing.rst");
    assert_eq!(
        staged.matches(".. list-table:: Operations").count(),
        2,
        "{staged}"
    );

    records::changeset_promote(&p.ctx, params(p, json!({
        "record": "RFC-0002", "implementation": "complete", "implementers": "Sample Implementer",
        "verification": "runtime", "verification_note": "unit tests only"
    }))).expect("promote");
    for rel in ["docs/spec/thing.rst", "docs/spec/extra.rst"] {
        holds(rel, CELL_TABLE);
        holds(rel, RUST_BLOCK);
        if crlf {
            assert!(
                !p.read(rel).replace("\r\n", "").contains('\n'),
                "{rel} mixes line endings"
            );
        }
    }
    assert_eq!(
        p.read("docs/spec/thing.rst")
            .matches(".. list-table:: Operations")
            .count(),
        2
    );
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
}

#[test]
fn tables_and_code_blocks_survive_staging_and_promote() {
    tables_survive_staging_and_promote(&Proj::valid(), false);
}

#[test]
fn tables_and_code_blocks_survive_staging_and_promote_with_crlf() {
    tables_survive_staging_and_promote(&Proj::valid_crlf(), true);
}

#[test]
fn records_module_reports_relations_for_status() {
    let p = Proj::valid();
    let text = palette_server::tools::status(&p.ctx, params(&p, json!({"record": "RFC-0002"})))
        .expect("status");
    assert!(text.contains("Linked from: RFC-0003 (Depends)"), "{text}");
    assert!(text.contains("Dependency closure: RFC-0001"), "{text}");
    assert!(text.contains("Pending changeset edits: 2"), "{text}");
    assert!(
        text.contains("staging document: docs/staging/spec/thing.rst"),
        "{text}"
    );
    let _ = Records::id_of;
    let _ = changeset::is_checked_status;
}

// ── links inside edit bodies and Changes against the dependency closure ──────────

/// Regenerates the indexes and staging documents after a hand edit.
fn regenerate(p: &Proj) {
    palette_server::ops::generate(&p.ctx, &p.arg()).expect("generate");
}

/// The 1-based line of the first line of `rel` that contains `needle`.
fn line_of(p: &Proj, rel: &str, needle: &str) -> usize {
    p.read(rel)
        .lines()
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("{rel} has no line with {needle:?}"))
        + 1
}

fn p004(p: &Proj) -> Vec<palette_server::lint::Finding> {
    p.lint().into_iter().filter(|f| f.rule == "P004").collect()
}

/// The body of a spec document created by a `Create:` edit, with `contract` as the
/// Contract section's text.
fn spec_body(contract: &str) -> String {
    [
        "Scope and authority",
        "Definitions and model",
        "Contract",
        "Errors and edge cases",
        "Ownership and ordering",
        "Compatibility",
        "Conformance",
        "References",
    ]
    .iter()
    .map(|t| {
        let text = if *t == "Contract" { contract } else { "Text." };
        format!("{t}\n{}\n\n{text}\n\n", "^".repeat(t.len()))
    })
    .fold(":Status: Contract\n\n".to_string(), |a, s| a + &s)
}

/// A changeset section that creates `logical` with `contract` as its Contract text.
fn create_section(logical: &str, title: &str, contract: &str) -> String {
    format!(
        "\n{logical}\n{}\n\nCreate: {title}\n{}\n\n{}",
        "-".repeat(logical.len()),
        "~".repeat(title.len() + 8),
        spec_body(contract).trim_end()
    ) + "\n"
}

/// Appends `text` (with `\n` line breaks) to RFC-0002's changeset, in the body of its
/// last edit (`Insert into: Contract` of `spec/thing.rst`), in the file's line endings.
fn append_to_rfc2_changeset(p: &Proj, text: &str) {
    p.mutate("docs/changeset/rfc-0002.rst", |s| {
        let eol = if s.contains("\r\n") { "\r\n" } else { "\n" };
        format!("{s}{eol}{}{eol}", text.replace('\n', eol))
    });
}

fn edit_body_links_read_from_the_target(p: &Proj) {
    let cs = "docs/changeset/rfc-0002.rst";
    let before = p.read(cs);
    // Right for the target (`docs/spec/`), wrong for the changeset folder.
    append_to_rfc2_changeset(
        p,
        "See `the thing itself <thing.rst>`_, `its errors <#errors-and-edge-cases>`_ and `the overview <../design/overview.rst>`_.",
    );
    assert!(p004(p).is_empty(), "{}", show(&p004(p)));

    p.write(cs, &before);
    append_to_rfc2_changeset(p, "See `a missing file <missing.rst>`_.");
    let line = line_of(p, cs, "<missing.rst>");
    let f = p004(p);
    assert!(
        f.iter().any(|x| x.file == cs
            && x.line == line
            && x.message
                .contains("link target `missing.rst` does not exist")),
        "{}",
        show(&f)
    );

    // Right only relative to the changeset folder: judged from the target, so missing.
    p.write(cs, &before);
    append_to_rfc2_changeset(p, "See `this changeset <rfc-0002.rst>`_.");
    let f = p004(p);
    assert!(
        has(&f, "P004", cs, "link target `rfc-0002.rst` does not exist"),
        "{}",
        show(&f)
    );

    p.write(cs, &before);
    append_to_rfc2_changeset(p, "See `nothing <#nowhere>`_.");
    let f = p004(p);
    assert!(
        has(
            &f,
            "P004",
            cs,
            "anchor `#nowhere` does not exist in `the document this edit changes`"
        ),
        "{}",
        show(&f)
    );

    // Text outside every edit body is still read from the changeset's folder.
    p.write(cs, &before);
    p.mutate(cs, |s| {
        let eol = if s.contains("\r\n") { "\r\n" } else { "\n" };
        s.replacen(
            &format!("==================={eol}{eol}"),
            &format!(
                "==================={eol}{eol}Edits of `this changeset <rfc-0002.rst>`_.{eol}{eol}"
            ),
            1,
        )
    });
    assert!(p004(p).is_empty(), "{}", show(&p004(p)));
}

#[test]
fn edit_body_links_are_read_from_the_target_document() {
    edit_body_links_read_from_the_target(&Proj::valid());
}

#[test]
fn edit_body_links_are_read_from_the_target_document_with_crlf() {
    edit_body_links_read_from_the_target(&Proj::valid_crlf());
}

#[test]
fn edit_body_links_see_what_the_record_creates() {
    let p = Proj::valid();
    p.replace(
        "docs/rfc/rfc-0002-beta.rst",
        ":Changes: spec/thing.rst (Contract; Rules)",
        ":Changes: spec/thing.rst (Contract; Rules); spec/extra.rst (created); spec/more.rst (created)",
    );
    let cs = "docs/changeset/rfc-0002.rst";
    let before = p.read(cs);
    // `more.rst` is created by the same changeset; `Rules` is a section its
    // `Insert into` adds to the existing `thing.rst`.
    p.mutate(cs, |s| {
        format!(
            "{s}{}{}",
            create_section(
                "spec/extra.rst",
                "Extra",
                "See `more <more.rst#contract>`_ and `the rules <thing.rst#rules>`_."
            ),
            create_section("spec/more.rst", "More", "Text.")
        )
    });
    assert!(p004(&p).is_empty(), "{}", show(&p004(&p)));

    p.write(cs, &before);
    p.mutate(cs, |s| {
        format!(
            "{s}{}",
            create_section(
                "spec/extra.rst",
                "Extra",
                "See `absent <absent.rst>`_ and `nothing <thing.rst#nowhere>`_."
            )
        )
    });
    let f = p004(&p);
    assert!(
        has(&f, "P004", cs, "link target `absent.rst` does not exist"),
        "{}",
        show(&f)
    );
    assert!(
        has(
            &f,
            "P004",
            cs,
            "anchor `#nowhere` does not exist in `thing.rst`"
        ),
        "{}",
        show(&f)
    );
}

#[test]
fn edit_body_links_see_what_the_dependency_closure_creates() {
    for depends in [true, false] {
        let p = Proj::valid();
        p.mutate("docs/changeset/rfc-0002.rst", |s| {
            format!("{s}{}", create_section("spec/extra.rst", "Extra", "Text."))
        });
        p.replace(
            "docs/rfc/rfc-0002-beta.rst",
            ":Changes: spec/thing.rst (Contract; Rules)",
            ":Changes: spec/thing.rst (Contract; Rules); spec/extra.rst (created)",
        );
        accept_rfc4_depending_on_rfc2(&p, depends);
        p.replace(
            "docs/changeset/rfc-0004.rst",
            "The delta rules.",
            "The delta rules, beside `the extra <extra.rst#contract>`_.",
        );
        let f = p004(&p);
        let reported = has(
            &f,
            "P004",
            "docs/changeset/rfc-0004.rst",
            "link target `extra.rst` does not exist",
        );
        assert_eq!(reported, !depends, "depends: {depends}\n{}", show(&f));
    }
}

#[test]
fn edit_body_links_hold_in_staging_and_after_promotion() {
    let p = Proj::valid();
    p.replace(
        "docs/rfc/rfc-0002-beta.rst",
        ":Changes: spec/thing.rst (Contract; Rules)",
        ":Changes: spec/thing.rst (Contract; Rules); spec/extra.rst (created)",
    );
    records::changeset_edit(&p.ctx, params(&p, json!({
        "record": "RFC-0002", "action": "add", "kind": "create", "document": "spec/extra.rst",
        "target": "Extra", "body": spec_body("See `the rules <thing.rst#rules>`_.")
    }))).expect("create");
    let link =
        "See `the extra <extra.rst#contract>`_ and `the overview <../design/overview.rst>`_.";
    records::changeset_edit(&p.ctx, params(&p, json!({
        "record": "RFC-0002", "action": "replace", "kind": "insert_into", "document": "spec/thing.rst",
        "target": "Contract", "body": format!("Rules\n^^^^^\n\nThe beta rules apply to every operation.\n\n{link}\n")
    }))).expect("replace");
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
    assert!(p.read("docs/staging/spec/thing.rst").contains(link));
    assert!(
        p.read("docs/staging/spec/extra.rst")
            .contains("`the rules <thing.rst#rules>`_")
    );

    records::changeset_promote(
        &p.ctx,
        params(
            &p,
            json!({
                "record": "RFC-0002", "implementation": "complete", "implementers": "S",
                "verification": "static", "verification_note": "n"
            }),
        ),
    )
    .expect("promote");
    assert!(p.read("docs/spec/thing.rst").contains(link));
    assert!(
        p.read("docs/spec/extra.rst")
            .contains("`the rules <thing.rst#rules>`_")
    );
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
}

fn move_family(p: &Proj, family: &str, placement: &str) {
    palette_server::ops::init::layout_set(
        &p.ctx,
        params(
            p,
            json!({"family": family, "placement": placement, "confirmed_by_user": true}),
        ),
    )
    .unwrap_or_else(|e| panic!("move {family} to {placement}: {}", e.message));
}

#[test]
fn moving_families_keeps_edit_body_links_relative_to_the_target() {
    let p = Proj::valid();
    p.replace(
        "docs/rfc/rfc-0002-beta.rst",
        ":Changes: spec/thing.rst (Contract; Rules)",
        ":Changes: spec/thing.rst (Contract; Rules); spec/extra.rst (created)",
    );
    records::changeset_edit(&p.ctx, params(&p, json!({
        "record": "RFC-0002", "action": "add", "kind": "create", "document": "spec/extra.rst",
        "target": "Extra", "body": spec_body("See `the thing <thing.rst>`_.")
    }))).expect("create");
    records::changeset_edit(&p.ctx, params(&p, json!({
        "record": "RFC-0002", "action": "replace", "kind": "insert_into", "document": "spec/thing.rst",
        "target": "Contract",
        "body": "Rules\n^^^^^\n\nSee `the extra <extra.rst>`_ and `the overview <../design/overview.rst>`_.\n"
    }))).expect("replace");
    let cs = |p: &Proj, dir: &str| p.read(&format!("{dir}/rfc-0002.rst"));
    // Outside every edit body, a link is read from the changeset's own folder.
    p.mutate("docs/changeset/rfc-0002.rst", |s| {
        s.replacen(
            "===================\n\n",
            "===================\n\nEdits of `RFC-0002 <../rfc/rfc-0002-beta.rst>`_.\n\n",
            1,
        )
    });
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));

    // The changeset moves; the target documents do not, so edit bodies keep their links.
    move_family(&p, "changeset", "docs/records/changes");
    let text = cs(&p, "docs/records/changes");
    assert!(
        text.contains("`RFC-0002 <../../rfc/rfc-0002-beta.rst>`_"),
        "{text}"
    );
    assert!(
        text.contains("See `the extra <extra.rst>`_ and `the overview <../design/overview.rst>`_."),
        "{text}"
    );
    assert!(text.contains("See `the thing <thing.rst>`_."), "{text}");
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));

    // The spec family moves one level deeper: links from its edit bodies to the design
    // family gain a level; links between spec documents, created or not, stay.
    move_family(&p, "spec", "docs/deep/spec");
    let text = cs(&p, "docs/records/changes");
    assert!(
        text.contains(
            "See `the extra <extra.rst>`_ and `the overview <../../design/overview.rst>`_."
        ),
        "{text}"
    );
    assert!(text.contains("See `the thing <thing.rst>`_."), "{text}");
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));

    // The design family moves: the link to it follows, read from the spec folder.
    move_family(&p, "design", "docs/design2");
    let text = cs(&p, "docs/records/changes");
    assert!(
        text.contains("`the overview <../../design2/overview.rst>`_."),
        "{text}"
    );
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
}

#[test]
fn palette_links_from_edit_bodies_follow_the_target_placement() {
    let p = Proj::valid();
    // The documents that link to the spec move first, so no project document links
    // into `_palette/`.
    move_family(&p, "glossary", "internal");
    move_family(&p, "design", "internal");
    move_family(&p, "spec", "internal");
    let cs = "docs/changeset/rfc-0002.rst";
    let before = p.read(cs);
    // The edit's target lives in `_palette/spec/`, so its text may link into `_palette/`.
    append_to_rfc2_changeset(&p, "See `the state <../state.rst>`_.");
    assert!(p004(&p).is_empty(), "{}", show(&p004(&p)));
    // The same target from text outside every edit body is a project document's link.
    p.write(cs, &before);
    p.replace(
        cs,
        "===================\n\n",
        "===================\n\nSee `the state <../../_palette/state.rst>`_.\n\n",
    );
    let f = p004(&p);
    assert!(
        has(&f, "P004", cs, "must not link into `_palette/`"),
        "{}",
        show(&f)
    );
}

#[test]
fn changes_accept_documents_the_dependency_closure_creates() {
    let setup = |depends: bool, changes: &str| {
        let p = Proj::valid();
        p.mutate("docs/changeset/rfc-0002.rst", |s| {
            format!("{s}{}", create_section("spec/extra.rst", "Extra", "Text."))
        });
        p.replace(
            "docs/rfc/rfc-0002-beta.rst",
            ":Changes: spec/thing.rst (Contract; Rules)",
            ":Changes: spec/thing.rst (Contract; Rules); spec/extra.rst (created)",
        );
        accept_rfc4_depending_on_rfc2(&p, depends);
        p.replace(
            "docs/rfc/rfc-0004-delta.rst",
            ":Changes: none",
            &format!(":Changes: {changes}"),
        );
        p.write(
            "docs/changeset/rfc-0004.rst",
            "Changeset: RFC-0004\n===================\n\nspec/extra.rst\n--------------\n\nReplace: Contract\n~~~~~~~~~~~~~~~~~\n\nContract\n^^^^^^^^\n\nThe delta contract.\n",
        );
        regenerate(&p);
        p
    };
    let rfc4 = "docs/rfc/rfc-0004-delta.rst";

    let p = setup(true, "spec/extra.rst (Contract)");
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));

    let p = setup(false, "spec/extra.rst (Contract)");
    assert!(
        has(
            &p.lint(),
            "P006",
            rfc4,
            "`spec/extra.rst` does not exist and this record's changeset does not create it"
        ),
        "{}",
        show(&p.lint())
    );

    let p = setup(true, "spec/extra.rst (Contract; Nowhere)");
    let f = p.lint();
    assert!(has(&f, "P006", rfc4, "section `Nowhere`"), "{}", show(&f));
    assert!(!has(&f, "P006", rfc4, "section `Contract`"), "{}", show(&f));

    // `(created)` still means this record's own changeset creates the document.
    let p = setup(true, "spec/extra.rst (created)");
    let f = p.lint();
    assert!(
        has(&f, "P006", rfc4, "a dependency's changeset creates it"),
        "{}",
        show(&f)
    );
}
