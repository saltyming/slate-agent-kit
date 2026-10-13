//! Regression guards for five review findings on RFC-0019, RFC-0020 and RFC-0021: a
//! staging placement equal to another family's, case aliases of a changeset target, a
//! dropped staging placement under the write tools, document rules on the mirror of a
//! document outside the families, and an `Acceptance` field outside `Records`.

mod common;

use common::*;
use palette_server::docs::{Role, Snapshot};
use palette_server::errors::ErrCode;
use palette_server::ops::{self, records};
use palette_server::vfs::DiskSource;
use serde_json::json;

fn edit(p: &Proj, kind: &str, document: &str, target: &str, body: &str) -> ToolResult {
    records::changeset_edit(
        &p.ctx,
        params(
            p,
            json!({"record": "RFC-0002", "action": "add", "kind": kind, "document": document, "target": target, "body": body}),
        ),
    )
}

fn promote(p: &Proj) -> ToolResult {
    records::changeset_promote(
        &p.ctx,
        params(
            p,
            json!({"record": "RFC-0002", "implementation": "complete", "implementers": "S", "verification": "static", "verification_note": "n"}),
        ),
    )
}

fn assert_refused_by_layout(p: &Proj) {
    let before = p.snapshot();
    let err = ops::generate(&p.ctx, &p.arg()).expect_err("generate refused");
    assert_eq!(err.code, ErrCode::InvariantViolation, "{}", err.message);
    assert!(
        err.message.contains("P012") && err.message.contains("staging"),
        "{}",
        err.message
    );
    let err = records::record_update(
        &p.ctx,
        params(
            p,
            json!({"record": "RFC-0003", "title": "Gamma proposal again"}),
        ),
    )
    .expect_err("write tool refused");
    assert_eq!(err.code, ErrCode::InvariantViolation, "{}", err.message);
    assert_eq!(p.snapshot(), before, "nothing written");
}

// ── 1. staging placed where another family is ────────────────────────────

#[test]
fn staging_placed_at_another_familys_folder_is_dropped_not_the_family() {
    for staging_first in [true, false] {
        for p in [Proj::valid(), Proj::valid_crlf()] {
            let layout = p.read("_palette/layout.rst");
            let eol = if layout.contains("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            let mut text = layout.replace(
                &format!(":staging: docs/staging{eol}"),
                if staging_first { "" } else { "PLACEHOLDER" },
            );
            if staging_first {
                text = text.replace(
                    &format!(":changeset: docs/changeset{eol}"),
                    &format!(":changeset: docs/changeset{eol}:staging: docs/spec{eol}"),
                );
            } else {
                text = text.replace("PLACEHOLDER", "").replace(
                    &format!(":spec: docs/spec{eol}"),
                    &format!(":spec: docs/spec{eol}:staging: docs/spec{eol}"),
                );
            }
            p.write("_palette/layout.rst", &text);
            let snap = Snapshot::load(&DiskSource, &p.root).expect("load");
            let role = snap
                .files
                .iter()
                .find(|f| f.rel == "docs/spec/thing.rst")
                .map(|f| f.role);
            assert_eq!(role, Some(Role::Spec), "staging_first={staging_first}");
            assert!(
                has(
                    &p.lint(),
                    "P012",
                    "layout.rst",
                    "both placed at `docs/spec`"
                ),
                "{}",
                show(&p.lint())
            );
            assert_refused_by_layout(&p);
            assert!(p.exists("docs/spec/thing.rst"));
        }
    }
}

// ── 2. case aliases of a target ──────────────────────────────────────────

#[test]
fn a_target_spelled_with_another_case_than_on_disk_is_refused() {
    let p = Proj::valid();
    p.write(
        "docs/guide.rst",
        "Guide\n=====\n\nBuild\n-----\n\nWith make.\n",
    );
    let before = p.snapshot();
    let err = edit(&p, "delete", "DOCS/guide.rst", "Build", "").expect_err("alias");
    assert_eq!(err.code, ErrCode::InvalidParams, "{}", err.message);
    // Refused the same way on a case-sensitive file system, where `DOCS/` does not
    // exist, and on a case-insensitive one, where it names the same file.
    let needle = "is spelled `docs/guide.rst` on disk";
    assert!(err.message.contains(needle), "{}", err.message);
    assert_eq!(p.snapshot(), before);
    // The same spelling in a hand-written changeset is P007 and never applied.
    p.mutate("docs/changeset/rfc-0002.rst", |s| {
        format!("{s}\nDOCS/guide.rst\n--------------\n\nDelete: Build\n~~~~~~~~~~~~~\n")
    });
    assert!(
        has(&p.lint(), "P007", "changeset/rfc-0002.rst", needle),
        "{}",
        show(&p.lint())
    );
    // The exact spelling is accepted.
    let q = Proj::valid();
    q.write(
        "docs/guide.rst",
        "Guide\n=====\n\nBuild\n-----\n\nWith make.\n",
    );
    edit(&q, "delete", "docs/guide.rst", "Build", "").expect("exact spelling");
}

