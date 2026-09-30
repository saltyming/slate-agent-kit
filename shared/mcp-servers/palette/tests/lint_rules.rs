//! One failing fixture per lint rule P001 to P016, made by mutating the valid fixture,
//! plus positive variants for the rules that have an exception.

mod common;

use common::*;
use palette_server::lint::{self, Severity};

fn failing(rule: &str, file: &str, needle: &str, mutate: impl FnOnce(&Proj)) {
    let p = Proj::valid();
    mutate(&p);
    let f = p.lint();
    assert!(
        has(&f, rule, file, needle),
        "expected {rule} in {file} containing {needle:?}; got:\n{}",
        show(&f)
    );
}

fn passing(rule: &str, mutate: impl FnOnce(&Proj)) {
    let p = Proj::valid();
    mutate(&p);
    let f = p.lint();
    assert!(
        f.iter().all(|x| x.rule != rule),
        "unexpected {rule}:\n{}",
        show(&f)
    );
}

#[test]
fn valid_fixture_has_no_findings() {
    let f = Proj::valid().lint();
    assert!(f.is_empty(), "{}", show(&f));
}

// ── P001 ────────────────────────────────────────────────────────────────

#[test]
fn p001_tables() {
    for table in [
        "+---+---+\n| a | b |\n+---+---+\n",
        "=====  =====\nA      B\n=====  =====\n",
        "| a | b |\n|---|---|\n| 1 | 2 |\n",
    ] {
        failing("P001", "principles.rst", "tables are not allowed", |p| {
            p.mutate("docs/principles.rst", |s| format!("{s}\n{table}"));
        });
    }
}

#[test]
fn p001_directive_substitution_footnote_citation() {
    failing("P001", "glossary.rst", "directive `note::`", |p| {
        p.mutate("docs/glossary.rst", |s| format!("{s}\n.. note:: careful\n"));
    });
    failing("P001", "glossary.rst", "substitution definitions", |p| {
        p.mutate("docs/glossary.rst", |s| {
            format!("{s}\n.. |x| replace:: y\n")
        });
    });
    failing("P001", "glossary.rst", "substitution references", |p| {
        p.mutate("docs/glossary.rst", |s| format!("{s}\nUse |name| here.\n"));
    });
    failing(
        "P001",
        "glossary.rst",
        "footnotes and citations are not allowed",
        |p| {
            p.mutate("docs/glossary.rst", |s| {
                format!("{s}\n.. [1] A footnote.\n")
            });
        },
    );
    failing(
        "P001",
        "glossary.rst",
        "footnote and citation references",
        |p| {
            p.mutate("docs/glossary.rst", |s| {
                format!("{s}\nSee [1]_ and [CIT2002]_.\n")
            });
        },
    );
}

#[test]
fn p001_comments_and_literal_blocks_are_allowed() {
    passing("P001", |p| {
        p.mutate("docs/glossary.rst", |s| format!("{s}\n.. a plain comment\n\nExample::\n\n  .. note:: inside a literal block\n  +---+\n"));
    });
}

#[test]
fn p001_headings() {
    failing("P001", "principles.rst", "overline", |p| {
        p.replace(
            "docs/principles.rst",
            "Principles — Sample\n===================",
            "===================\nPrinciples — Sample\n===================",
        );
    });
    failing("P001", "rfc-0003", "shorter than its title", |p| {
        p.replace(
            "docs/rfc/rfc-0003-gamma.rst",
            "Summary\n-------",
            "Summary\n---",
        )
    });
    failing("P001", "rfc-0003", "house-style order", |p| {
        p.replace(
            "docs/rfc/rfc-0003-gamma.rst",
            "Summary\n-------",
            "Summary\n*******",
        )
    });
    failing("P001", "backlog.rst", "skips a heading level", |p| {
        p.replace(
            "_palette/backlog.rst",
            "B-1 First thing\n~~~~~~~~~~~~~~~",
            "B-1 First thing\n^^^^^^^^^^^^^^^",
        )
    });
}

#[test]
fn p001_invalid_utf8() {
    let p = Proj::valid();
    let mut b = std::fs::read(p.path("docs/principles.rst")).expect("read");
    b.push(0xff);
    std::fs::write(p.path("docs/principles.rst"), b).expect("write");
    assert!(has(&p.lint(), "P001", "principles.rst", "not valid UTF-8"));
}

// ── P002 ────────────────────────────────────────────────────────────────

#[test]
fn p002_sections() {
    failing(
        "P002",
        "rfc-0003",
        "missing required section `Alternatives and costs`",
        |p| {
            p.replace("docs/rfc/rfc-0003-gamma.rst", "Alternatives and costs\n----------------------\n\nThe alternatives and costs of this record.\n\n", "");
        },
    );
    failing("P002", "rfc-0003", "extra section `Extra`", |p| {
        p.mutate("docs/rfc/rfc-0003-gamma.rst", |s| {
            format!("{s}\nExtra\n-----\n\ntext\n")
        })
    });
    failing("P002", "rfc-0003", "out of order", |p| {
        p.replace(
            "docs/rfc/rfc-0003-gamma.rst",
            "Evidence\n--------",
            "Design\n------",
        );
        p.replace(
            "docs/rfc/rfc-0003-gamma.rst",
            "\nDesign\n------\n\nThe design of this record.",
            "\nEvidence\n--------\n\nThe design of this record.",
        );
    });
}

