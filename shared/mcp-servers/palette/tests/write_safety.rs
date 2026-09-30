//! Transaction guarantees of the write tools: dry run, a failure partway leaving every
//! file unchanged, refusal of unparseable files, the project lock, conflicts, and each
//! tool's documented refusals.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use common::*;
use palette_server::errors::ErrCode;
use palette_server::ops::{Ctx, backlog, init, records, state};
use palette_server::project::Roots;
use palette_server::vfs::ProjectLock;
use serde_json::json;

struct Case {
    name: &'static str,
    setup: fn(&Proj),
    call: fn(&Proj, bool) -> ToolResult,
    corrupt: &'static str,
}

fn ok(r: ToolResult) {
    r.unwrap_or_else(|e| panic!("setup failed: {e}"));
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "backlog_add",
            setup: |_| {},
            call: |p, d| {
                backlog::backlog_add(
                    &p.ctx,
                    params(
                        p,
                        json!({"title": "New", "type": "bug", "source": "user", "dry_run": d, "body": "Body."}),
                    ),
                )
            },
            corrupt: "_palette/backlog.rst",
        },
        Case {
            name: "backlog_update",
            setup: |_| {},
            call: |p, d| {
                backlog::backlog_update(
                    &p.ctx,
                    params(
                        p,
                        json!({"item": "B-4", "status": "approved", "dry_run": d}),
                    ),
                )
            },
            corrupt: "_palette/backlog.rst",
        },
        Case {
            name: "phase_open",
            setup: |p| {
                ok(backlog::phase_close(
                    &p.ctx,
                    params(
                        p,
                        json!({"phase": 1, "items": [{"item": "B-1", "result": "done", "outcome": "RFC-0002"}]}),
                    ),
                ))
            },
            call: |p, d| {
                backlog::phase_open(
                    &p.ctx,
                    params(
                        p,
                        json!({"title": "Next", "goal": "Goal.", "reason": "Reason.", "exit_criteria": ["Checked."], "items": ["B-2"], "dry_run": d}),
                    ),
                )
            },
            corrupt: "_palette/backlog.rst",
        },
        Case {
            name: "deliverable_create",
            setup: |p| {
                ok(backlog::backlog_update(
                    &p.ctx,
                    params(p, json!({"item": "B-2", "status": "in-phase-1"})),
                ))
            },
            call: |p, d| {
                backlog::deliverable_create(
                    &p.ctx,
                    params(
                        p,
                        json!({"item": "B-2", "title": "Second", "what_and_why": "Why.", "done_when": ["Done."], "dry_run": d}),
                    ),
                )
            },
            corrupt: "_palette/backlog.rst",
        },
        Case {
            name: "deliverable_update",
            setup: |_| {},
            call: |p, d| {
                backlog::deliverable_update(
                    &p.ctx,
                    params(
                        p,
                        json!({"item": "B-1", "what_and_why": "New why.", "dry_run": d}),
                    ),
                )
            },
            corrupt: "_palette/phase-1/deliverables/deliverable-1-first-thing.rst",
        },
        Case {
            name: "phase_close",
            setup: |_| {},
            call: |p, d| {
                backlog::phase_close(
                    &p.ctx,
                    params(
                        p,
                        json!({"phase": 1, "items": [{"item": "B-1", "result": "done", "outcome": "RFC-0002"}], "dry_run": d}),
                    ),
                )
            },
            corrupt: "_palette/state.rst",
        },
        Case {
            name: "state_record",
            setup: |_| {},
            call: |p, d| {
                state::state_record(
                    &p.ctx,
                    params(
                        p,
                        json!({"kind": "question", "text": "Why?", "affects": "B-1", "dry_run": d}),
                    ),
                )
            },
            corrupt: "_palette/state.rst",
        },
        Case {
            name: "state_resolve",
            setup: |_| {},
            call: |p, d| {
                state::state_resolve(
                    &p.ctx,
                    params(p, json!({"id": "X-1", "resolution": "fixed", "dry_run": d})),
                )
            },
            corrupt: "_palette/state.rst",
        },
        Case {
            name: "record_create",
            setup: |_| {},
            call: |p, d| {
                records::record_create(
                    &p.ctx,
                    params(
                        p,
                        json!({
                            "kind": "adr", "title": "Other naming", "authors": "A", "within": "RFC-0001 (names)", "description": "Renames.",
                            "supersedes": [{"record": "ADR-0001", "note": "the first naming choice"}], "dry_run": d
                        }),
                    ),
                )
            },
            corrupt: "docs/adr/adr-0001-naming.rst",
        },
        Case {
            name: "record_update",
            setup: |_| {},
            call: |p, d| {
                records::record_update(
                    &p.ctx,
                    params(
                        p,
                        json!({"record": "RFC-0003", "description": "A new description.", "dry_run": d}),
                    ),
                )
            },
            corrupt: "docs/rfc/rfc-0003-gamma.rst",
        },
        Case {
            name: "changeset_edit",
            setup: |_| {},
            call: |p, d| {
                records::changeset_edit(
                    &p.ctx,
                    params(
                        p,
                        json!({
                            "record": "RFC-0002", "action": "replace", "kind": "insert_into", "document": "spec/thing.rst", "target": "Contract",
                            "body": "Rules\n^^^^^\n\nChanged rules text.", "dry_run": d
                        }),
                    ),
                )
            },
            corrupt: "docs/changeset/rfc-0002.rst",
        },
        Case {
            name: "changeset_promote",
            setup: |_| {},
            call: |p, d| {
                records::changeset_promote(
                    &p.ctx,
                    params(
                        p,
                        json!({
                            "record": "RFC-0002", "implementation": "complete", "implementers": "Sample", "verification": "static", "verification_note": "read", "dry_run": d
                        }),
                    ),
                )
            },
            corrupt: "docs/spec/thing.rst",
        },
        Case {
            name: "layout_set",
            setup: |_| {},
            call: |p, d| {
                init::layout_set(
                    &p.ctx,
                    params(
                        p,
                        json!({"family": "rfc", "placement": "internal", "confirmed_by_user": true, "dry_run": d}),
                    ),
                )
            },
            corrupt: "docs/rfc/rfc-0003-gamma.rst",
        },
    ]
}

