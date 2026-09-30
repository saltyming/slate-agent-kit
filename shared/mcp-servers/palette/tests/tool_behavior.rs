//! Behavior of individual tools: project setup, moving families, backlog and state
//! formats, record creation and update, and the read tools.

mod common;

use common::*;
use palette_server::errors::ErrCode;
use palette_server::ops::{backlog, init, records, state};
use palette_server::tools;
use serde_json::{Value, json};

// ── init ────────────────────────────────────────────────────────────────

#[test]
fn init_creates_a_lint_clean_project() {
    let p = Proj::empty();
    let r = init::init(
        &p.ctx,
        params(
            &p,
            json!({
                "name": "Demo", "checker": "make check",
                "placements": {"rfc": "docs/rfc", "glossary": "docs/glossary.rst"}
            }),
        ),
    )
    .expect("init");
    assert_eq!(p.read("_palette/.gitignore"), "*\n");
    let layout = p.read("_palette/layout.rst");
    assert!(
        layout.starts_with(
            "Layout — Demo\n=============\n\nFamilies\n--------\n\n:backlog: internal\n"
        ),
        "{layout}"
    );
    assert!(
        layout.contains(":rfc: docs/rfc\n")
            && layout.contains(":glossary: docs/glossary.rst\n")
            && layout.ends_with(":checker: make check\n")
    );
    assert_eq!(
        p.read("_palette/backlog.rst"),
        "Backlog — Demo\n==============\n\nPhases\n------\n\nNone.\n\nItems\n-----\n\nNone.\n"
    );
    assert!(
        p.read("_palette/state.rst")
            .contains(":Updated: 2026-09-30\n")
    );
    assert_eq!(r.files.len(), 4);
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
    let err = init::init(&p.ctx, params(&p, json!({}))).expect_err("twice");
    assert_eq!(err.code, ErrCode::AlreadyInitialized);
}

#[test]
fn init_keeps_an_existing_backlog_and_rejects_bad_placements() {
    let p = Proj::empty();
    p.write("docs/backlog.rst", "keep me\n");
    init::init(
        &p.ctx,
        params(&p, json!({"placements": {"backlog": "docs/backlog.rst"}})),
    )
    .expect("init");
    assert_eq!(p.read("docs/backlog.rst"), "keep me\n");
    for bad in [
        json!({"rfc": "../x"}),
        json!({"nope": "internal"}),
        json!({"backlog": "docs/backlog"}),
        json!({"rfc": "_palette/rfc"}),
    ] {
        let q = Proj::empty();
        let err = init::init(&q.ctx, params(&q, json!({"placements": bad}))).expect_err("bad");
        assert_eq!(err.code, ErrCode::InvalidParams, "{err}");
        assert!(q.files().is_empty());
    }
}

#[test]
fn a_whole_phase_lifecycle_on_a_fresh_project() {
    let p = Proj::empty();
    init::init(&p.ctx, params(&p, json!({"name": "Fresh"}))).expect("init");
    let c = &p.ctx;
    backlog::backlog_add(
        c,
        params(
            &p,
            json!({"title": "Only item", "type": "feature", "source": "user"}),
        ),
    )
    .expect("add");
    assert!(
        p.read("_palette/backlog.rst")
            .contains("Items\n-----\n\nB-1 Only item\n"),
        "the None. under Items is replaced"
    );
    backlog::backlog_update(c, params(&p, json!({"item": "B-1", "status": "approved"})))
        .expect("approve");
    backlog::phase_open(c, params(&p, json!({"title": "One", "goal": "G.", "reason": "R.", "exit_criteria": ["E."], "items": ["B-1"]}))).expect("open");
    assert!(
        p.read("_palette/backlog.rst")
            .contains(":phase-1: `phase-1/phase.rst <phase-1/phase.rst>`_ — active")
    );
    backlog::deliverable_create(
        c,
        params(
            &p,
            json!({"item": "B-1", "title": "Only item", "what_and_why": "W.", "done_when": ["D."]}),
        ),
    )
    .expect("deliverable");
    state::state_record(
        c,
        params(
            &p,
            json!({"kind": "decision", "text": "Use it", "source": "Me", "target": "docs/x.rst"}),
        ),
    )
    .expect("decision");
    backlog::phase_close(c, params(&p, json!({"phase": 1, "items": [{"item": "B-1", "result": "done", "outcome": "the changelog"}]}))).expect("close");
    let backlog_text = p.read("_palette/backlog.rst");
    assert!(
        backlog_text.contains(":phase-1: `phase-1/phase.rst <phase-1/phase.rst>`_ — closed")
            && backlog_text.contains(":Status: done")
            && backlog_text.contains(":Outcome: the changelog")
    );
    let files: Vec<String> = p
        .files()
        .into_iter()
        .filter(|f| !f.ends_with(".palette.lock"))
        .collect();
    assert_eq!(
        files,
        vec![
            "_palette/.gitignore",
            "_palette/backlog.rst",
            "_palette/layout.rst",
            "_palette/phase-1/deliverables/deliverable-1-only-item.rst",
            "_palette/phase-1/phase.rst",
            "_palette/state.rst"
        ]
    );
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
}