#[test]
fn p002_fields() {
    failing(
        "P002",
        "rfc-0003",
        "missing required field `:Areas:`",
        |p| p.replace("docs/rfc/rfc-0003-gamma.rst", ":Areas: thing\n", ""),
    );
    failing("P002", "rfc-0003", "out of order", |p| {
        p.replace(
            "docs/rfc/rfc-0003-gamma.rst",
            ":Areas: thing\n:Authors: Sample Author\n",
            ":Authors: Sample Author\n:Areas: thing\n",
        );
    });
    failing("P002", "rfc-0003", "outside the allowed values", |p| {
        p.replace(
            "docs/rfc/rfc-0003-gamma.rst",
            ":Status: Draft",
            ":Status: Done",
        )
    });
    failing("P002", "rfc-0003", "does not match", |p| {
        p.replace(
            "docs/rfc/rfc-0003-gamma.rst",
            "RFC-0003: Gamma proposal",
            "RFC three",
        )
    });
    failing("P002", "backlog.rst", "outside the allowed values", |p| {
        p.replace("_palette/backlog.rst", ":Type: feature", ":Type: epic")
    });
    failing("P002", "backlog.rst", "outside the allowed values", |p| {
        p.replace("_palette/backlog.rst", "— active", "— open")
    });
    failing("P012", "layout.rst", "setting `checker` is missing", |p| {
        p.replace("_palette/layout.rst", ":checker: none\n", "")
    });
    failing(
        "P002",
        "state.rst",
        "missing required field `:Updated:`",
        |p| p.replace("_palette/state.rst", ":Updated: 2026-09-01\n", ""),
    );
    failing(
        "P002",
        "phase.rst",
        "missing required section `Why now`",
        |p| {
            p.replace(
                "_palette/phase-1/phase.rst",
                "Why now\n-------\n\nEverything else builds on it.\n\n",
                "",
            )
        },
    );
}

#[test]
fn p002_accepts_subsections_under_required_sections() {
    passing("P002", |p| {
        p.replace(
            "docs/rfc/rfc-0003-gamma.rst",
            "Design\n------\n\nThe design of this record.",
            "Design\n------\n\nThe design of this record.\n\nDetail\n~~~~~~\n\nMore.",
        )
    });
}

// ── P003 ────────────────────────────────────────────────────────────────

#[test]
fn p003_identity() {
    failing(
        "P003",
        "rfc-0009",
        "file name says RFC-0009 but the title says RFC-0003",
        |p| {
            std::fs::rename(
                p.path("docs/rfc/rfc-0003-gamma.rst"),
                p.path("docs/rfc/rfc-0009-gamma.rst"),
            )
            .expect("rename");
        },
    );
    failing("P003", "Bad_Name", "kebab-case", |p| {
        let s = p.read("docs/design/overview.rst");
        p.write("docs/design/Bad_Name.rst", &s);
    });
    failing("P003", "rfc-0003-gamma", "duplicate number RFC-0003", |p| {
        let s = p.read("docs/rfc/rfc-0003-gamma.rst");
        p.write("docs/rfc/rfc-0003-gamma-copy.rst", &s);
    });
    failing(
        "P003",
        "deliverable-7",
        "deliverable 7 but the title says Deliverable 1",
        |p| {
            std::fs::rename(
                p.path("_palette/phase-1/deliverables/deliverable-1-first-thing.rst"),
                p.path("_palette/phase-1/deliverables/deliverable-7-first-thing.rst"),
            )
            .expect("rename");
        },
    );
    failing("P003", "phase.rst", "folder is phase-2", |p| {
        std::fs::rename(p.path("_palette/phase-1"), p.path("_palette/phase-2")).expect("rename");
    });
    failing("P003", "rfc-0002.rst", "file name says", |p| {
        p.replace(
            "docs/changeset/rfc-0002.rst",
            "Changeset: RFC-0002",
            "Changeset: RFC-0003",
        )
    });
    failing(
        "P003",
        "rfc-1-alpha",
        "must be `rfc-NNNN-<slug>.rst`",
        |p| {
            std::fs::rename(
                p.path("docs/rfc/rfc-0001-alpha.rst"),
                p.path("docs/rfc/rfc-1-alpha.rst"),
            )
            .expect("rename");
            let _ = ();
        },
    );
}

// ── P004 ────────────────────────────────────────────────────────────────