fn prepared(c: &Case) -> Proj {
    let p = Proj::valid();
    (c.setup)(&p);
    p
}

#[test]
fn dry_run_writes_nothing_and_returns_the_diff_of_the_real_run() {
    for c in cases() {
        let p = prepared(&c);
        let before = p.snapshot();
        let dry = (c.call)(&p, true).unwrap_or_else(|e| panic!("{}: {e}", c.name));
        assert_eq!(p.snapshot(), before, "{}: a dry run must not write", c.name);
        assert!(
            dry.dry_run && !dry.diff.is_empty() && !dry.files.is_empty(),
            "{}",
            c.name
        );
        let q = prepared(&c);
        let real = (c.call)(&q, false).unwrap_or_else(|e| panic!("{}: {e}", c.name));
        assert!(!real.dry_run);
        assert_eq!(
            real.diff, dry.diff,
            "{}: dry run and real run must report the same diff",
            c.name
        );
        assert_eq!(real.files, dry.files, "{}", c.name);
        assert_ne!(q.snapshot(), before, "{}: the real run must write", c.name);
    }
}

#[test]
fn init_dry_run_writes_nothing() {
    let p = Proj::empty();
    let r = init::init(&p.ctx, params(&p, json!({"dry_run": true, "name": "Demo"}))).expect("dry");
    assert!(p.files().is_empty());
    assert!(r.dry_run && r.diff.contains("+Layout — Demo"));
}