// ── layout_set ──────────────────────────────────────────────────────────

fn move_family(p: &Proj, family: &str, placement: &str) -> ToolResult {
    init::layout_set(
        &p.ctx,
        params(
            p,
            json!({"family": family, "placement": placement, "confirmed_by_user": true}),
        ),
    )
}

#[test]
fn moving_families_rewrites_links_in_both_directions() {
    let p = Proj::valid();
    move_family(&p, "glossary", "internal").expect("glossary");
    assert!(
        p.read("_palette/glossary.rst")
            .contains("`Contract <../docs/spec/thing.rst#contract>`_")
    );
    assert!(
        p.read("_palette/layout.rst")
            .contains(":glossary: internal")
    );

    move_family(&p, "state", "docs/state.rst").expect("state");
    assert!(
        p.read("docs/state.rst")
            .contains("`RFC-0001 <rfc/rfc-0001-alpha.rst>`_"),
        "{}",
        p.read("docs/state.rst")
    );
    assert!(!p.exists("_palette/state.rst"));

    move_family(&p, "design", "internal").expect("design");
    assert!(
        p.read("_palette/design/overview.rst")
            .contains("`the thing spec <../../docs/spec/thing.rst>`_")
    );

    move_family(&p, "phase", "plan").expect("phase");
    assert!(p.exists("plan/phase-1/phase.rst") && !p.exists("_palette/phase-1/phase.rst"));
    assert!(
        p.read("_palette/backlog.rst")
            .contains("`phase-1/phase.rst <../plan/phase-1/phase.rst>`_")
    );

    move_family(&p, "deliverable", "plan").expect("deliverable");
    assert!(p.exists("plan/phase-1/deliverables/deliverable-1-first-thing.rst"));
    assert!(
        p.read("_palette/backlog.rst")
            .contains("<../plan/phase-1/deliverables/deliverable-1-first-thing.rst>")
    );

    move_family(&p, "staging", "internal").expect("staging");
    assert!(p.exists("_palette/staging/spec/thing.rst") && !p.exists("docs/staging"));
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
    // Everything still reads through the layout.
    let layout: Value =
        serde_json::from_str(&tools::layout(&p.ctx, params(&p, json!({}))).expect("layout"))
            .expect("json");
    assert_eq!(layout["families"]["state"]["placement"], "docs/state.rst");
}

#[test]
fn a_move_that_would_break_the_link_rules_is_refused() {
    let p = Proj::valid();
    let before = p.snapshot();
    // docs/design links to the spec; internal spec files may not be linked from a project path.
    let err = move_family(&p, "spec", "internal").expect_err("refused");
    assert_eq!(err.code, ErrCode::InvariantViolation);
    assert!(
        err.message.contains("P004") && err.message.contains("overview.rst"),
        "{}",
        err.message
    );
    assert_eq!(p.snapshot(), before);
}

#[test]
fn moving_a_family_rewrites_wrapped_links() {
    let p = Proj::valid();
    // Text on one line, target on the next: the target is rewritten in place.
    p.replace(
        "docs/glossary.rst",
        "Defined: `Contract <spec/thing.rst#contract>`_.",
        "Defined: `Contract\n  <spec/thing.rst#contract>`_.",
    );
    // A target that itself wraps is written back on one line.
    p.mutate("docs/glossary.rst", |s| {
        format!("{s}\nother\n  See `the contract <spec/\n  thing.rst#contract>`_ again.\n")
    });
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
    move_family(&p, "glossary", "internal").expect("glossary");
    let text = p.read("_palette/glossary.rst");
    assert!(
        text.contains("Defined: `Contract\n  <../docs/spec/thing.rst#contract>`_."),
        "{text}"
    );
    assert!(
        text.contains("  See `the contract <../docs/spec/thing.rst#contract>`_ again.\n"),
        "{text}"
    );
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
}