#[test]
fn p004_links() {
    failing("P004", "overview.rst", "does not exist", |p| {
        p.replace(
            "docs/design/overview.rst",
            "../spec/thing.rst",
            "../spec/missing.rst",
        )
    });
    failing("P004", "glossary.rst", "anchor `#nope`", |p| {
        p.replace("docs/glossary.rst", "#contract", "#nope")
    });
    failing(
        "P004",
        "principles.rst",
        "must not link into `_palette/`",
        |p| {
            p.mutate("docs/principles.rst", |s| {
                format!("{s}\nSee `the backlog <../_palette/backlog.rst>`_.\n")
            });
        },
    );
    failing("P004", "rfc-0003", "RFC-0009, which does not exist", |p| {
        p.replace(
            "docs/rfc/rfc-0003-gamma.rst",
            ":Depends: RFC-0002 (the beta rules it builds on)",
            ":Depends: RFC-0009 (a missing record)",
        )
    });
}

#[test]
fn p004_internal_documents_may_link_to_project_paths() {
    passing("P004", |p| {
        p.mutate("_palette/state.rst", |s| {
            s.replace(
                "Target:\n  docs/design/overview.rst.",
                "Target:\n  `docs/design/overview.rst <../docs/design/overview.rst>`_.",
            )
        })
    });
}

// ── P005 ────────────────────────────────────────────────────────────────

#[test]
fn p005_forward_links_and_cycles() {
    let p = Proj::valid();
    p.replace(
        "docs/rfc/rfc-0001-alpha.rst",
        ":Depends: none",
        ":Depends: RFC-0002 (a newer record)",
    );
    let f = p.lint();
    assert!(has(&f, "P005", "rfc-0001", "created later"), "{}", show(&f));
    assert!(
        has(
            &f,
            "P005",
            "rfc-0001",
            "relation cycle among RFC-0001, RFC-0002"
        ),
        "{}",
        show(&f)
    );
}

#[test]
fn p005_cross_kind_dates() {
    failing("P005", "adr-0001", "later than", |p| {
        p.replace(
            "docs/adr/adr-0001-naming.rst",
            ":Date: 2026-02-01",
            ":Date: 2025-12-01",
        );
    });
}

#[test]
fn p005_redundant_depends_and_related() {
    let redundant =
        ":Depends: RFC-0002 (the beta rules it builds on); RFC-0001 (the alpha contract)";
    let f = {
        let p = Proj::valid();
        p.replace(
            "docs/rfc/rfc-0003-gamma.rst",
            ":Depends: RFC-0002 (the beta rules it builds on)",
            redundant,
        );
        p.lint()
    };
    assert!(
        f.iter().any(|x| x.rule == "P005"
            && x.severity == Severity::Warning
            && x.message.contains("already reachable")),
        "{}",
        show(&f)
    );
    passing("P005", |p| {
        p.replace("docs/rfc/rfc-0003-gamma.rst", ":Depends: RFC-0002 (the beta rules it builds on)", ":Depends: RFC-0002 (the beta rules it builds on); RFC-0001 (directly uses the alpha names)");
    });
    let f = {
        let p = Proj::valid();
        p.replace(
            "docs/rfc/rfc-0003-gamma.rst",
            ":Related: none",
            ":Related: RFC-0002 (context)",
        );
        p.lint()
    };
    assert!(
        f.iter().any(|x| x.rule == "P005"
            && x.severity == Severity::Warning
            && x.message.contains("both Depends and Related")),
        "{}",
        show(&f)
    );
}

#[test]
fn p005_supersedes_requires_superseded_status() {
    failing(
        "P005",
        "rfc-0003",
        "status is Accepted; its status must be Superseded",
        |p| {
            p.replace(
                "docs/rfc/rfc-0003-gamma.rst",
                ":Supersedes: none",
                ":Supersedes: RFC-0001 (the alpha contract)",
            );
        },
    );
    passing("P005", |p| {
        p.replace(
            "docs/rfc/rfc-0003-gamma.rst",
            ":Supersedes: none",
            ":Supersedes: RFC-0001 (the alpha contract)",
        );
        p.replace(
            "docs/rfc/rfc-0001-alpha.rst",
            ":Status: Accepted",
            ":Status: Superseded",
        );
    });
}

#[test]
fn p005_a_superseded_record_needs_a_newer_record_that_names_it() {
    failing(
        "P005",
        "rfc-0001",
        "is Superseded but no newer record names it in Supersedes",
        |p| {
            p.replace(
                "docs/rfc/rfc-0001-alpha.rst",
                ":Status: Accepted",
                ":Status: Superseded",
            )
        },
    );
    // Passing: the newer record names it (even while that record is still a draft).
    passing("P005", |p| {
        p.replace(
            "docs/rfc/rfc-0001-alpha.rst",
            ":Status: Accepted",
            ":Status: Superseded",
        );
        p.replace(
            "docs/rfc/rfc-0003-gamma.rst",
            ":Supersedes: none",
            ":Supersedes: RFC-0001 (the alpha contract)",
        );
    });
}

// ── P006 ────────────────────────────────────────────────────────────────