#[test]
fn a_failure_partway_leaves_every_file_unchanged() {
    for c in cases() {
        let mut p = prepared(&c);
        let count = (c.call)(&p, true)
            .unwrap_or_else(|e| panic!("{}: {e}", c.name))
            .files
            .len();
        let mut steps = vec![count - 1];
        if count > 2 {
            steps.push(1);
        }
        for fail_at in steps {
            let before = p.snapshot();
            let fired = Arc::new(AtomicBool::new(false));
            let f2 = fired.clone();
            p.ctx.commit_hook = Some(Arc::new(move |i| {
                if i == fail_at {
                    f2.store(true, Ordering::SeqCst);
                    Err(std::io::Error::other("injected failure"))
                } else {
                    Ok(())
                }
            }));
            let err = (c.call)(&p, false).expect_err(c.name);
            assert_eq!(err.code, ErrCode::IoError, "{}: {err}", c.name);
            assert!(
                fired.load(Ordering::SeqCst),
                "{}: the failure was not injected at step {fail_at}",
                c.name
            );
            assert_eq!(
                p.snapshot(),
                before,
                "{}: every file must keep its previous content after a failure at step {fail_at}",
                c.name
            );
        }
    }
}

#[test]
fn a_file_that_does_not_parse_is_refused_and_nothing_changes() {
    for c in cases() {
        let p = prepared(&c);
        let mut bytes = std::fs::read(p.path(c.corrupt)).expect("read");
        bytes.extend_from_slice(b"\n\xff\xfe\n");
        std::fs::write(p.path(c.corrupt), bytes).expect("corrupt");
        let before = p.snapshot();
        let err = (c.call)(&p, false).expect_err(c.name);
        assert_eq!(err.code, ErrCode::ParseError, "{}: {err}", c.name);
        assert!(
            err.message
                .contains(c.corrupt.rsplit('/').next().unwrap_or(""))
                || err.message.contains(".rst"),
            "{}: {}",
            c.name,
            err.message
        );
        assert_eq!(p.snapshot(), before, "{}", c.name);
    }
}

#[test]
fn a_structurally_broken_document_is_refused_with_its_line() {
    let p = Proj::valid();
    p.replace("_palette/backlog.rst", "Items\n-----", "Things\n------");
    let before = p.snapshot();
    let err = backlog::backlog_add(
        &p.ctx,
        params(&p, json!({"title": "X", "type": "bug", "source": "user"})),
    )
    .expect_err("parse");
    assert_eq!(err.code, ErrCode::ParseError);
    assert!(
        err.message.starts_with("_palette/backlog.rst:1:"),
        "{}",
        err.message
    );
    assert_eq!(p.snapshot(), before);

    let p = Proj::valid();
    p.replace(
        "_palette/state.rst",
        "Open questions\n--------------\n",
        "Questions\n---------\n",
    );
    let err = state::state_record(
        &p.ctx,
        params(
            &p,
            json!({"kind": "decision", "text": "T", "source": "S", "target": "X"}),
        ),
    )
    .expect_err("parse");
    assert_eq!(err.code, ErrCode::ParseError);
    assert!(err.message.contains("state.rst") && err.message.contains("Open questions"));
}

#[test]
fn the_project_lock_makes_a_second_writer_fail_with_locked() {
    let p = Proj::valid();
    let held = ProjectLock::acquire(
        &p.path("_palette/.palette.lock"),
        std::time::Duration::from_secs(1),
    )
    .expect("lock");
    let before = p.snapshot();
    let err = backlog::backlog_add(
        &p.ctx,
        params(&p, json!({"title": "X", "type": "bug", "source": "user"})),
    )
    .expect_err("locked");
    assert_eq!(err.code, ErrCode::Locked);
    assert_eq!(p.snapshot(), before);
    drop(held);
    backlog::backlog_add(
        &p.ctx,
        params(&p, json!({"title": "X", "type": "bug", "source": "user"})),
    )
    .expect("free");
}

