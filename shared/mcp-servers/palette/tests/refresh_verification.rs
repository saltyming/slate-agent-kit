//! `palette_record_refresh` writes `Verification` (RFC-0024, B-9): the kind and a
//! one-line note together, through the writer promotion uses; both absent keeps the
//! field; one without the other, an empty note or another kind is `invalid_params`.
//! `Accepted`, `Revised` and the body never change.

mod common;

use common::*;
use palette_server::errors::ErrCode;
use palette_server::ops::records;
use serde_json::{Value, json};

const ADR: &str = "docs/adr/adr-0001-naming.rst";

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

/// Every line of `text` except those of the named header fields.
fn without(text: &str, names: &[&str]) -> Vec<String> {
    text.lines()
        .filter(|l| !names.iter().any(|n| l.starts_with(&format!(":{n}: "))))
        .map(String::from)
        .collect()
}

#[test]
fn a_kind_and_a_note_are_written_as_promotion_writes_them() {
    for p in [Proj::valid(), Proj::valid_crlf()] {
        let before = p.read(ADR);
        refresh(
            &p,
            json!({"record": "ADR-0001", "verification": "static", "verification_note": "validate.sh; the naming grep only"}),
        )
        .expect("refresh");
        let after = p.read(ADR);
        assert_eq!(
            field(&after, "Verification"),
            "static — 2026-09-30; validate.sh, the naming grep only"
        );
        assert_eq!(
            field(&after, "Implementation"),
            "complete — a naming choice"
        );
        // Accepted, Revised, the other fields and the body are the lines they were.
        assert_eq!(
            without(&before, &["Verification", "Implementation"]),
            without(&after, &["Verification", "Implementation"])
        );
        if before.contains("\r\n") {
            assert!(!after.replace("\r\n", "").contains('\n'), "CRLF kept");
        }
        let errors = p.errors();
        assert!(errors.is_empty(), "{}", show(&errors));
    }
}

#[test]
fn every_kind_the_template_lists_is_accepted() {
    for kind in ["none", "documentation", "static", "build", "runtime"] {
        let p = Proj::valid();
        refresh(
            &p,
            json!({"record": "ADR-0001", "verification": kind, "verification_note": "n"}),
        )
        .unwrap_or_else(|e| panic!("{kind}: {}", e.message));
        assert_eq!(
            field(&p.read(ADR), "Verification"),
            format!("{kind} — 2026-09-30; n")
        );
        let errors = p.errors();
        assert!(errors.is_empty(), "{kind}: {}", show(&errors));
    }
}

#[test]
fn neither_argument_keeps_the_field() {
    let p = Proj::valid();
    let before = p.read(ADR);
    refresh(&p, json!({"record": "ADR-0001"})).expect("refresh");
    assert_eq!(
        field(&p.read(ADR), "Verification"),
        field(&before, "Verification")
    );
    // A given implementation alongside the note.
    refresh(
        &p,
        json!({"record": "ADR-0001", "implementation": "partial", "verification": "build", "verification_note": "cargo test"}),
    )
    .expect("refresh with both");
    let after = p.read(ADR);
    assert_eq!(field(&after, "Implementation"), "partial — a naming choice");
    assert_eq!(
        field(&after, "Verification"),
        "build — 2026-09-30; cargo test"
    );
}

#[test]
fn one_without_the_other_an_empty_note_and_another_kind_are_invalid_params() {
    let p = Proj::valid();
    let before = p.snapshot();
    for (args, needle) in [
        (
            json!({"verification": "static"}),
            "verification needs verification_note",
        ),
        (
            json!({"verification_note": "validate.sh"}),
            "verification_note needs verification",
        ),
        (
            json!({"verification": "static", "verification_note": "  \n "}),
            "verification_note is empty",
        ),
        (
            json!({"verification": "static", "verification_note": ""}),
            "verification_note is empty",
        ),
        (
            json!({"verification": "tested", "verification_note": "n"}),
            "verification must be none, documentation, static, build, runtime, got `tested`",
        ),
        (
            json!({"verification": "", "verification_note": "n"}),
            "verification must be",
        ),
    ] {
        let mut args = args;
        args["record"] = json!("ADR-0001");
        let err = refresh(&p, args.clone()).expect_err(&args.to_string());
        assert_eq!(err.code, ErrCode::InvalidParams, "{args}: {}", err.message);
        assert!(err.message.contains(needle), "{args}: {}", err.message);
        assert_eq!(p.snapshot(), before, "{args}");
    }
}

#[test]
fn a_draft_record_is_still_refused_first() {
    let p = Proj::valid();
    let err = refresh(
        &p,
        json!({"record": "RFC-0003", "verification": "static", "verification_note": "n"}),
    )
    .expect_err("draft");
    assert_eq!(err.code, ErrCode::InvariantViolation);
    assert!(err.message.contains("RFC-0003 is Draft"), "{}", err.message);
}

#[test]
fn a_dry_run_shows_the_field_and_writes_nothing() {
    let p = Proj::valid();
    let before = p.snapshot();
    let r = refresh(
        &p,
        json!({"record": "ADR-0001", "dry_run": true, "verification": "runtime", "verification_note": "ran the tool"}),
    )
    .expect("dry run");
    assert!(
        r.diff
            .contains("+:Verification: runtime — 2026-09-30; ran the tool"),
        "{}",
        r.diff
    );
    assert_eq!(p.snapshot(), before);
}