#[test]
fn p006_changes_targets() {
    failing(
        "P006",
        "rfc-0002",
        "does not exist and this record's changeset does not create it",
        |p| {
            p.replace(
                "docs/rfc/rfc-0002-beta.rst",
                ":Changes: spec/thing.rst (Contract; Rules)",
                ":Changes: spec/missing.rst (Contract)",
            );
        },
    );
    failing("P006", "rfc-0002", "section `Nonexistent`", |p| {
        p.replace(
            "docs/rfc/rfc-0002-beta.rst",
            ":Changes: spec/thing.rst (Contract; Rules)",
            ":Changes: spec/thing.rst (Nonexistent)",
        );
    });
    failing("P006", "rfc-0002", "not a maintained document path", |p| {
        p.replace(
            "docs/rfc/rfc-0002-beta.rst",
            ":Changes: spec/thing.rst (Contract; Rules)",
            ":Changes: src/main.rs (main)",
        );
    });
    passing("P006", |p| {
        p.mutate("docs/changeset/rfc-0002.rst", |s| {
            format!("{s}\nspec/other.rst\n--------------\n\nCreate: Other\n~~~~~~~~~~~~~\n\n:Status: Contract\n\nScope and authority\n^^^^^^^^^^^^^^^^^^^\n\ntext\n")
        });
        p.replace(
            "docs/rfc/rfc-0002-beta.rst",
            ":Changes: spec/thing.rst (Contract; Rules)",
            ":Changes: spec/thing.rst (Contract; Rules); spec/other.rst (created)",
        );
    });
}

// ── P007 ────────────────────────────────────────────────────────────────

#[test]
fn p007_unresolved_edit_and_orphan_changeset() {
    failing("P007", "rfc-0002.rst", "does not resolve", |p| {
        p.replace(
            "docs/changeset/rfc-0002.rst",
            "Replace: Contract",
            "Replace: Nope",
        )
    });
    failing("P007", "rfc-0009.rst", "changeset has no record", |p| {
        p.write("docs/changeset/rfc-0009.rst", "Changeset: RFC-0009\n===================\n\nspec/thing.rst\n--------------\n\nDelete: Contract\n~~~~~~~~~~~~~~~~\n");
    });
    failing("P007", "rfc-0002.rst", "not an edit", |p| {
        p.mutate("docs/changeset/rfc-0002.rst", |s| {
            format!("{s}\nChange: Contract\n~~~~~~~~~~~~~~~~\n\nx\n")
        })
    });
}

#[test]
fn p007_independent_changesets_editing_one_section() {
    let p = Proj::valid();
    let rfc3 = p.read("docs/rfc/rfc-0003-gamma.rst");
    let eps = rfc3
        .replace("RFC-0003: Gamma proposal", "RFC-0005: Epsilon")
        .replace(":Status: Draft", ":Status: Accepted")
        .replace(
            ":Accepted: none",
            ":Accepted: Sample Owner (2026-03-11T00:00Z)",
        )
        .replace(
            ":Depends: RFC-0002 (the beta rules it builds on)",
            ":Depends: none",
        );
    p.write("docs/rfc/rfc-0005-eps.rst", &eps);
    p.write(
        "docs/changeset/rfc-0005.rst",
        "Changeset: RFC-0005\n===================\n\nspec/thing.rst\n--------------\n\nInsert into: Contract\n~~~~~~~~~~~~~~~~~~~~~\n\nEps\n^^^\n\nEpsilon text.\n",
    );
    let f = p.lint();
    assert!(
        has(&f, "P007", "rfc-0002.rst", "neither depends on the other"),
        "{}",
        show(&f)
    );
    // With a dependency between them the conflict is gone.
    p.replace(
        "docs/rfc/rfc-0005-eps.rst",
        ":Depends: none",
        ":Depends: RFC-0002 (directly orders after the beta rules)",
    );
    let f = p.lint();
    assert!(
        !has(&f, "P007", "", "neither depends on the other"),
        "{}",
        show(&f)
    );
}

#[test]
fn p007_complete_record_with_edits() {
    failing("P007", "rfc-0002-beta", "marked complete", |p| {
        p.replace(
            "docs/rfc/rfc-0002-beta.rst",
            ":Implementation: not-started —",
            ":Implementation: complete —",
        );
    });
}

// ── P008 ────────────────────────────────────────────────────────────────

#[test]
fn p008_generated_files() {
    failing("P008", "docs/rfc/index.rst", "differs", |p| {
        p.mutate("docs/rfc/index.rst", |s| {
            s.replace("Alpha contract", "Alpha contract (edited)")
        })
    });
    failing("P008", "docs/staging/spec/thing.rst", "differs", |p| {
        p.mutate("docs/staging/spec/thing.rst", |s| {
            s.replace("two operations", "three operations")
        })
    });
    failing("P008", "docs/staging/spec/thing.rst", "missing", |p| {
        p.remove("docs/staging/spec/thing.rst")
    });
    failing("P008", "docs/changeset/index.rst", "missing", |p| {
        p.remove("docs/changeset/index.rst")
    });
    failing(
        "P008",
        "docs/staging/spec/orphan.rst",
        "no accepted changeset produces",
        |p| {
            p.write("docs/staging/spec/orphan.rst", "Orphan\n======\n");
        },
    );
    failing("P008", "docs/staging/spec/thing.rst", "differs", |p| {
        p.replace(
            "docs/changeset/rfc-0002.rst",
            "The beta rules apply to every operation.",
            "The beta rules apply to all operations.",
        )
    });
}

