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