// ── backlog ─────────────────────────────────────────────────────────────

#[test]
fn backlog_update_edits_only_the_named_fields() {
    let p = Proj::valid();
    let before = p.read("_palette/backlog.rst");
    backlog::backlog_update(&p.ctx, params(&p, json!({
        "item": "B-2", "title": "A much longer title for the second thing", "depends": [], "body": "Line one of the new body. Line two of the new body.", "type": "chore", "outcome": "the log"
    }))).expect("update");
    let after = p.read("_palette/backlog.rst");
    let title = "B-2 A much longer title for the second thing";
    assert!(
        after.contains(&format!("{title}\n{}\n", "~".repeat(title.chars().count()))),
        "{after}"
    );
    assert!(
        after.contains(":Type: chore")
            && after.contains(":Depends: none")
            && after.contains(":Outcome: the log")
    );
    // Everything outside B-2 is byte-for-byte the same.
    let cut = |s: &str| {
        let a = s.find("B-2 ").expect("b2");
        let b = s.find("B-3 ").expect("b3");
        (s[..a].to_string(), s[b..].to_string())
    };
    assert_eq!(cut(&before), cut(&after));
    backlog::backlog_update(&p.ctx, params(&p, json!({"item": "B-2", "body": ""})))
        .expect("remove body");
    assert!(!p.read("_palette/backlog.rst").contains("Line one"));
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
}

#[test]
fn dropping_is_allowed_from_any_status() {
    for item in ["B-1", "B-2", "B-3", "B-4"] {
        let p = Proj::valid();
        backlog::backlog_update(
            &p.ctx,
            params(&p, json!({"item": item, "status": "dropped"})),
        )
        .unwrap_or_else(|e| panic!("{item}: {e}"));
    }
}

// ── state ───────────────────────────────────────────────────────────────

#[test]
fn state_entries_have_the_template_shape() {
    let p = Proj::valid();
    let c = &p.ctx;
    state::state_record(c, params(&p, json!({"kind": "decision", "text": "Ship the small version.", "source": "Sample Owner", "target": "docs/rfc/rfc-0003-gamma.rst"}))).expect("d");
    state::state_record(c, params(&p, json!({"kind": "question", "text": "What about the third operation?", "affects": "B-2; RFC-0003", "proposal": "Defer it.", "proposal_by": "Sample Author"}))).expect("q");
    state::state_record(c, params(&p, json!({"kind": "discrepancy", "text": "Two names for one thing", "evidence": "runtime"}))).expect("x");
    let s = p.read("_palette/state.rst");
    assert!(s.contains("- D-3 Ship the small version. Source: Sample Owner, 2026-09-30. Target:\n  docs/rfc/rfc-0003-gamma.rst.\n") || s.contains("- D-3 Ship the small version. Source: Sample Owner, 2026-09-30. Target: docs/rfc/rfc-0003-gamma.rst.\n"), "{s}");
    assert!(s.replace("\n  ", " ").contains("- Q-2 What about the third operation? Affects: B-2; RFC-0003. Proposal: Defer it (Sample Author, 2026-09-30)."), "{s}");
    assert!(
        s.contains("- X-2 Two names for one thing. Evidence: runtime, 2026-09-30."),
        "{s}"
    );
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
    assert!(
        p.lint().iter().all(|f| f.rule != "P011"),
        "entries stay within three lines"
    );
}

#[test]
fn resolving_the_last_entry_restores_none() {
    let p = Proj::valid();
    state::state_resolve(
        &p.ctx,
        params(&p, json!({"id": "X-1", "resolution": "fixed"})),
    )
    .expect("fixed");
    let s = p.read("_palette/state.rst");
    assert!(
        s.ends_with("Discrepancies\n-------------\n\nNone.\n"),
        "{s}"
    );
    state::state_resolve(
        &p.ctx,
        params(&p, json!({"id": "Q-1", "resolution": "withdrawn"})),
    )
    .expect("withdrawn");
    assert!(
        p.read("_palette/state.rst")
            .contains("Open questions\n--------------\n\nNone.\n")
    );
    // A new entry replaces the None.
    state::state_record(
        &p.ctx,
        params(
            &p,
            json!({"kind": "discrepancy", "text": "Again", "evidence": "static"}),
        ),
    )
    .expect("again");
    assert!(
        p.read("_palette/state.rst")
            .ends_with("- X-1 Again. Evidence: static, 2026-09-30.\n")
    );
    assert!(p.errors().is_empty());
}