#[test]
fn p008_ignores_line_ending_conversion() {
    let p = Proj::valid_crlf();
    assert!(p.lint().is_empty(), "{}", show(&p.lint()));
}

// ── P009 / P010 / P011 ──────────────────────────────────────────────────

#[test]
fn p009_status_placement() {
    failing("P009", "phase.rst", "Status field", |p| {
        p.replace(
            "_palette/phase-1/phase.rst",
            "Goal\n----",
            ":Status: active\n\nGoal\n----",
        )
    });
    failing("P009", "deliverable-1", "checkboxes", |p| {
        p.replace(
            "_palette/phase-1/deliverables/deliverable-1-first-thing.rst",
            "- A person can run",
            "- [ ] A person can run",
        )
    });
    failing("P009", "state.rst", "done/pending marker", |p| {
        p.replace("_palette/state.rst", "- D-1 Use", "- D-1 (done) Use")
    });
    failing("P009", "state.rst", "checkboxes", |p| {
        p.replace("_palette/state.rst", "- D-1 Use", "- [x] D-1 Use")
    });
}

#[test]
fn p010_development_stages() {
    let cases = [
        (
            "_palette/backlog.rst",
            "Build the first thing.",
            "2026-09-29 landed the first thing.",
            "dated result",
        ),
        (
            "_palette/backlog.rst",
            "Build the first thing.",
            "Run wf_abc123def produced the first thing.",
            "run or dispatch identifier",
        ),
        (
            "_palette/state.rst",
            "Should the thing support a third operation?",
            "Use 3 sonnet subagents for the thing.",
            "model-routing",
        ),
        (
            "_palette/phase-1/phase.rst",
            "Everything else builds on it.",
            "Task 3f2a9c1e-1234-4bcd-8ef0-0123456789ab finished.",
            "run or dispatch identifier",
        ),
    ];
    for (file, from, to, needle) in cases {
        let p = Proj::valid();
        p.replace(file, from, to);
        let f = p.lint();
        assert!(
            f.iter().any(|x| x.rule == "P010"
                && x.severity == Severity::Warning
                && x.message.contains(needle)),
            "{file}: {}",
            show(&f)
        );
    }
    passing("P010", |p| {
        p.replace(
            "_palette/backlog.rst",
            "Build the first thing.",
            "Build the first thing; the model decides how.",
        )
    });
}

#[test]
fn p011_budgets() {
    let long = "filler line\n".repeat(301);
    failing("P011", "state.rst", "state has", |p| {
        p.mutate("_palette/state.rst", |s| {
            s.replace("Open questions\n", &format!("{long}\nOpen questions\n"))
        })
    });
    failing("P011", "state.rst", "state entry has 4 lines", |p| {
        p.replace("_palette/state.rst", "- D-1 Use the sample layout. Source: Sample Owner, 2026-08-01. Target:\n  docs/design/overview.rst.", "- D-1 Use the sample layout.\n  Source: Sample Owner, 2026-08-01.\n  Target:\n  docs/design/overview.rst.");
    });
    failing("P011", "phase.rst", "keep it within 120", |p| {
        p.mutate("_palette/phase-1/phase.rst", |s| {
            format!("{s}{}", "- more\n".repeat(120))
        })
    });
    failing("P011", "deliverable-1", "keep it within 120", |p| {
        p.mutate(
            "_palette/phase-1/deliverables/deliverable-1-first-thing.rst",
            |s| format!("{s}{}", "- more\n".repeat(120)),
        )
    });
    failing("P011", "backlog.rst", "body of 5 lines", |p| {
        p.replace(
            "_palette/backlog.rst",
            "Build the first thing.",
            "one\ntwo\nthree\nfour\nfive",
        )
    });
    assert!(Proj::valid().lint().iter().all(|f| f.rule != "P011"));
}

// ── P012 / P013 / P014 ──────────────────────────────────────────────────

#[test]
fn p012_layout() {
    failing("P012", "layout.rst", "family `glossary` is missing", |p| {
        p.replace("_palette/layout.rst", ":glossary: docs/glossary.rst\n", "")
    });
    failing(
        "P012",
        "layout.rst",
        "`deliverables` is not a document family",
        |p| {
            p.replace(
                "_palette/layout.rst",
                ":deliverable: internal",
                ":deliverables: internal",
            )
        },
    );
    failing("P012", "layout.rst", "leaves the project", |p| {
        p.replace(
            "_palette/layout.rst",
            ":spec: docs/spec",
            ":spec: ../outside",
        )
    });
    failing("P012", "layout.rst", "inside `_palette/`", |p| {
        p.replace(
            "_palette/layout.rst",
            ":adr: docs/adr",
            ":adr: _palette/adr",
        )
    });
    failing("P012", "layout.rst", "absolute", |p| {
        p.replace("_palette/layout.rst", ":adr: docs/adr", ":adr: /etc/adr")
    });
    failing("P012", "layout.rst", "both placed at", |p| {
        p.replace("_palette/layout.rst", ":adr: docs/adr", ":adr: docs/rfc")
    });
}

