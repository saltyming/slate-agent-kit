RFC-0015: The benchmark suite is committed
==========================================

:Status: Accepted
:Implementation: not-started — the benchmarks/ directory with its license texts
  and README, and the suite's design document
:Verification: none — 2026-09-30; not verified yet
:Areas: benchmarks; licensing; repository layout
:Authors: Claude Fable 5.1
:Reviewers: Hamin Sung
:Implementers: none yet
:Accepted: Hamin Sung (2026-09-30T14:20Z)
:Date: 2026-09-30
:Revised: none
:Depends: RFC-0011 (direct use of the benchmark suite and the task layout the
  bench server runs)
:Supersedes: RFC-0011 (in part: the suite and its fixtures stay outside any
  public path)
:Related: none
:Changes: design/benchmark-suite.rst (Ownership and state)
:Description: The benchmark fixtures, tasks, scorers and hidden tests live in
  this repository under benchmarks/ with per-file licenses; only run results
  stay outside it.

Summary
-------

RFC-0011 kept the benchmark suite outside any public path. The suite is committed instead: fixtures extracted from saltyos, tasks, scorers and hidden tests live under ``benchmarks/`` in this repository, each file under the license of the saltyos component it came from. Run results are not committed; they go where each run directs.

Problem and context
-------------------

The licenses of the excerpts were settled component by component (D-26, D-31) because the excerpts were meant to be published with the kit, so that anyone can rerun the benchmarks the kit's routing evidence rests on. The record and the deliverable said the opposite, that fixtures are never committed; that was an error of the draft, not a decision.

Evidence
--------

- Decisions D-26 and D-31 of this repository's state (2026-09-30): an excerpt takes the license of its saltyos component, ``lib/`` excerpts LGPL-2.1-or-later, kernel and userland GPL-2.0-only, corrected in the extracted copies only.
- The user, 2026-09-30: the fixtures are committed; that is why the licenses were asked.
- This repository's crates declare ``license = "MIT"``; saltyos's root license is GPL-2.0-only and five of its ``lib/`` components carry LGPL-2.1-or-later.

Goals and non-goals
-------------------

Goals:

- A checkout of this repository contains everything needed to rerun a benchmark except the models.
- Every committed excerpt states its license, and the directory states that it is not under the repository's own license.

Non-goals:

- Committing run results.
- Changing any file in the saltyos tree.

Requirements and invariants
---------------------------

- ``benchmarks/`` holds ``fixtures/<name>/`` (the excerpt and its manifest), ``tasks/<benchmark>/<task>/`` (``task.toml``, the prompt, optional ``documents/``), ``scorers/`` and ``hidden/`` (tests that never enter a run's working tree).
- Every extracted source file carries the SPDX identifier of its component; ``benchmarks/LICENSES/`` holds the full texts of GPL-2.0 and LGPL-2.1; ``benchmarks/README`` says the directory follows per-file SPDX identifiers, not the repository's license.
- Hidden tests are committed; "hidden" means absent from a run's working tree while the run lasts.
- Run results (the bench server's JSONL and run directories) are never committed by the suite; the first wave's analysis is written into the backlog item it feeds.
- The kit renders, the rule checks and the release artifacts do not include ``benchmarks/``.

Design
------

The suite's design document states where the suite lives and who owns what; the contract of a task directory stays in ``spec/bench.rst``. ``benchmarks/`` is plain files: nothing in the Rust workspace depends on it, and the bench server is pointed at ``benchmarks/tasks`` by path like at any task set.

Impact and compatibility
------------------------

- The repository gains files under GPL-2.0-only and LGPL-2.1-or-later beside its MIT sources, as separate works in their own directory.
- Publishing hidden tests and seeded-defect lists means a model trained after publication may have seen them; the models measured in the first wave have not.

Implementation and transition
-----------------------------

The directory is created with the first extracted fixture; the license texts and the README come with it.

Verification strategy
---------------------

- Every file under ``benchmarks/fixtures`` and ``benchmarks/hidden`` that was copied from saltyos has an SPDX line matching its component.
- ``benchmarks/README`` and ``benchmarks/LICENSES/`` exist; no result file is tracked.
- ``validate.sh`` and the kit renders are unaffected by the directory.

Alternatives and costs
----------------------

- Keep the suite outside the repository: the evidence behind the kit's routing could not be rerun by anyone else.
- A separate repository for the suite: one more thing to keep in step with the bench server's task layout.

Open questions
--------------

None.

References
----------

- `RFC-0011 <rfc-0011-measurement.rst>`_