// ── 3. a dropped staging placement under the write tools ─────────────────

#[test]
fn a_staging_placement_that_holds_families_refuses_every_write() {
    for p in [Proj::valid(), Proj::valid_crlf()] {
        p.replace(
            "_palette/layout.rst",
            ":staging: docs/staging",
            ":staging: docs",
        );
        assert_refused_by_layout(&p);
        assert!(!p.exists("_palette/staging"));
    }
}

#[test]
fn an_invalid_or_missing_staging_line_refuses_every_write() {
    let p = Proj::valid();
    p.replace(
        "_palette/layout.rst",
        ":staging: docs/staging",
        ":staging: ../out",
    );
    assert_refused_by_layout(&p);
    let q = Proj::valid();
    q.replace("_palette/layout.rst", ":staging: docs/staging\n", "");
    assert_refused_by_layout(&q);
}

// ── 4. document rules on the mirror of a document outside the families ──

#[test]
fn an_underscore_named_document_is_edited_staged_and_promoted() {
    for p in [Proj::valid(), Proj::valid_crlf()] {
        let eol = if p.read("_palette/layout.rst").contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        p.write(
            "docs/coding_style.rst",
            &"Coding style\n============\n\nNaming\n------\n\nShort names.\n".replace('\n', eol),
        );
        edit(
            &p,
            "replace_body",
            "docs/coding_style.rst",
            "Naming",
            "Naming\n^^^^^^\n\nShort names that say what they hold.",
        )
        .expect("edit");
        assert!(p.exists("docs/staging/coding_style.rst"));
        let f = p.lint();
        assert!(
            !f.iter().any(|x| x.file.contains("coding_style")),
            "{}",
            show(&f)
        );
        promote(&p).expect("promote");
        assert!(
            p.read("docs/coding_style.rst")
                .contains("say what they hold")
        );
        assert!(!p.exists("docs/staging/coding_style.rst"));
        assert!(p.errors().is_empty(), "{}", show(&p.errors()));
    }
}

// ── 5. `Acceptance` outside the Records section ─────────────────────────

#[test]
fn acceptance_outside_the_records_section_is_p002() {
    for layout in [true, false] {
        for p in [Proj::valid(), Proj::valid_crlf()] {
            if !layout {
                std::fs::remove_dir_all(p.path("_palette")).expect("rm _palette");
            }
            let eol = if p.read("docs/contributing.rst").contains("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            p.replace(
                "docs/contributing.rst",
                &format!(":Date: 2026-01-01{eol}"),
                &format!(":Date: 2026-01-01{eol}:Acceptance: who-and-time{eol}"),
            );
            assert!(
                has(
                    &p.lint(),
                    "P002",
                    "docs/contributing.rst",
                    "`:Acceptance:` belongs in the `Records` section"
                ),
                "layout={layout}: {}",
                show(&p.lint())
            );
        }
    }
}

#[test]
fn acceptance_in_another_section_is_p002() {
    let p = Proj::valid();
    p.replace(
        "docs/contributing.rst",
        "Conventional Commits, unsigned; one change per commit.",
        ":Acceptance: who\n\nConventional Commits, unsigned; one change per commit.",
    );
    assert!(
        has(
            &p.lint(),
            "P002",
            "docs/contributing.rst",
            "`:Acceptance:` belongs in the `Records` section"
        ),
        "{}",
        show(&p.lint())
    );
    // In its section it is not reported.
    let q = Proj::valid();
    q.replace(
        "docs/contributing.rst",
        ":Amends: none\n",
        ":Amends: none\n:Acceptance: who\n",
    );
    assert!(q.errors().is_empty(), "{}", show(&q.errors()));
}
