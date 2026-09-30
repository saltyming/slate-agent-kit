//! Runs every write tool in sequence on the valid fixture, in LF and CRLF variants,
//! checking after each step that only the listed files changed and that the project
//! still lints without errors.

mod common;

use std::collections::BTreeSet;

use common::*;
use palette_server::ops::{self, WriteResult, backlog, init, records, state};
use serde_json::json;

fn step(
    p: &Proj,
    label: &str,
    f: impl FnOnce(&palette_server::ops::Ctx) -> ToolResult,
) -> WriteResult {
    let before = p.snapshot();
    let res = f(&p.ctx).unwrap_or_else(|e| panic!("{label}: {e}"));
    let after = p.snapshot();
    let mut changed: BTreeSet<String> = BTreeSet::new();
    for k in before.keys().chain(after.keys()) {
        if before.get(k) != after.get(k) {
            changed.insert(k.clone());
        }
    }
    let listed: BTreeSet<String> = res.files.iter().map(|(f, _)| f.clone()).collect();
    assert_eq!(
        changed, listed,
        "{label}: the changed files must be exactly the listed files"
    );
    let errs = p.errors();
    assert!(
        errs.is_empty(),
        "{label}: lint errors after the step:\n{}",
        show(&errs)
    );
    assert!(!res.diff.is_empty(), "{label}: empty diff");
    res
}

/// Every file that a step changed must keep the project's line ending, CRLF or LF.
fn assert_endings(p: &Proj, res: &WriteResult, crlf: bool, label: &str) {
    for (f, action) in &res.files {
        if *action == "deleted" {
            continue;
        }
        let text = p.read(f);
        let bare_lf = text.replace("\r\n", "").contains('\n');
        let has_crlf = text.contains("\r\n");
        if crlf {
            assert!(!bare_lf, "{label}: {f} has a bare LF in a CRLF project");
        } else {
            assert!(!has_crlf, "{label}: {f} has CRLF in an LF project");
        }
    }
}

