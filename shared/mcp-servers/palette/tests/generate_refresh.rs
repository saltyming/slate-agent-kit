//! `palette_generate` and `palette_record_refresh` (RFC-0021): regeneration through the
//! server, and a record's derived fields brought in line when its change landed outside
//! a changeset.

mod common;

use common::*;
use palette_server::errors::ErrCode;
use palette_server::ops::{self, NOTHING_TO_GENERATE, records};
use serde_json::{Value, json};

const STAGED: &str = "docs/staging/spec/thing.rst";
const ADR: &str = "docs/adr/adr-0001-naming.rst";
const RFC2: &str = "docs/rfc/rfc-0002-beta.rst";

fn generate(p: &Proj, dry_run: bool) -> ToolResult {
    ops::generate_tool(&p.ctx, params(p, json!({"dry_run": dry_run})))
}

fn refresh(p: &Proj, args: Value) -> ToolResult {
    records::record_refresh(&p.ctx, params(p, args))
}

fn field<'a>(text: &'a str, name: &str) -> &'a str {
    let prefix = format!(":{name}: ");
    text.lines()
        .find_map(|l| l.strip_prefix(&prefix))
        .map(|v| v.trim_end_matches('\r'))
        .unwrap_or_else(|| panic!("no {name}"))
}

fn assert_clean(p: &Proj) {
    let errors = p.errors();
    assert!(errors.is_empty(), "{}", show(&errors));
}

#[test]
fn generate_repairs_drift_and_then_has_nothing_to_do() {
    for p in [Proj::valid(), Proj::valid_crlf()] {
        let crlf = p.read(STAGED).contains("\r\n");
        let good = p.read(STAGED);
        p.mutate(STAGED, |s| s.replace("two operations", "three operations"));
        p.write("docs/staging/spec/orphan.rst", "Orphan\n======\n");
        let drifted = p.snapshot();

        let dry = generate(&p, true).expect("dry run");
        assert!(dry.dry_run);
        assert_eq!(
            dry.files,
            vec![
                ("docs/staging/spec/orphan.rst".to_string(), "deleted"),
                (STAGED.to_string(), "modified"),
            ]
        );
        assert!(dry.diff.contains("three operations") && dry.message.is_none());
        assert_eq!(p.snapshot(), drifted, "a dry run writes nothing");

        let done = generate(&p, false).expect("generate");
        assert_eq!(done.files.len(), 2);
        assert_eq!(p.read(STAGED), good, "crlf={crlf}");
        assert!(!p.exists("docs/staging/spec/orphan.rst"));
        assert_clean(&p);

        let idle = p.snapshot();
        let again = generate(&p, false).expect("generate again");
        assert!(again.files.is_empty() && again.diff.is_empty());
        assert_eq!(again.message.as_deref(), Some(NOTHING_TO_GENERATE));
        let json: Value = serde_json::from_str(&again.to_json()).expect("json");
        assert!(
            json["message"]
                .as_str()
                .is_some_and(|m| m.starts_with("nothing to generate"))
        );
        assert_eq!(json["files"], json!([]));
        assert_eq!(p.snapshot(), idle);
    }
}

#[test]
fn generate_works_without_a_layout_like_the_command_line() {
    let p = Proj::valid();
    std::fs::remove_dir_all(p.path("_palette")).expect("rm _palette");
    p.mutate(STAGED, |s| s.replace("two operations", "three operations"));
    let r = generate(&p, false).expect("generate");
    assert_eq!(r.files, vec![(STAGED.to_string(), "modified")]);
    assert!(p.read(STAGED).contains("two operations"));
}

/// The header lines `palette_record_refresh` must never touch.
fn events(text: &str) -> Vec<String> {
    ["Revised", "Verification", "Accepted"]
        .iter()
        .map(|n| field(text, n).to_string())
        .collect()
}

#[test]
fn a_record_without_a_changeset_becomes_complete_or_the_given_value() {
    for p in [Proj::valid(), Proj::valid_crlf()] {
        let before = p.read(ADR);
        refresh(&p, json!({"record": "ADR-0001"})).expect("refresh");
        let after = p.read(ADR);
        assert_eq!(
            field(&after, "Implementation"),
            "complete — a naming choice"
        );
        assert_eq!(
            field(&after, "Implementers"),
            "none yet",
            "only written when given"
        );
        assert_eq!(events(&before), events(&after));
        assert_eq!(field(&after, "Status"), "Accepted");
        if before.contains("\r\n") {
            assert!(!after.replace("\r\n", "").contains('\n'));
        }
        assert_clean(&p);

        refresh(
            &p,
            json!({"record": "ADR-0001", "implementation": "abandoned", "implementation_scope": "the naming was dropped", "implementers": "Sample Implementer"}),
        )
        .expect("abandoned");
        let after = p.read(ADR);
        assert_eq!(
            field(&after, "Implementation"),
            "abandoned — the naming was dropped"
        );
        assert_eq!(field(&after, "Implementers"), "Sample Implementer");
        assert_eq!(events(&before), events(&after));
        assert_clean(&p);
    }
}