#[test]
fn a_file_changed_between_read_and_write_is_a_conflict() {
    let mut p = Proj::valid();
    let target = p.path("_palette/backlog.rst");
    // The clock is read after the project is loaded and before the commit, so it stands in
    // for a hand edit that lands in between.
    p.ctx.clock = Arc::new(move || {
        std::fs::write(&target, "Backlog — hand edited\n").expect("hand edit");
        TODAY.to_string()
    });
    let err = backlog::backlog_add(
        &p.ctx,
        params(&p, json!({"title": "X", "type": "bug", "source": "user"})),
    )
    .expect_err("conflict");
    assert_eq!(err.code, ErrCode::Conflict);
    assert_eq!(p.read("_palette/backlog.rst"), "Backlog — hand edited\n");
}

#[test]
fn hand_edits_elsewhere_in_a_file_are_kept_byte_for_byte() {
    let p = Proj::valid();
    p.replace(
        "_palette/backlog.rst",
        "Build the first thing.",
        "Build the first thing.  (hand edit, two spaces)",
    );
    let before = p.read("_palette/backlog.rst");
    backlog::backlog_update(
        &p.ctx,
        params(&p, json!({"item": "B-4", "status": "approved"})),
    )
    .expect("update");
    let after = p.read("_palette/backlog.rst");
    assert_eq!(
        before.replacen(":Status: proposed", ":Status: approved", 1),
        after
    );
}

#[test]
fn a_lint_error_in_the_result_blocks_the_write() {
    let p = Proj::valid();
    let before = p.snapshot();
    // The record name is fine but the relation points at a newer record: P005.
    let err = records::record_update(
        &p.ctx,
        params(
            &p,
            json!({
                "record": "RFC-0003", "depends": [{"record": "RFC-0009", "note": "does not exist"}]
            }),
        ),
    )
    .expect_err("lint");
    assert_eq!(err.code, ErrCode::InvariantViolation);
    assert!(err.message.contains("P004"), "{}", err.message);
    assert_eq!(p.snapshot(), before);
}

fn refuses(p: &Proj, code: ErrCode, needle: &str, r: ToolResult) {
    let before = p.snapshot();
    let err = r.expect_err(needle);
    assert_eq!(err.code, code, "{err}");
    assert!(
        err.message.contains(needle),
        "expected {needle:?} in {:?}",
        err.message
    );
    assert_eq!(p.snapshot(), before, "a refusal must not change anything");
}