#[test]
fn p013_backlog() {
    failing("P013", "backlog.rst", "duplicate item id B-3", |p| {
        p.replace(
            "_palette/backlog.rst",
            "B-4 Fourth thing\n~~~~~~~~~~~~~~~~",
            "B-3 Fourth thing\n~~~~~~~~~~~~~~~~",
        )
    });
    failing("P013", "backlog.rst", "in-phase-9", |p| {
        p.replace(
            "_palette/backlog.rst",
            ":Status: approved",
            ":Status: in-phase-9",
        )
    });
    failing("P013", "backlog.rst", "does not exist", |p| {
        p.remove("_palette/phase-1/deliverables/deliverable-1-first-thing.rst")
    });
    let p = Proj::valid();
    p.replace(
        "_palette/backlog.rst",
        ":Outcome: RFC-0001",
        ":Outcome: none",
    );
    let f = p.lint();
    assert!(
        f.iter().any(|x| x.rule == "P013"
            && x.severity == Severity::Warning
            && x.message.contains("Outcome is none")),
        "{}",
        show(&f)
    );
}

#[test]
fn p014_time_varying_fields() {
    failing(
        "P014",
        "rfc-0002",
        "`:Implementation:` is not in its template format",
        |p| {
            p.replace(
                "docs/rfc/rfc-0002-beta.rst",
                ":Implementation: not-started —",
                ":Implementation: begun —",
            )
        },
    );
    failing("P014", "rfc-0002", "`:Implementation:`", |p| {
        p.replace(
            "docs/rfc/rfc-0002-beta.rst",
            ":Implementation: not-started — the beta rules section of the thing spec",
            ":Implementation: not-started",
        )
    });
    failing("P014", "rfc-0002", "`:Verification:`", |p| {
        p.replace(
            "docs/rfc/rfc-0002-beta.rst",
            ":Verification: documentation — 2026-02-10; decision record only",
            ":Verification: maybe — 2026-02-10; x",
        )
    });
    failing("P014", "rfc-0002", "`:Revised:`", |p| {
        p.replace(
            "docs/rfc/rfc-0002-beta.rst",
            ":Revised: none",
            ":Revised: yesterday — fixed",
        )
    });
    failing("P014", "rfc-0002", "`:Implementers:`", |p| {
        p.replace(
            "docs/rfc/rfc-0002-beta.rst",
            ":Implementers: none yet",
            ":Implementers:",
        )
    });
    passing("P014", |p| {
        p.replace(
            "docs/rfc/rfc-0002-beta.rst",
            ":Revised: none",
            ":Revised: 2026-02-11 — fixed a typo; 2026-02-12 — fixed another",
        )
    });
}

// ── The failures that motivated the document system, at small scale ────

#[test]
fn motivating_failure_oversized_state() {
    let p = Proj::valid();
    let mut entries = String::new();
    for n in 3..320 {
        entries.push_str(&format!(
            "- D-{n} Decision number {n}. Source: Sample Owner, 2026-08-01. Target: docs/x.rst.\n"
        ));
    }
    p.mutate("_palette/state.rst", |s| {
        s.replace(
            "\nOpen questions\n",
            &format!("{entries}\nOpen questions\n"),
        )
    });
    assert!(
        p.lint()
            .iter()
            .any(|f| f.rule == "P011" && f.message.contains("state has"))
    );
}

#[test]
fn motivating_failure_status_recorded_in_two_files() {
    let p = Proj::valid();
    p.replace(
        "_palette/phase-1/deliverables/deliverable-1-first-thing.rst",
        ":Backlog: B-1",
        ":Backlog: B-1\n:Status: done",
    );
    p.replace("_palette/state.rst", "- D-1 Use", "- D-1 (pending) Use");
    let f = p.lint();
    assert!(
        has(&f, "P009", "deliverable-1", "Status field"),
        "{}",
        show(&f)
    );
    assert!(
        has(&f, "P009", "state.rst", "done/pending marker"),
        "{}",
        show(&f)
    );
}

#[test]
fn motivating_failure_dependency_cycle() {
    let p = Proj::valid();
    p.replace(
        "docs/rfc/rfc-0001-alpha.rst",
        ":Depends: none",
        ":Depends: RFC-0003 (a newer record)",
    );
    let f = p.lint();
    assert!(has(&f, "P005", "rfc-0001", "created later"), "{}", show(&f));
    assert!(
        has(&f, "P005", "rfc-0001", "relation cycle"),
        "{}",
        show(&f)
    );
}

// ── Output shape ────────────────────────────────────────────────────────