fn scenario(p: &Proj, crlf: bool) {
    macro_rules! run {
        ($label:expr, $call:expr) => {{
            let r = step(p, $label, $call);
            assert_endings(p, &r, crlf, $label);
            r
        }};
    }
    let r = run!("backlog_add", |c| backlog::backlog_add(
        c,
        params(
            p,
            json!({
                "title": "Fifth thing", "type": "test", "source": "user", "priority_signal": "low",
                "depends": ["B-2"], "body": "Test the fifth thing."
            })
        )
    ));
    assert_eq!(r.allocated, vec!["B-5"]);
    assert!(r.diff.contains("+B-5 Fifth thing"));
    assert!(p.read("_palette/backlog.rst").contains(":Depends: B-2"));

    run!("backlog_update approve", |c| backlog::backlog_update(
        c,
        params(
            p,
            json!({
                "item": "B-4", "status": "approved", "title": "Fourth thing, renamed", "body": "A new body for the fourth thing."
            })
        )
    ));
    run!("backlog_update in-phase", |c| backlog::backlog_update(
        c,
        params(
            p,
            json!({
                "item": "B-2", "status": "in-phase-1", "priority_signal": "high"
            })
        )
    ));
    let r = run!("deliverable_create", |c| backlog::deliverable_create(
        c,
        params(
            p,
            json!({
                "item": "B-2", "title": "Second thing", "what_and_why": "The second thing is missing.",
                "done_when": ["A person can use the second thing."], "not_this": ["The third thing."],
                "implementation_reference": ["src/second.rs"]
            })
        )
    ));
    assert_eq!(r.allocated, vec!["deliverable-2"]);
    assert!(p.exists("_palette/phase-1/deliverables/deliverable-2-second-thing.rst"));
    run!("deliverable_update", |c| backlog::deliverable_update(
        c,
        params(
            p,
            json!({
                "item": "B-1", "done_when": ["A person can run it.", "A person can stop it."]
            })
        )
    ));

    let r = run!("state_record decision", |c| state::state_record(
        c,
        params(
            p,
            json!({
                "kind": "decision", "text": "Operations are named open and close.", "source": "Sample Owner", "target": "docs/adr/adr-0001-naming.rst"
            })
        )
    ));
    assert_eq!(r.allocated, vec!["D-3"]);
    assert!(
        p.read("_palette/state.rst")
            .contains(":Updated: 2026-09-30")
    );
    let r = run!("state_record question", |c| state::state_record(
        c,
        params(
            p,
            json!({
                "kind": "question", "text": "Who owns the thing?", "affects": "B-1", "proposal": "Sample Owner", "proposal_by": "Sample Author"
            })
        )
    ));
    assert_eq!(r.allocated, vec!["Q-2"]);
    let r = run!("state_record discrepancy", |c| state::state_record(
        c,
        params(
            p,
            json!({
                "kind": "discrepancy", "text": "The design and the spec disagree on naming", "evidence": "build", "date": "2026-09-20"
            })
        )
    ));
    assert_eq!(r.allocated, vec!["X-2"]);

    run!("state_resolve graduated", |c| state::state_resolve(
        c,
        params(
            p,
            json!({
                "id": "D-3", "resolution": "graduated", "written": "ADR-0001"
            })
        )
    ));
    let r = run!("state_resolve answered", |c| state::state_resolve(
        c,
        params(
            p,
            json!({
                "id": "Q-1", "resolution": "answered", "answer": "No third operation in this version.", "source": "Sample Owner", "target": "docs/rfc/rfc-0003-gamma.rst"
            })
        )
    ));
    assert_eq!(r.allocated, vec!["D-4"]);
    run!("state_resolve withdrawn", |c| state::state_resolve(
        c,
        params(p, json!({"id": "Q-2", "resolution": "withdrawn"}))
    ));
    run!("state_resolve fixed", |c| state::state_resolve(
        c,
        params(p, json!({"id": "X-1", "resolution": "fixed"}))
    ));

    let r = run!("record_create rfc", |c| records::record_create(
        c,
        params(
            p,
            json!({
                "kind": "rfc", "title": "Delta rules", "authors": "Sample Author", "areas": "thing", "description": "Adds delta rules.",
                "depends": [{"record": "RFC-0002", "note": "the beta rules it extends"}],
                "changes": [{"document": "spec/thing.rst", "sections": ["Contract"]}],
                "sections": {"Summary": "Delta rules apply after beta rules."}
            })
        )
    ));
    assert_eq!(r.allocated, vec!["RFC-0004"]);
    assert!(p.exists("docs/rfc/rfc-0004-delta-rules.rst"));
    let r = run!("record_create adr", |c| records::record_create(
        c,
        params(
            p,
            json!({
                "kind": "adr", "title": "Second naming", "authors": "Sample Author", "within": "RFC-0001 (the permitted names of operations)",
                "description": "Renames the operations.", "supersedes": [{"record": "ADR-0001", "note": "the first naming choice"}]
            })
        )
    ));
    assert_eq!(r.allocated, vec!["ADR-0002"]);
    assert!(
        p.read("docs/adr/adr-0001-naming.rst")
            .contains(":Status: Superseded")
    );

    run!("record_update propose", |c| records::record_update(
        c,
        params(p, json!({"record": "RFC-0003", "status": "Proposed"}))
    ));
    run!("record_update accept", |c| records::record_update(
        c,
        params(
            p,
            json!({
                "record": "RFC-0003", "status": "Accepted", "accepted_by": "Sample Owner"
            })
        )
    ));
    assert!(
        p.read("docs/rfc/rfc-0003-gamma.rst")
            .contains(":Accepted: 2026-09-30, Sample Owner")
    );
    run!("record_update clarification", |c| records::record_update(
        c,
        params(
            p,
            json!({
                "record": "RFC-0003", "sections": {"Summary": "A third operation for the thing, named close."},
                "clarification": true, "revision_note": "Corrected the summary"
            })
        )
    ));
    assert!(
        p.read("docs/rfc/rfc-0003-gamma.rst")
            .contains(":Revised: 2026-09-30 — Corrected the summary")
    );
    run!("record_update draft", |c| records::record_update(
        c,
        params(
            p,
            json!({
                "record": "RFC-0004", "title": "Delta rules for the thing", "description": "Adds delta rules to the thing."
            })
        )
    ));

    run!("changeset_edit add", |c| records::changeset_edit(
        c,
        params(
            p,
            json!({
                "record": "RFC-0004", "action": "add", "kind": "insert_into", "document": "spec/thing.rst", "target": "Contract",
                "body": "Delta\n^^^^^\n\nThe delta rules apply after the beta rules."
            })
        )
    ));
    assert!(p.exists("docs/changeset/rfc-0004.rst"));
    run!("changeset_edit replace", |c| records::changeset_edit(
        c,
        params(
            p,
            json!({
                "record": "RFC-0004", "action": "replace", "kind": "insert_into", "document": "spec/thing.rst", "target": "Contract",
                "body": "Delta\n^^^^^\n\nThe delta rules apply to every operation, after the beta rules."
            })
        )
    ));
    run!("changeset_edit remove", |c| records::changeset_edit(
        c,
        params(
            p,
            json!({
                "record": "RFC-0004", "action": "remove", "kind": "insert_into", "document": "spec/thing.rst", "target": "Contract"
            })
        )
    ));
    assert!(!p.exists("docs/changeset/rfc-0004.rst"));
    run!("changeset_edit add again", |c| records::changeset_edit(
        c,
        params(
            p,
            json!({
                "record": "RFC-0004", "action": "add", "kind": "insert_into", "document": "spec/thing.rst", "target": "Contract",
                "body": "Delta\n^^^^^\n\nThe delta rules apply after the beta rules."
            })
        )
    ));

    run!("changeset_promote partial", |c| records::changeset_promote(
        c,
        params(
            p,
            json!({
                "record": "RFC-0002", "edits": [{"kind": "replace", "document": "spec/thing.rst", "target": "Contract"}],
                "implementation": "partial", "implementers": "Sample Implementer", "verification": "static", "verification_note": "read only"
            })
        )
    ));
    assert!(
        p.read("docs/spec/thing.rst")
            .contains("The thing has two operations: ``open`` and ``close``.")
    );
    assert!(
        p.read("docs/rfc/rfc-0002-beta.rst")
            .contains(":Implementation: partial — ")
    );
    run!(
        "changeset_promote complete",
        |c| records::changeset_promote(
            c,
            params(
                p,
                json!({
                    "record": "RFC-0002", "implementation": "complete", "implementers": "Sample Implementer",
                    "verification": "runtime", "verification_note": "unit tests only"
                })
            )
        )
    );
    assert!(!p.exists("docs/changeset/rfc-0002.rst"));
    assert!(
        p.read("docs/spec/thing.rst")
            .replace("\r\n", "\n")
            .contains("Rules\n~~~~~")
    );

    let r = run!("phase_close", |c| backlog::phase_close(
        c,
        params(
            p,
            json!({
                "phase": 1,
                "items": [{"item": "B-1", "result": "done", "outcome": "RFC-0002"}, {"item": "B-2", "result": "dropped"}],
                "new_items": [{"title": "Follow-up", "type": "tech-debt", "body": "Follow up on the first phase."}]
            })
        )
    ));
    assert_eq!(r.allocated, vec!["B-6"]);
    assert!(p.exists("_palette/phase-1/phase.rst"));
    assert!(p.exists("_palette/phase-1/deliverables/deliverable-1-first-thing.rst"));
    assert!(!p.read("_palette/state.rst").contains("Graduated to"));

    let r = run!("phase_open", |c| backlog::phase_open(
        c,
        params(
            p,
            json!({
                "title": "Second increment", "goal": "The fourth thing works.", "reason": "It follows the first increment.",
                "assumptions": [{"assumption": "The first increment is stable", "risk": "the fourth thing is redone"}],
                "exit_criteria": ["A person can use the fourth thing."], "items": ["B-4"]
            })
        )
    ));
    assert_eq!(r.allocated, vec!["phase-2"]);
    assert!(p.exists("_palette/phase-2/phase.rst"));

    let r = run!("layout_set internal", |c| init::layout_set(
        c,
        params(
            p,
            json!({
                "family": "rfc", "placement": "internal", "confirmed_by_user": true
            })
        )
    ));
    assert!(
        p.exists("_palette/rfc/rfc-0001-alpha.rst") && !p.exists("docs/rfc/rfc-0001-alpha.rst")
    );
    assert!(
        r.files
            .iter()
            .any(|(f, a)| f == "_palette/rfc/index.rst" && *a == "created")
    );
    assert!(
        p.read("_palette/rfc/rfc-0003-gamma.rst")
            .contains("../../docs/adr/adr-0001-naming.rst")
    );
    run!("layout_set back", |c| init::layout_set(
        c,
        params(
            p,
            json!({
                "family": "rfc", "placement": "docs/rfc", "confirmed_by_user": true
            })
        )
    ));
    assert!(p.exists("docs/rfc/rfc-0001-alpha.rst") && !p.exists("_palette/rfc"));
    assert!(
        p.read("docs/rfc/rfc-0003-gamma.rst")
            .contains("`ADR-0001 <../adr/adr-0001-naming.rst>`_")
    );
    let _ = ops::Ctx::new;
}

#[test]
fn every_write_tool_succeeds_in_an_lf_project() {
    let p = Proj::valid();
    scenario(&p, false);
}

#[test]
fn every_write_tool_keeps_crlf_in_a_crlf_project() {
    let p = Proj::valid_crlf();
    scenario(&p, true);
}