#[test]
fn documented_refusals() {
    let p = Proj::valid();
    let c = &p.ctx;
    refuses(
        &p,
        ErrCode::InvalidParams,
        "Type",
        backlog::backlog_add(
            c,
            params(&p, json!({"title": "X", "type": "epic", "source": "user"})),
        ),
    );
    refuses(
        &p,
        ErrCode::NotFound,
        "B-9",
        backlog::backlog_add(
            c,
            params(
                &p,
                json!({"title": "X", "type": "bug", "source": "user", "depends": ["B-9"]}),
            ),
        ),
    );
    refuses(
        &p,
        ErrCode::InvariantViolation,
        "cannot move from proposed to done",
        backlog::backlog_update(c, params(&p, json!({"item": "B-4", "status": "done"}))),
    );
    refuses(
        &p,
        ErrCode::InvariantViolation,
        "cannot move from proposed to in-phase-1",
        backlog::backlog_update(
            c,
            params(&p, json!({"item": "B-4", "status": "in-phase-1"})),
        ),
    );
    refuses(
        &p,
        ErrCode::InvariantViolation,
        "cannot move from approved to done",
        backlog::backlog_update(c, params(&p, json!({"item": "B-2", "status": "done"}))),
    );
    refuses(
        &p,
        ErrCode::NotFound,
        "B-9",
        backlog::backlog_update(c, params(&p, json!({"item": "B-9", "status": "dropped"}))),
    );
    refuses(
        &p,
        ErrCode::InvariantViolation,
        "still active",
        backlog::phase_open(
            c,
            params(
                &p,
                json!({"title": "T", "goal": "G", "reason": "R", "exit_criteria": ["x"]}),
            ),
        ),
    );
    refuses(
        &p,
        ErrCode::InvalidParams,
        "still holds B-1",
        backlog::phase_close(c, params(&p, json!({"phase": 1, "items": []}))),
    );
    refuses(
        &p,
        ErrCode::InvalidParams,
        "outcome pointer",
        backlog::phase_close(
            c,
            params(
                &p,
                json!({"phase": 1, "items": [{"item": "B-1", "result": "done"}]}),
            ),
        ),
    );
    refuses(
        &p,
        ErrCode::InvariantViolation,
        "already has a deliverable",
        backlog::deliverable_create(
            c,
            params(
                &p,
                json!({"item": "B-1", "title": "T", "what_and_why": "W", "done_when": ["d"]}),
            ),
        ),
    );
    refuses(
        &p,
        ErrCode::InvariantViolation,
        "approved",
        backlog::deliverable_create(
            c,
            params(
                &p,
                json!({"item": "B-2", "title": "T", "what_and_why": "W", "done_when": ["d"]}),
            ),
        ),
    );
    refuses(
        &p,
        ErrCode::InvalidParams,
        "kind must be",
        state::state_record(c, params(&p, json!({"kind": "note", "text": "T"}))),
    );
    refuses(
        &p,
        ErrCode::InvalidParams,
        "evidence must be",
        state::state_record(
            c,
            params(
                &p,
                json!({"kind": "discrepancy", "text": "T", "evidence": "guess"}),
            ),
        ),
    );
    refuses(
        &p,
        ErrCode::InvalidParams,
        "can be resolved as graduated",
        state::state_resolve(c, params(&p, json!({"id": "D-1", "resolution": "fixed"}))),
    );
    refuses(
        &p,
        ErrCode::NotFound,
        "D-9",
        state::state_resolve(
            c,
            params(
                &p,
                json!({"id": "D-9", "resolution": "graduated", "written": "RFC-0001"}),
            ),
        ),
    );
    refuses(
        &p,
        ErrCode::NotFound,
        "does not exist",
        state::state_resolve(
            c,
            params(
                &p,
                json!({"id": "D-1", "resolution": "graduated", "written": "docs/nowhere.rst"}),
            ),
        ),
    );
    refuses(
        &p,
        ErrCode::InvariantViolation,
        "already a pointer",
        state::state_resolve(
            c,
            params(
                &p,
                json!({"id": "D-2", "resolution": "graduated", "written": "RFC-0001"}),
            ),
        ),
    );
    refuses(
        &p,
        ErrCode::InvalidParams,
        "not a section",
        records::record_create(
            c,
            params(
                &p,
                json!({"kind": "rfc", "title": "T", "authors": "A", "areas": "x", "description": "D", "sections": {"Nope": "x"}}),
            ),
        ),
    );
    refuses(
        &p,
        ErrCode::InvariantViolation,
        "only an Accepted record",
        records::record_create(
            c,
            params(
                &p,
                json!({"kind": "rfc", "title": "T", "authors": "A", "areas": "x", "description": "D", "supersedes": [{"record": "RFC-0003", "note": "n"}]}),
            ),
        ),
    );
    refuses(
        &p,
        ErrCode::InvariantViolation,
        "clarification: true",
        records::record_update(
            c,
            params(&p, json!({"record": "RFC-0002", "description": "Changed."})),
        ),
    );
    refuses(
        &p,
        ErrCode::InvalidParams,
        "accepted_by",
        records::record_update(
            c,
            params(&p, json!({"record": "RFC-0003", "status": "Proposed"})),
        )
        .and_then(|_| {
            records::record_update(
                c,
                params(&p, json!({"record": "RFC-0003", "status": "Accepted"})),
            )
        }),
    );
    refuses(
        &p,
        ErrCode::InvariantViolation,
        "Superseded is set only",
        records::record_update(
            c,
            params(&p, json!({"record": "RFC-0003", "status": "Superseded"})),
        ),
    );
    refuses(
        &p,
        ErrCode::InvariantViolation,
        "cannot move from Accepted to Proposed",
        records::record_update(
            c,
            params(&p, json!({"record": "RFC-0002", "status": "Proposed"})),
        ),
    );
    refuses(
        &p,
        ErrCode::InvalidParams,
        "already has",
        records::changeset_edit(
            c,
            params(
                &p,
                json!({
                    "record": "RFC-0002", "action": "add", "kind": "replace", "document": "spec/thing.rst", "target": "Contract", "body": "Contract\n^^^^^^^^\n\nx"
                }),
            ),
        ),
    );
    refuses(
        &p,
        ErrCode::InvariantViolation,
        "P007",
        records::changeset_edit(
            c,
            params(
                &p,
                json!({
                    "record": "RFC-0002", "action": "add", "kind": "delete", "document": "spec/thing.rst", "target": "Nowhere"
                }),
            ),
        ),
    );
    refuses(
        &p,
        ErrCode::InvalidParams,
        "1 edit(s) remain",
        records::changeset_promote(
            c,
            params(
                &p,
                json!({
                    "record": "RFC-0002", "edits": [{"kind": "replace", "document": "spec/thing.rst", "target": "Contract"}],
                    "implementation": "complete", "implementers": "S", "verification": "static", "verification_note": "n"
                }),
            ),
        ),
    );
    refuses(
        &p,
        ErrCode::InvariantViolation,
        "only an Accepted record",
        records::changeset_promote(
            c,
            params(
                &p,
                json!({
                    "record": "RFC-0003", "implementation": "partial", "implementers": "S", "verification": "static", "verification_note": "n"
                }),
            ),
        ),
    );
    refuses(
        &p,
        ErrCode::InvalidParams,
        "confirmed_by_user",
        init::layout_set(
            c,
            params(&p, json!({"family": "rfc", "placement": "internal"})),
        ),
    );
    refuses(
        &p,
        ErrCode::InvalidParams,
        "already placed",
        init::layout_set(
            c,
            params(
                &p,
                json!({"family": "rfc", "placement": "docs/rfc", "confirmed_by_user": true}),
            ),
        ),
    );
    refuses(
        &p,
        ErrCode::InvalidParams,
        "leaves the project",
        init::layout_set(
            c,
            params(
                &p,
                json!({"family": "rfc", "placement": "../x", "confirmed_by_user": true}),
            ),
        ),
    );
    refuses(
        &p,
        ErrCode::AlreadyInitialized,
        "already has",
        init::init(c, params(&p, json!({}))),
    );
}