#[test]
fn findings_are_ordered_errors_first_and_render_as_json() {
    let p = Proj::valid();
    p.replace(
        "docs/rfc/rfc-0003-gamma.rst",
        ":Related: none",
        ":Related: RFC-0002 (context)",
    );
    p.replace(
        "docs/principles.rst",
        "Keep the thing small",
        "Keep the thing small\n--",
    );
    p.mutate("docs/glossary.rst", |s| format!("{s}\n.. note:: x\n"));
    let f = p.lint();
    let first_warning = f
        .iter()
        .position(|x| x.severity == Severity::Warning)
        .expect("a warning");
    assert!(
        f[..first_warning]
            .iter()
            .all(|x| x.severity == Severity::Error)
    );
    assert!(
        f[first_warning..]
            .iter()
            .all(|x| x.severity == Severity::Warning)
    );
    let json: serde_json::Value = serde_json::from_str(&lint::to_json(&f)).expect("json");
    assert_eq!(json["errors"].as_u64().expect("n") as usize, first_warning);
    let first = &json["findings"][0];
    for key in ["rule", "severity", "file", "line", "message"] {
        assert!(first.get(key).is_some(), "missing {key}");
    }
}

// ── P015 ─────────────────────────────────────────────────────────────────

#[test]
fn p015_stray_files() {
    failing(
        "P015",
        "_palette/notes/research.md",
        "belongs to no document family",
        |p| {
            p.write("_palette/notes/research.md", "# notes\n");
        },
    );
    failing(
        "P015",
        "docs/rfc/README.md",
        "belongs to no document family",
        |p| {
            p.write("docs/rfc/README.md", "readme\n");
        },
    );
    failing(
        "P015",
        "docs/spec/drafts",
        "belongs to no document family",
        |p| {
            std::fs::create_dir_all(p.path("docs/spec/drafts")).expect("mk");
        },
    );
    passing("P015", |_| {});
}

// ── RFC-0009: partial supersession, Amends, Accepted, amends-until ────────

#[test]
fn p005_partial_supersession_leaves_the_older_record_accepted() {
    passing("P005", |p| {
        p.replace(
            "docs/rfc/rfc-0003-gamma.rst",
            ":Supersedes: none",
            ":Supersedes: RFC-0001 (in part: the open operation)",
        );
    });
    // A partially superseded record that is also wholly superseded elsewhere is
    // Superseded without complaint.
    passing("P005", |p| {
        p.replace(
            "docs/rfc/rfc-0002-beta.rst",
            ":Supersedes: none",
            ":Supersedes: RFC-0001 (in part: the open operation)",
        );
        p.replace(
            "docs/rfc/rfc-0003-gamma.rst",
            ":Supersedes: none",
            ":Supersedes: RFC-0001 (the alpha contract)",
        );
        p.replace(
            "docs/rfc/rfc-0001-alpha.rst",
            ":Status: Accepted",
            ":Status: Superseded",
        );
    });
    // A partial entry alone does not justify Superseded.
    failing(
        "P005",
        "rfc-0001",
        "no newer record names it in Supersedes as a whole",
        |p| {
            p.replace(
                "docs/rfc/rfc-0003-gamma.rst",
                ":Supersedes: none",
                ":Supersedes: RFC-0001 (in part: the open operation)",
            );
            p.replace(
                "docs/rfc/rfc-0001-alpha.rst",
                ":Status: Accepted",
                ":Status: Superseded",
            );
        },
    );
}

fn with_amends(p: &Proj, until: &str) {
    if until != "none" {
        p.replace(
            "_palette/layout.rst",
            ":amends-until: none",
            &format!(":amends-until: {until}"),
        );
    }
    p.replace(
        "docs/rfc/rfc-0003-gamma.rst",
        ":Related: none\n",
        ":Related: none\n:Amends: RFC-0001 (the open operation)\n",
    );
}

#[test]
fn p005_amends_is_admitted_up_to_the_layout_date() {
    // RFC-0003 is dated 2026-03-10.
    passing("P005", |p| with_amends(p, "2026-03-10"));
    passing("P005", |p| with_amends(p, "2026-12-31"));
    failing(
        "P005",
        "rfc-0003",
        "admitted on records dated up to 2026-03-09",
        |p| with_amends(p, "2026-03-09"),
    );
    failing("P005", "rfc-0003", "`amends-until` admits none", |p| {
        with_amends(p, "none")
    });
    failing("P005", "adr-0001", "an ADR amends only an ADR", |p| {
        p.replace(
            "_palette/layout.rst",
            ":amends-until: none",
            ":amends-until: 2026-12-31",
        );
        p.replace(
            "docs/adr/adr-0001-naming.rst",
            ":Related: none\n",
            ":Related: none\n:Amends: RFC-0001 (the names)\n",
        );
    });
    // Amends follows the age rule like every relation.
    failing("P005", "rfc-0001", "created later", |p| {
        p.replace(
            "_palette/layout.rst",
            ":amends-until: none",
            ":amends-until: 2026-12-31",
        );
        p.replace(
            "docs/rfc/rfc-0001-alpha.rst",
            ":Related: none\n",
            ":Related: none\n:Amends: RFC-0002 (the beta rules)\n",
        );
    });
}

