//! The contributing document's `Acceptance` setting (RFC-0020): how P016 reports an
//! `Accepted` value without the time, with and without a layout, on LF and CRLF.

mod common;

use common::*;
use palette_server::lint::{Finding, Severity};
use palette_server::ops::records;
use serde_json::json;

const CONTRIBUTING: &str = "docs/contributing.rst";
const RECORD: &str = "docs/rfc/rfc-0002-beta.rst";

/// The valid fixture (LF or CRLF, with or without `_palette/`) whose RFC-0002 names
/// who accepted it but not when.
fn project(crlf: bool, layout: bool) -> Proj {
    let p = if crlf {
        Proj::valid_crlf()
    } else {
        Proj::valid()
    };
    if !layout {
        std::fs::remove_dir_all(p.path("_palette")).expect("rm _palette");
    }
    p.mutate(RECORD, |s| {
        s.replace(
            ":Accepted: Sample Owner (2026-02-10T00:00Z)",
            ":Accepted: Sample Owner",
        )
    });
    p
}

fn eol(p: &Proj) -> &'static str {
    if p.read(CONTRIBUTING).contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn set_acceptance(p: &Proj, value: &str) {
    let e = eol(p);
    p.replace(
        CONTRIBUTING,
        &format!(":Amends: none{e}"),
        &format!(":Amends: none{e}:Acceptance: {value}{e}"),
    );
}

fn p016(p: &Proj) -> Vec<Finding> {
    p.lint().into_iter().filter(|f| f.rule == "P016").collect()
}

fn each_variant(check: impl Fn(&Proj, &str)) {
    for crlf in [false, true] {
        for layout in [true, false] {
            let label = format!("crlf={crlf} layout={layout}");
            check(&project(crlf, layout), &label);
        }
    }
}

#[test]
fn who_and_time_makes_a_missing_time_an_error() {
    each_variant(|p, label| {
        set_acceptance(p, "who-and-time");
        let f = p016(p);
        assert!(
            f.len() == 1
                && f[0].severity == Severity::Error
                && f[0].file == RECORD
                && f[0].message.contains("`:Acceptance: who-and-time`"),
            "{label}: {}",
            show(&p.lint())
        );
        assert!(
            !has(&p.lint(), "P002", CONTRIBUTING, ""),
            "{label}: {}",
            show(&p.lint())
        );
    });
}

#[test]
fn who_accepts_the_person_alone() {
    each_variant(|p, label| {
        set_acceptance(p, "who");
        assert!(p016(p).is_empty(), "{label}: {}", show(&p.lint()));
        assert!(
            !has(&p.lint(), "P002", CONTRIBUTING, ""),
            "{label}: {}",
            show(&p.lint())
        );
    });
}

#[test]
fn without_the_setting_a_missing_time_stays_a_warning() {
    // No field.
    each_variant(|p, label| {
        let f = p016(p);
        assert!(
            f.len() == 1 && f[0].severity == Severity::Warning,
            "{label}: {}",
            show(&p.lint())
        );
        assert!(
            !has(&p.lint(), "P002", CONTRIBUTING, ""),
            "{label}: {}",
            show(&p.lint())
        );
    });
    // No Records section: the field is not read from another section.
    each_variant(|p, label| {
        let e = eol(p);
        p.mutate(CONTRIBUTING, |s| {
            let cut = s.find(&format!("Records{e}-------")).expect("Records");
            format!(
                "{}Notes{e}-----{e}{e}:Acceptance: who-and-time{e}",
                &s[..cut]
            )
        });
        let f = p016(p);
        assert!(
            f.len() == 1 && f[0].severity == Severity::Warning,
            "{label}: {}",
            show(&p.lint())
        );
    });
    // No contributing document.
    each_variant(|p, label| {
        p.remove(CONTRIBUTING);
        let f = p016(p);
        assert!(
            f.len() == 1 && f[0].severity == Severity::Warning,
            "{label}: {}",
            show(&p.lint())
        );
    });
}

#[test]
fn a_value_or_place_outside_the_template_is_p002() {
    each_variant(|p, label| {
        set_acceptance(p, "always");
        assert!(
            has(
                &p.lint(),
                "P002",
                CONTRIBUTING,
                "`:Acceptance:` has a value outside the allowed values"
            ),
            "{label}: {}",
            show(&p.lint())
        );
        // A value outside the two reads as no setting: the warning stays.
        assert!(p016(p).iter().all(|f| f.severity == Severity::Warning));
    });
    each_variant(|p, label| {
        let e = eol(p);
        p.replace(
            CONTRIBUTING,
            &format!(":Amends: none{e}"),
            &format!(":Acceptance: who{e}:Amends: none{e}"),
        );
        assert!(
            has(&p.lint(), "P002", CONTRIBUTING, "out of order"),
            "{label}: {}",
            show(&p.lint())
        );
    });
}

#[test]
fn acceptance_still_writes_who_and_time_under_who() {
    let p = Proj::valid();
    set_acceptance(&p, "who");
    records::record_create(&p.ctx, params(&p, json!({
        "kind": "rfc", "title": "Epsilon", "authors": "A", "areas": "thing", "description": "D."
    }))).expect("create");
    for status in ["Proposed", "Accepted"] {
        let mut v = json!({"record": "RFC-0004", "status": status});
        if status == "Accepted" {
            v["accepted_by"] = json!("Owner");
        }
        records::record_update(&p.ctx, params(&p, v)).expect(status);
    }
    let rec = p
        .files()
        .into_iter()
        .find(|f| f.starts_with("docs/rfc/rfc-0004-"))
        .expect("rfc-0004");
    assert!(
        p.read(&rec).contains(&format!(":Accepted: Owner ({NOW})")),
        "{}",
        p.read(&rec)
    );
}