#[test]
fn layout_set_refuses_a_destination_that_already_holds_a_file() {
    let p = Proj::valid();
    p.write("_palette/rfc/rfc-0001-alpha.rst", "occupied\n");
    let before = p.snapshot();
    let err = init::layout_set(
        &p.ctx,
        params(
            &p,
            json!({"family": "rfc", "placement": "internal", "confirmed_by_user": true}),
        ),
    )
    .expect_err("occupied");
    assert_eq!(err.code, ErrCode::InvariantViolation);
    assert!(err.message.contains("already exists"));
    assert_eq!(p.snapshot(), before);
}

#[test]
fn project_resolution_applies_to_every_write_tool() {
    let p = Proj::valid();
    let other = tempfile::tempdir().expect("other");
    // Outside the roots.
    let out = backlog::backlog_add(&p.ctx, serde_json::from_value(json!({"project": other.path().to_string_lossy(), "title": "X", "type": "bug", "source": "user"})).expect("params"))
        .expect_err("outside");
    assert_eq!(out.code, ErrCode::OutsideRoots);
    // No roots at all.
    let mut none = p.ctx.clone();
    none.roots = Roots::default();
    let err = backlog::backlog_add(
        &none,
        params(&p, json!({"title": "X", "type": "bug", "source": "user"})),
    )
    .expect_err("no roots");
    assert_eq!(err.code, ErrCode::NoProjectRoot);
    // Inside a root but not initialized.
    let sub = p.path("docs");
    let err = backlog::backlog_add(&p.ctx, serde_json::from_value(json!({"project": sub.to_string_lossy(), "title": "X", "type": "bug", "source": "user"})).expect("params"))
        .expect_err("no layout");
    assert_eq!(err.code, ErrCode::NoLayout);
    // A relative path is refused.
    let err = backlog::backlog_add(
        &p.ctx,
        serde_json::from_value(
            json!({"project": "docs", "title": "X", "type": "bug", "source": "user"}),
        )
        .expect("params"),
    )
    .expect_err("relative");
    assert_eq!(err.code, ErrCode::InvalidParams);
}