#[test]
fn graduating_leaves_a_one_line_pointer() {
    let p = Proj::valid();
    state::state_resolve(
        &p.ctx,
        params(
            &p,
            json!({"id": "D-1", "resolution": "graduated", "written": "docs/design/overview.rst"}),
        ),
    )
    .expect("graduate");
    let s = p.read("_palette/state.rst");
    assert!(
        s.contains(
            "- D-1 Graduated to `docs/design/overview.rst <../docs/design/overview.rst>`_.\n"
        ),
        "{s}"
    );
    assert!(!s.contains("Target:\n  docs/design/overview.rst."));
}

// ── records ─────────────────────────────────────────────────────────────

#[test]
fn record_create_writes_the_template_with_initial_values() {
    let p = Proj::valid();
    let r = records::record_create(&p.ctx, params(&p, json!({
        "kind": "rfc", "title": "Delta rules", "authors": "Sample Author", "areas": "thing; rules", "description": "Adds delta rules.",
        "related": [{"record": "RFC-0001", "note": "the alpha names"}]
    }))).expect("create");
    assert_eq!(r.allocated, vec!["RFC-0004"]);
    let text = p.read("docs/rfc/rfc-0004-delta-rules.rst");
    let expected_head = "RFC-0004: Delta rules\n=====================\n\n:Status: Draft\n:Implementation: not-started — the change this record describes\n:Verification: none — 2026-09-30; not verified yet\n:Areas: thing; rules\n:Authors: Sample Author\n:Reviewers: none yet\n:Implementers: none yet\n:Accepted: none\n:Date: 2026-09-30\n:Revised: none\n:Depends: none\n:Supersedes: none\n:Related: RFC-0001 (the alpha names)\n:Changes: none\n:Description: Adds delta rules.\n\nSummary\n-------\n\nNone.\n";
    assert!(text.starts_with(expected_head), "{text}");
    assert!(text.ends_with("References\n----------\n\nNone.\n"));
    assert!(p.errors().is_empty());
    // Numbers keep counting per kind, and an unreadable file still holds its number.
    let mut b = std::fs::read(p.path("docs/rfc/rfc-0004-delta-rules.rst")).expect("read");
    b.push(0xff);
    std::fs::write(p.path("docs/rfc/rfc-0004-delta-rules.rst"), b).expect("corrupt");
    let r = records::record_create(&p.ctx, params(&p, json!({"kind": "rfc", "title": "Next", "authors": "A", "areas": "x", "description": "D"}))).expect("create");
    assert_eq!(
        r.allocated,
        vec!["RFC-0005"],
        "the number of an unreadable record is not reused"
    );
}

#[test]
fn record_changes_accept_single_file_families_and_project_documents() {
    let p = Proj::valid();
    p.write("AGENTS.md", "# Agents\n\nBuild with make.\n");
    let create = |changes: Value| {
        records::record_create(
            &p.ctx,
            params(
                &p,
                json!({
                    "kind": "rfc", "title": "Delta rules", "authors": "A", "areas": "thing", "description": "D.",
                    "changes": changes
                }),
            ),
        )
    };
    for (changes, needle) in [
        (
            json!([{"document": "AGENTS.md", "sections": ["a", "b"]}]),
            "exactly one element",
        ),
        (
            json!([{"document": "AGENTS.md", "sections": []}]),
            "exactly one element",
        ),
        (
            json!([{"document": "AGENTS.md", "sections": ["created"]}]),
            "`created` is only for a new design or spec document",
        ),
        (
            json!([{"document": "principles.rst", "sections": ["created"]}]),
            "`created` is only for a new design or spec document",
        ),
        (
            json!([{"document": "src/main.rs", "sections": ["main"]}]),
            "Changes lists documents, not source files",
        ),
    ] {
        let err = create(changes.clone()).expect_err("invalid");
        assert_eq!(err.code, ErrCode::InvalidParams, "{changes}");
        assert!(err.message.contains(needle), "{changes}: {}", err.message);
    }
    let err = create(json!([{"document": "BUILDING.md", "sections": ["the steps"]}]))
        .expect_err("missing");
    assert_eq!(err.code, ErrCode::InvariantViolation);
    assert!(
        err.message.contains("`BUILDING.md` does not exist"),
        "{}",
        err.message
    );

    create(json!([
        {"document": "principles.rst", "sections": ["Keep the thing small"]},
        {"document": "AGENTS.md", "sections": ["build; flags"]}
    ]))
    .expect("create");
    let text = p.read("docs/rfc/rfc-0004-delta-rules.rst");
    assert!(
        text.contains(
            ":Changes: principles.rst (Keep the thing small); AGENTS.md (build; flags)\n"
        ),
        "{text}"
    );
    records::record_update(&p.ctx, params(&p, json!({
        "record": "RFC-0004", "changes": [{"document": "glossary.rst", "sections": ["Terms"]}]
    }))).expect("update");
    assert!(
        p.read("docs/rfc/rfc-0004-delta-rules.rst")
            .contains(":Changes: glossary.rst (Terms)\n")
    );
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
}