#[test]
fn p002_amends_sits_after_related_with_a_relation_value() {
    failing("P002", "rfc-0003", "belongs right after `:Related:`", |p| {
        p.replace(
            "_palette/layout.rst",
            ":amends-until: none",
            ":amends-until: 2026-12-31",
        );
        p.replace(
            "docs/rfc/rfc-0003-gamma.rst",
            ":Changes: none\n",
            ":Changes: none\n:Amends: RFC-0001 (the open operation)\n",
        );
    });
    failing("P002", "rfc-0003", "`:Amends:` has a value outside", |p| {
        p.replace(
            "_palette/layout.rst",
            ":amends-until: none",
            ":amends-until: 2026-12-31",
        );
        p.replace(
            "docs/rfc/rfc-0003-gamma.rst",
            ":Related: none\n",
            ":Related: none\n:Amends: RFC-0001\n",
        );
    });
}

#[test]
fn p002_amends_needs_at_least_one_entry() {
    failing("P002", "rfc-0003", "has no entry", |p| {
        p.replace(
            "_palette/layout.rst",
            ":amends-until: none",
            ":amends-until: 2026-12-31",
        );
        p.replace(
            "docs/rfc/rfc-0003-gamma.rst",
            ":Related: none\n",
            ":Related: none\n:Amends: ;\n",
        );
    });
}

#[test]
fn p005_repeated_supersedes_targets_are_all_seen() {
    // A partial and a whole entry for the same record: the whole one counts.
    passing("P005", |p| {
        p.replace(
            "docs/rfc/rfc-0003-gamma.rst",
            ":Supersedes: none",
            ":Supersedes: RFC-0001 (in part: the names); RFC-0001 (the alpha contract)",
        );
        p.replace(
            "docs/rfc/rfc-0001-alpha.rst",
            ":Status: Accepted",
            ":Status: Superseded",
        );
    });
}

#[test]
fn p012_amends_until_is_required_and_a_date_or_none() {
    failing("P012", "layout.rst", "`amends-until` is missing", |p| {
        p.replace("_palette/layout.rst", ":amends-until: none\n", "");
    });
    failing("P012", "layout.rst", "`none` or a date", |p| {
        p.replace(
            "_palette/layout.rst",
            ":amends-until: none",
            ":amends-until: soon",
        );
    });
}

#[test]
fn p016_accepted_without_the_time_is_a_warning() {
    let p = Proj::valid();
    p.replace(
        "docs/rfc/rfc-0001-alpha.rst",
        ":Accepted: Sample Owner (2026-01-10T00:00Z)",
        ":Accepted: Sample Owner",
    );
    let f = p.lint();
    assert!(
        has(
            &f,
            "P016",
            "rfc-0001",
            "names who accepted the record but not when"
        ),
        "{}",
        show(&f)
    );
    assert!(
        f.iter().all(|x| x.severity == Severity::Warning),
        "{}",
        show(&f)
    );
    // The earlier `<date>, <who>` form is the same warning, not an error.
    let p = Proj::valid();
    p.replace(
        "docs/rfc/rfc-0001-alpha.rst",
        ":Accepted: Sample Owner (2026-01-10T00:00Z)",
        ":Accepted: 2026-01-10, Sample Owner",
    );
    let f = p.lint();
    assert!(
        has(
            &f,
            "P016",
            "rfc-0001",
            "names who accepted the record but not when"
        ) && f.iter().all(|x| x.severity == Severity::Warning),
        "{}",
        show(&f)
    );
    // A time that is not on the calendar or the clock is an error.
    failing("P002", "rfc-0001", "is not a UTC time", |p| {
        p.replace(
            "docs/rfc/rfc-0001-alpha.rst",
            ":Accepted: Sample Owner (2026-01-10T00:00Z)",
            ":Accepted: Sample Owner (2026-13-40T99:99Z)",
        );
    });
    failing("P002", "rfc-0001", "is not a calendar date", |p| {
        p.replace(
            "docs/rfc/rfc-0001-alpha.rst",
            ":Date: 2026-01-10",
            ":Date: 2026-02-30",
        );
    });
    // A parenthetical that is not a UTC time is an error, not a longer name.
    failing(
        "P002",
        "rfc-0001",
        "the parenthetical is not a UTC time",
        |p| {
            p.replace(
                "docs/rfc/rfc-0001-alpha.rst",
                ":Accepted: Sample Owner (2026-01-10T00:00Z)",
                ":Accepted: Sample Owner (2026-01-10 09:00)",
            );
        },
    );
}

#[test]
fn p007_and_p014_accept_every_implementation_value() {
    for v in [
        "in-progress",
        "partial",
        "abandoned",
        "unassessed",
        "not-applicable",
    ] {
        // RFC-0002 has a changeset with edits; only `complete` is refused then.
        passing("P007", |p| {
            p.replace(
                "docs/rfc/rfc-0002-beta.rst",
                ":Implementation: not-started — the beta rules section of the thing spec",
                &format!(":Implementation: {v} — the beta rules section of the thing spec"),
            );
        });
        passing("P014", |p| {
            p.replace(
                "docs/rfc/rfc-0002-beta.rst",
                ":Implementation: not-started — the beta rules section of the thing spec",
                &format!(":Implementation: {v} — the beta rules section of the thing spec"),
            );
        });
    }
}