#[test]
fn ctx_is_cloneable_and_defaults_to_a_ten_second_lock() {
    let c = Ctx::new(Roots::default());
    assert_eq!(c.lock_timeout, std::time::Duration::from_secs(10));
    assert_eq!(c.clone().lock_timeout, c.lock_timeout);
}

#[cfg(unix)]
#[test]
fn symbolic_links_in_a_write_path_are_rejected() {
    let p = Proj::valid();
    let outside = tempfile::tempdir().expect("outside");
    std::fs::remove_dir_all(p.path("docs/adr")).expect("rm");
    std::os::unix::fs::symlink(outside.path(), p.path("docs/adr")).expect("symlink");
    let err = records::record_create(&p.ctx, params(&p, json!({
        "kind": "adr", "title": "Through a link", "authors": "A", "within": "RFC-0001 (names)", "description": "D"
    }))).expect_err("symlink");
    assert_eq!(err.code, ErrCode::InvariantViolation);
    assert!(err.message.contains("symbolic link"), "{}", err.message);
    assert!(
        std::fs::read_dir(outside.path())
            .expect("dir")
            .next()
            .is_none()
    );
}

#[test]
fn init_leaves_nothing_behind_when_it_fails_partway() {
    let mut p = Proj::empty();
    p.ctx.commit_hook = Some(Arc::new(|i| {
        if i == 1 {
            Err(std::io::Error::other("injected failure"))
        } else {
            Ok(())
        }
    }));
    let err = init::init(
        &p.ctx,
        params(
            &p,
            json!({"name": "Demo", "placements": {"rfc": "docs/rfc"}}),
        ),
    )
    .expect_err("failure");
    assert_eq!(err.code, ErrCode::IoError);
    assert!(p.files().is_empty(), "no file may remain: {:?}", p.files());
    assert!(
        !p.path("_palette").exists(),
        "the folder init created is removed too"
    );
}

#[test]
fn generate_fails_partway_without_leaving_a_partial_result() {
    let mut p = Proj::valid();
    for f in [
        "docs/rfc/index.rst",
        "docs/adr/index.rst",
        "docs/staging/spec/thing.rst",
    ] {
        p.remove(f);
    }
    let before = p.snapshot();
    p.ctx.commit_hook = Some(Arc::new(|i| {
        if i == 1 {
            Err(std::io::Error::other("injected failure"))
        } else {
            Ok(())
        }
    }));
    let err = palette_server::ops::generate(&p.ctx, &p.arg()).expect_err("failure");
    assert_eq!(err.code, ErrCode::IoError);
    assert_eq!(p.snapshot(), before);
    p.ctx.commit_hook = None;
    let ok = palette_server::ops::generate(&p.ctx, &p.arg()).expect("generate");
    assert_eq!(ok.files.len(), 3);
    assert!(p.errors().is_empty());
}