#[test]
fn record_update_clarifications_accumulate_revised_entries() {
    let p = Proj::valid();
    let c = &p.ctx;
    records::record_update(c, params(&p, json!({"record": "RFC-0002", "title": "Beta rules, clarified", "clarification": true, "revision_note": "Fixed the title; nothing else"}))).expect("first");
    records::record_update(c, params(&p, json!({"record": "RFC-0002", "sections": {"Summary": "New summary."}, "clarification": true, "revision_note": "Corrected the summary"}))).expect("second");
    let t = p.read("docs/rfc/rfc-0002-beta.rst");
    assert!(
        t.starts_with("RFC-0002: Beta rules, clarified\n===============================\n"),
        "{t}"
    );
    assert!(t.replace("\n  ", " ").contains(":Revised: 2026-09-30 — Fixed the title, nothing else; 2026-09-30 — Corrected the summary\n"), "{t}");
    assert!(t.contains("Summary\n-------\n\nNew summary.\n\nProblem and context"));
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
}

#[test]
fn record_update_supersession_flips_the_older_record() {
    let p = Proj::valid();
    records::record_update(&p.ctx, params(&p, json!({
        "record": "RFC-0003", "supersedes": [{"record": "RFC-0001", "note": "the alpha contract"}]
    }))).expect("supersede");
    assert!(
        p.read("docs/rfc/rfc-0001-alpha.rst")
            .contains(":Status: Superseded")
    );
    let err = records::record_update(
        &p.ctx,
        params(&p, json!({"record": "RFC-0003", "supersedes": []})),
    )
    .expect_err("undo");
    assert_eq!(err.code, ErrCode::InvariantViolation);
}

// ── read tools ──────────────────────────────────────────────────────────

#[test]
fn template_returns_the_embedded_text() {
    for (name, needle) in [
        ("rfc", "RFC-<NNNN>: <Title>"),
        ("adr", "ADR-<NNNN>"),
        ("backlog", "B-<n>"),
        ("layout", ":checker:"),
        ("house-style", "RST house style"),
    ] {
        let t = tools::template(serde_json::from_value(json!({"family": name})).expect("p"))
            .expect(name);
        assert!(t.contains(needle), "{name}");
    }
    let err = tools::template(serde_json::from_value(json!({"family": "staging"})).expect("p"))
        .expect_err("staging");
    assert_eq!(err.code, ErrCode::InvalidParams);
    assert!(
        tools::template(serde_json::from_value(json!({"family": "nope"})).expect("p")).is_err()
    );
}

#[test]
fn lint_tool_filters_by_path_but_runs_every_rule() {
    let p = Proj::valid();
    p.replace(
        "docs/rfc/rfc-0003-gamma.rst",
        ":Status: Draft",
        ":Status: Done",
    );
    p.replace(
        "docs/glossary.rst",
        "Terms",
        "Terms\n-----\n\n.. note:: x\n\nMore",
    );
    let all: Value =
        serde_json::from_str(&tools::lint(&p.ctx, params(&p, json!({}))).expect("lint"))
            .expect("json");
    assert!(all["errors"].as_u64().expect("n") >= 2);
    let only: Value = serde_json::from_str(
        &tools::lint(&p.ctx, params(&p, json!({"paths": ["docs/rfc"]}))).expect("lint"),
    )
    .expect("json");
    let files: Vec<&str> = only["findings"]
        .as_array()
        .expect("arr")
        .iter()
        .map(|f| f["file"].as_str().expect("s"))
        .collect();
    assert!(
        !files.is_empty() && files.iter().all(|f| f.starts_with("docs/rfc/")),
        "{files:?}"
    );
    let err = tools::lint(&p.ctx, params(&p, json!({"paths": ["../x"]}))).expect_err("dotdot");
    assert_eq!(err.code, ErrCode::InvalidParams);
}