#[test]
fn remaining_edits_refuse_or_keep_partial() {
    let p = Proj::valid();
    let before = p.snapshot();
    let err = refresh(&p, json!({"record": "RFC-0002"})).expect_err("nothing promoted");
    assert_eq!(err.code, ErrCode::InvariantViolation);
    assert!(
        err.message.contains("none has been promoted")
            && err.message.contains("palette_changeset_promote"),
        "{}",
        err.message
    );
    let err = refresh(
        &p,
        json!({"record": "RFC-0002", "implementation": "complete"}),
    )
    .expect_err("complete with edits remaining");
    assert_eq!(err.code, ErrCode::InvalidParams);
    assert!(
        err.message.contains("palette_changeset_promote"),
        "{}",
        err.message
    );
    assert_eq!(p.snapshot(), before);

    // A given value other than complete is the caller's statement, edits or not.
    let q = Proj::valid();
    refresh(
        &q,
        json!({"record": "RFC-0002", "implementation": "in-progress"}),
    )
    .expect("in-progress");
    assert!(field(&q.read(RFC2), "Implementation").starts_with("in-progress — "));
    refresh(
        &q,
        json!({"record": "RFC-0002", "implementation": "abandoned"}),
    )
    .expect("abandoned");
    assert!(field(&q.read(RFC2), "Implementation").starts_with("abandoned — "));
    assert_clean(&q);

    // Promote one of the two edits: the record is partial.
    records::changeset_promote(&p.ctx, params(&p, json!({
        "record": "RFC-0002", "edits": [{"kind": "replace", "document": "spec/thing.rst", "target": "Contract"}],
        "implementation": "partial", "implementers": "S", "verification": "static", "verification_note": "n"
    }))).expect("partial promotion");
    let promoted = p.read(RFC2);
    refresh(&p, json!({"record": "RFC-0002", "implementers": "T"})).expect("refresh partial");
    let after = p.read(RFC2);
    assert!(field(&after, "Implementation").starts_with("partial — "));
    assert_eq!(field(&after, "Implementers"), "T");
    assert_eq!(events(&promoted), events(&after));
    let err = refresh(
        &p,
        json!({"record": "RFC-0002", "implementation": "complete"}),
    )
    .expect_err("still one edit");
    assert_eq!(err.code, ErrCode::InvalidParams);
    assert_clean(&p);
}

#[test]
fn a_record_named_in_a_whole_supersedes_entry_becomes_superseded() {
    for p in [Proj::valid(), Proj::valid_crlf()] {
        p.replace(
            "docs/rfc/rfc-0003-gamma.rst",
            ":Supersedes: none",
            ":Supersedes: RFC-0001 (the alpha contract)",
        );
        let path = "docs/rfc/rfc-0001-alpha.rst";
        let before = p.read(path);
        refresh(&p, json!({"record": "RFC-0001"})).expect("refresh");
        let after = p.read(path);
        assert_eq!(field(&after, "Status"), "Superseded");
        assert_eq!(
            field(&after, "Implementation"),
            "complete — the alpha contract"
        );
        assert_eq!(events(&before), events(&after));
        assert!(
            !has(&p.lint(), "P005", "rfc-0001", ""),
            "{}",
            show(&p.lint())
        );
        // A partial entry leaves the status.
        let q = Proj::valid();
        q.replace(
            "docs/rfc/rfc-0003-gamma.rst",
            ":Supersedes: none",
            ":Supersedes: RFC-0001 (in part: the open operation)",
        );
        refresh(&q, json!({"record": "RFC-0001"})).expect("refresh");
        assert_eq!(field(&q.read(path), "Status"), "Accepted");
    }
}

#[test]
fn draft_unknown_and_bad_values_are_refused() {
    let p = Proj::valid();
    let before = p.snapshot();
    let err = refresh(&p, json!({"record": "RFC-0003"})).expect_err("draft");
    assert_eq!(err.code, ErrCode::InvariantViolation);
    assert!(err.message.contains("Draft"), "{}", err.message);
    let err = refresh(&p, json!({"record": "RFC-0099"})).expect_err("unknown");
    assert_eq!(err.code, ErrCode::NotFound);
    let err = refresh(&p, json!({"record": "ADR-0001", "implementation": "done"}))
        .expect_err("bad value");
    assert_eq!(err.code, ErrCode::InvalidParams);
    let dry = refresh(&p, json!({"record": "ADR-0001", "dry_run": true})).expect("dry run");
    assert!(dry.dry_run && dry.diff.contains("+:Implementation: complete"));
    assert_eq!(p.snapshot(), before);
}
