RFC-0016: Bench runs in the user's harness home
===============================================

:Status: Accepted
:Implementation: not-started — the bench crate and its contract; the benchmark
  deliverables
:Verification: none — 2026-09-30; not verified yet
:Areas: bench server; measurement; agent-exec
:Authors: Claude Fable 5.1
:Reviewers: Hamin Sung
:Implementers: none yet
:Accepted: Hamin Sung (2026-09-30T22:37Z)
:Date: 2026-09-30
:Revised: none
:Depends: RFC-0011 (the bench server, its record schema and the paired design);
  RFC-0008 (direct use of agent-exec's RunSpec isolation to run in the user's
  home)
:Supersedes: RFC-0011 (in part: the isolated backend home, the kit and none
  harness conditions and the kit installation per run)
:Related: none
:Changes: spec/bench.rst (Cell; Run; Isolation; Tools; Conformance)
:Description: The bench server runs every cell in the user's own harness home
  with the kit installed there, dropping the isolated home, the kit and no-kit
  conditions and the auth field.

Summary
-------

RFC-0011 ran each benchmark cell in a backend home created for the run, with the kit installed there at a pinned version or absent for a ``none`` condition. Every cell now runs in the user's own harness home, as aside and dispatch do, with the kit and the MCP servers installed there; the record carries the installed kit version. The isolated home, the ``kit`` input, the ``harness`` element of a cell and the ``auth`` field are removed.

Problem and context
-------------------

The isolated home was added for experimental control: a pinned kit version and a no-kit comparison. Neither serves the phase: B-8 asks which model and effort suit which work under the kit as it is used, and the kit is always installed there. The isolated home then caused every further problem of the design: the backend's credentials live in the same store as its configuration, so the run had to carry them (an ``auth`` field with a symlink into the user's credential file); the kit had to be installed per run, which registered MCP servers that only the claude backend loaded; and the version probe and the installer ran outside the isolation they were meant to keep. Running in the user's home removes all of them and measures the condition the user runs.

Evidence
--------

- Deliverable 10 and ``spec/bench.rst`` (2026-09-30): the ``harness: kit | none`` cell element, the ``kit`` input and the per-run installation.
- The bench crate review (2026-09-30, aside): the credential symlink lets a token refresh write through to the user's file; the ``--version`` probe runs unisolated and unbounded; kit runs on codex ignore the installed servers (``IgnoreUserConfig``) while claude runs load them.
- aside and dispatch run their backends in the user's home with no credential handling (``spec/agent-exec.rst``).
- The user, 2026-09-30: run as is; the difference between the users' harnesses is what the measurement should show, and a run can turn a server off itself when it needs to.

Goals and non-goals
-------------------

Goals: every run authenticates and loads the kit exactly as the user's own sessions do; nothing of the user's credentials is read, copied or linked; the record says which kit version the run saw. Non-goals: a no-kit condition; a kit version other than the installed one; isolating the backend's session logs from the user's store.

Requirements and invariants
---------------------------

- A cell is (backend, model, effort); a grid holds no harness condition.
- A run reads the user's harness home and writes there only what the backend writes on its own (session logs); the server writes nothing there.
- ``cell.kit_version`` is read from the kit manifest in the harness home when the run starts, ``null`` when none is installed; it is recorded, never chosen.
- No credential file is read, copied, linked or logged by the server.

Design
------

A run executes in the user's harness home: codex with ``CODEX_HOME`` untouched and without ``--ignore-user-config``; claude with ``CLAUDE_CONFIG_DIR`` untouched and the ``Permissions`` isolation. The working tree is the run's ``work/`` under a ``WorkspaceWrite`` sandbox, the prompt on stdin, the guard on, as before. ``bench_grid`` loses ``kit`` and ``auth``; ``grid.toml`` records the kit versions seen. The run directory keeps ``work/``, ``stdout``, ``stderr`` and ``final.md``; there is no ``home/``. ``cli_versions`` carries the backend CLI's ``--version``. A record whose ``kit_version`` differs from the grid's first is noted in ``log.txt`` so that a kit reinstall during a grid is visible.

Impact and compatibility
------------------------

Removed: the isolated home, per-run kit installation, ``auth`` and ``link-credentials``, the no-kit condition; the crate's ``kit.rs`` and ``auth.rs``. Kept: the record schema otherwise, resumption, cancellation, pricing. Session logs of benchmark runs accumulate in the user's harness stores. A kit reinstall during a grid changes the condition; the record makes it visible. Kit-effect comparisons are outside the suite. Deliverables 10 and 11 lose their pinned-kit and no-kit lines. The bench crate is unreleased, so no record in use carries the old ``cell.harness`` element.

Implementation and transition
-----------------------------

One change to the bench crate (remove the home, installer and auth paths; read the manifest; adjust the record, the request checks and the tests), the changeset of this record promoted with it, and the two deliverables edited. No release step of its own: the crate ships with the bench server's first release.

Verification strategy
---------------------

- ``cargo test -p bench``: records carry ``kit_version`` from a manifest fixture and ``null`` without one; a grid over a stub backend adds only session files under the stub's harness store; no code path opens a credential file (tests assert the run environment and argv).
- Before the first wave: one dry run and one real run per backend in the user's home, recording usage and cost.

Alternatives and costs
----------------------

- Isolated home with linked credentials (RFC-0011 as implemented): a reproducible kit version and a no-kit condition, at the cost of carrying the user's credential into each run, installing the kit per run, asymmetric MCP loading, and no path for macOS claude credentials held in the Keychain.
- Isolated home with the kit placed at project level in the working tree: no credential handling, but the kit is rendered for user-level installation and would load by a different path than the users' own.

Open questions
--------------

None.

References
----------

- ``spec/bench.rst``, ``spec/agent-exec.rst``
- Deliverables 10 and 11 of phase 2