#[test]
fn status_stays_under_the_limit_and_says_what_it_omitted() {
    let p = Proj::valid();
    let small = tools::status(&p.ctx, params(&p, json!({}))).expect("status");
    assert!(
        small.contains("Phases: phase-1 active")
            && small.contains("Items (4):")
            && small.contains("Open questions (1):")
    );
    assert!(small.contains("Lint: 0 error(s), 0 warning(s)"), "{small}");
    assert!(!small.contains("Omitted"));
    // Many questions overflow the budget.
    let mut extra = String::new();
    for n in 2..200 {
        extra.push_str(&format!("- Q-{n} Question number {n} that has a fairly long text so that the list grows past the limit. Affects: B-2. Proposal: none.\n"));
    }
    p.mutate("_palette/state.rst", |s| {
        s.replace("\nDiscrepancies\n", &format!("{extra}\nDiscrepancies\n"))
    });
    let big = tools::status(&p.ctx, params(&p, json!({}))).expect("status");
    assert!(big.chars().count() < 4_000, "{}", big.chars().count());
    assert!(
        big.contains("Omitted to stay under 4000 characters:"),
        "{big}"
    );
    assert!(big.contains("Open questions (199):"));
    let rec =
        tools::status(&p.ctx, params(&p, json!({"record": "RFC-0009"}))).expect_err("unknown");
    assert_eq!(rec.code, ErrCode::NotFound);
}

#[test]
fn any_write_refreshes_generated_files_that_drifted() {
    let p = Proj::valid();
    p.remove("docs/rfc/index.rst");
    p.mutate("docs/changeset/index.rst", |s| {
        s.replace("Edits: 2", "Edits: 9")
    });
    p.mutate("docs/staging/spec/thing.rst", |s| {
        s.replace("two operations", "three operations")
    });
    p.write("docs/staging/spec/orphan.rst", "Orphan\n======\n");
    assert!(p.errors().len() >= 4);
    // A write that concerns none of these files still repairs them: generated files are
    // written only by the server.
    let r = backlog::backlog_add(
        &p.ctx,
        params(
            &p,
            json!({"title": "Unrelated", "type": "chore", "source": "user"}),
        ),
    )
    .expect("add");
    let files: Vec<&str> = r.files.iter().map(|(f, _)| f.as_str()).collect();
    for f in [
        "_palette/backlog.rst",
        "docs/rfc/index.rst",
        "docs/changeset/index.rst",
        "docs/staging/spec/thing.rst",
        "docs/staging/spec/orphan.rst",
    ] {
        assert!(files.contains(&f), "{f} in {files:?}");
    }
    assert!(!p.exists("docs/staging/spec/orphan.rst"));
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
}

#[test]
fn multi_id_pointers_are_recognised_counted_and_removed_at_close() {
    let p = Proj::valid();
    // The leader's own convention: one pointer for several decisions, with ranges.
    p.replace(
        "_palette/state.rst",
        "- D-2 Graduated to `RFC-0001 <../docs/rfc/rfc-0001-alpha.rst>`_.",
        "- D-2, D-5 to D-9 Graduated to `RFC-0001 <../docs/rfc/rfc-0001-alpha.rst>`_.",
    );
    let status = tools::status(&p.ctx, params(&p, json!({}))).expect("status");
    assert!(
        status.contains("Decisions not yet graduated (1):"),
        "{status}"
    );
    // Numbers named inside the pointer are taken: the next decision is D-10.
    let r = state::state_record(
        &p.ctx,
        params(
            &p,
            json!({"kind": "decision", "text": "Next", "source": "S", "target": "docs/x.rst"}),
        ),
    )
    .expect("record");
    assert_eq!(r.allocated, vec!["D-10"]);
    let err = state::state_resolve(
        &p.ctx,
        params(
            &p,
            json!({"id": "D-5", "resolution": "graduated", "written": "RFC-0001"}),
        ),
    )
    .expect_err("inside a pointer");
    assert_eq!(err.code, ErrCode::InvariantViolation);
    assert!(err.message.contains("already graduated"), "{}", err.message);
    // Closing the phase removes every pointer.
    backlog::phase_close(&p.ctx, params(&p, json!({"phase": 1, "items": [{"item": "B-1", "result": "done", "outcome": "RFC-0002"}]}))).expect("close");
    let s = p.read("_palette/state.rst");
    assert!(
        !s.contains("Graduated to") && s.contains("D-1 Use the sample layout"),
        "{s}"
    );
}

// ── RFC-0009: partial supersession, several active phases ───────────────

#[test]
fn a_partial_supersession_leaves_the_older_record_and_shows_the_part() {
    let p = Proj::valid();
    records::record_update(&p.ctx, params(&p, json!({
        "record": "RFC-0003",
        "supersedes": [{"record": "RFC-0001", "note": "the open operation", "partial": true}]
    }))).expect("partial supersede");
    assert!(
        p.read("docs/rfc/rfc-0003-gamma.rst")
            .contains(":Supersedes: RFC-0001 (in part: the open operation)")
    );
    assert!(
        p.read("docs/rfc/rfc-0001-alpha.rst")
            .contains(":Status: Accepted")
    );
    assert!(
        p.read("docs/rfc/index.rst")
            .contains("Superseded by: RFC-0003 (in part: the open operation).")
    );
    let status = tools::status(&p.ctx, params(&p, json!({"record": "RFC-0001"}))).expect("status");
    assert!(
        status.contains("Superseded by: RFC-0003 (in part: the open operation)"),
        "{status}"
    );
    // Widening the entry to a whole supersession flips the older record; narrowing
    // a whole one back to a part is refused.
    records::record_update(
        &p.ctx,
        params(
            &p,
            json!({
                "record": "RFC-0003",
                "supersedes": [{"record": "RFC-0001", "note": "the alpha contract"}]
            }),
        ),
    )
    .expect("whole supersede");
    assert!(
        p.read("docs/rfc/rfc-0001-alpha.rst")
            .contains(":Status: Superseded")
    );
    let err = records::record_update(&p.ctx, params(&p, json!({
        "record": "RFC-0003",
        "supersedes": [{"record": "RFC-0001", "note": "the open operation", "partial": true}]
    }))).expect_err("narrow");
    assert_eq!(err.code, ErrCode::InvariantViolation);
    // The marker inside the note is the same request.
    let err = records::record_update(
        &p.ctx,
        params(
            &p,
            json!({
                "record": "RFC-0003",
                "supersedes": [{"record": "RFC-0001", "note": "in part: the open operation"}]
            }),
        ),
    )
    .expect_err("narrow through the note");
    assert_eq!(err.code, ErrCode::InvariantViolation);
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
}

#[test]
fn an_rfc_supersedes_an_adr_in_part_and_the_adr_index_shows_it() {
    let p = Proj::valid();
    records::record_update(
        &p.ctx,
        params(
            &p,
            json!({
                "record": "RFC-0003",
                "supersedes": [{"record": "ADR-0001", "note": "the close name", "partial": true}]
            }),
        ),
    )
    .expect("partial supersede of an ADR");
    assert!(
        p.read("docs/rfc/rfc-0003-gamma.rst")
            .contains(":Supersedes: ADR-0001 (in part: the close name)")
    );
    assert!(
        p.read("docs/adr/adr-0001-naming.rst")
            .contains(":Status: Accepted")
    );
    assert!(
        p.read("docs/adr/index.rst")
            .contains("Superseded by: RFC-0003 (in part: the close name).")
    );
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
}

#[test]
fn record_create_with_a_partial_supersession() {
    let p = Proj::valid();
    records::record_create(&p.ctx, params(&p, json!({
        "kind": "rfc", "title": "Delta", "authors": "A", "description": "D.", "areas": "thing",
        "supersedes": [{"record": "RFC-0002", "note": "the beta names", "partial": true}]
    }))).expect("create");
    assert!(
        p.read("docs/rfc/rfc-0004-delta.rst")
            .contains(":Supersedes: RFC-0002 (in part: the beta names)")
    );
    assert!(
        p.read("docs/rfc/rfc-0002-beta.rst")
            .contains(":Status: Accepted")
    );
    // A note that carries the marker is partial too, and the marker is written once.
    let before = p.read("docs/rfc/rfc-0001-alpha.rst");
    records::record_create(&p.ctx, params(&p, json!({
        "kind": "rfc", "title": "Epsilon", "authors": "A", "description": "D.", "areas": "thing",
        "supersedes": [{"record": "RFC-0001", "note": "In part: the open operation"}]
    }))).expect("create");
    assert!(
        p.read("docs/rfc/rfc-0005-epsilon.rst")
            .contains(":Supersedes: RFC-0001 (in part: the open operation)")
    );
    assert_eq!(
        p.read("docs/rfc/rfc-0001-alpha.rst"),
        before,
        "the older record is untouched"
    );
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
}

#[test]
fn a_wrapped_body_link_is_listed_as_linked_from() {
    let p = Proj::valid();
    p.replace(
        "docs/rfc/rfc-0003-gamma.rst",
        "for the naming choice.",
        "for the naming choice and\n`the alpha\ncontract <rfc-0001-alpha.rst>`_ for the base.",
    );
    // Any write regenerates the index.
    state::state_record(
        &p.ctx,
        params(
            &p,
            json!({"kind": "decision", "text": "x", "source": "me", "target": "y"}),
        ),
    )
    .expect("write");
    let index = p.read("docs/rfc/index.rst");
    let alpha = index
        .split("\n\n")
        .find(|b| b.starts_with("`RFC-0001 "))
        .expect("RFC-0001 entry");
    assert!(
        alpha.contains("Linked from: RFC-0002 (Depends), RFC-0003 (link), ADR-0001 (Depends), ADR-0001 (Within)."),
        "{alpha}"
    );
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
}

#[test]
fn amended_by_is_computed_for_a_legacy_amends_field() {
    let p = Proj::valid();
    p.replace(
        "docs/contributing.rst",
        ":Amends: none",
        ":Amends: until 2026-12-31",
    );
    p.replace(
        "docs/rfc/rfc-0003-gamma.rst",
        ":Related: none\n",
        ":Related: none\n:Amends: RFC-0001 (the open operation)\n",
    );
    // Any write regenerates the index.
    state::state_record(
        &p.ctx,
        params(
            &p,
            json!({"kind": "decision", "text": "x", "source": "me", "target": "y"}),
        ),
    )
    .expect("write");
    assert!(
        p.read("docs/rfc/index.rst")
            .contains("Amended by: RFC-0003 (the open operation).")
    );
    let status = tools::status(&p.ctx, params(&p, json!({"record": "RFC-0001"}))).expect("status");
    assert!(
        status.contains("Amended by: RFC-0003 (the open operation)"),
        "{status}"
    );
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
}

#[test]
fn two_phases_can_be_active_and_an_item_belongs_to_one() {
    let p = Proj::valid();
    let c = &p.ctx;
    // phase-1 is active; B-2 is approved.
    backlog::phase_open(c, params(&p, json!({"title": "Two", "goal": "G.", "reason": "R.", "exit_criteria": ["E."], "items": ["B-2"]}))).expect("second phase");
    let text = p.read("_palette/backlog.rst");
    assert!(
        text.contains(":phase-1: `phase-1/phase.rst <phase-1/phase.rst>`_ — active")
            && text.contains(":phase-2: `phase-2/phase.rst <phase-2/phase.rst>`_ — active")
            && text.contains(":Status: in-phase-2"),
        "{text}"
    );
    backlog::deliverable_create(
        c,
        params(
            &p,
            json!({"item": "B-2", "title": "Second thing", "what_and_why": "W.", "done_when": ["D."]}),
        ),
    )
    .expect("deliverable in the second phase");
    assert!(p.exists("_palette/phase-2/deliverables/deliverable-2-second-thing.rst"));
    let err = backlog::backlog_update(
        c,
        params(&p, json!({"item": "B-1", "status": "in-phase-2"})),
    )
    .expect_err("an item is in one phase");
    assert_eq!(err.code, ErrCode::InvariantViolation);
    let status = tools::status(c, params(&p, json!({}))).expect("status");
    assert!(
        status.contains("phase-1 active; phase-2 active"),
        "{status}"
    );
    assert!(p.errors().is_empty(), "{}", show(&p.errors()));
}

#[test]
fn accepting_writes_who_and_the_utc_time() {
    let p = Proj::valid();
    records::record_update(
        &p.ctx,
        params(
            &p,
            json!({
                "record": "RFC-0003", "status": "Proposed"
            }),
        ),
    )
    .expect("propose");
    records::record_update(
        &p.ctx,
        params(
            &p,
            json!({
                "record": "RFC-0003", "status": "Accepted", "accepted_by": "Sample Owner"
            }),
        ),
    )
    .expect("accept");
    assert!(
        p.read("docs/rfc/rfc-0003-gamma.rst")
            .contains(":Accepted: Sample Owner (2026-09-30T12:00Z)")
    );
    assert!(p.lint().is_empty(), "{}", show(&p.lint()));
}
