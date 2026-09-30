RFC-0008: Shared backend execution layer
========================================

:Status: Draft
:Implementation: not-started — the agent-exec crate, harness-log moved beside it
  with usage parsers, aside and dispatch rebuilt on both, copilot removed from
  servers, installer, prefs and rules
:Verification: none — 2026-09-30; not verified yet
:Areas: MCP servers; aside; dispatch; installer; rules; prefs
:Authors: Claude Fable 5.1
:Reviewers: none yet
:Implementers: none yet
:Accepted: none
:Date: 2026-09-30
:Revised: none
:Depends: RFC-0002 (the prefs schema and the server registration that the
  copilot removal edits)
:Supersedes: none
:Related: none
:Changes: spec/agent-exec.rst (created); design/architecture.rst (Sources;
  Interfaces and dependencies); spec/prefs.rst (Settings)
:Description: One crate runs every backend CLI invocation for aside, dispatch
  and later servers and returns a record with normalized token usage; copilot
  leaves the kit.

Summary
-------

Backend CLI invocation moves out of aside and dispatch into one library crate,
``shared/crates/agent-exec``: it looks the binary up, builds the argv, spawns
under a guard, captures output within caps, classifies a failure and retries
along a model fallback chain, and returns a ``RunRecord`` whose ``Usage`` holds
token counts in one normalized set of buckets. ``harness-log`` moves to
``shared/crates/`` and gains the parsers that read usage from each backend's
own output. aside and dispatch keep their behavior and differ only in the
parameters they pass. The copilot backend leaves the servers, the installer,
the prefs and the rules, so two backends per server remain to unify. aside
adopts dispatch's parent-death guard.

Problem and context
-------------------

aside and dispatch each carry a full copy of the backend execution layer: the
backend enumeration, the argv builders, the binary lookup and install hints,
the failure classifier and the fallback loop. The copies have already drifted
in comments and capture caps, and a defect found in one has to be found again
in the other. A third server that runs backends (the bench server of phase 2)
would take a third copy.

Neither server reads the token usage its backends report, although codex,
claude and opencode all report it in their headless output. Phase 2 needs those
counts per run, and the harness-log library, which is where a per-run parser
belongs, only locates codex sessions today.

copilot is the one backend whose token reporting is unverified and the one the
user no longer runs. Removing it before the extraction leaves two backends per
server to unify instead of three.

dispatch runs its backends under a parent-death guard so that a killed dispatch
takes its subtree with it; aside does not, so a backend that aside started
outlives an aside server that was killed.

Evidence
--------

Measured on the ``next`` branch at 9cd43eb, 2026-09-30:

- ``errkind.rs`` is 248 lines in both servers and identical except for two
  comment lines that name the other server.
- ``aside/src/backend.rs`` (550 lines) and ``dispatch/src/backend.rs`` (773
  lines) each define ``Backend``, ``build_command``, ``which``,
  ``install_hint``, ``version`` and a re-entry depth marker; the fallback loop
  is in ``aside/src/main.rs`` (``dispatch`` method) and
  ``dispatch/src/executor.rs`` (``run``), both iterating primary model then
  chain and advancing on ``errkind::classify(...).is_retry_worthy()``. Capture
  caps differ: 50 KB stdout and 2 KB stderr in aside, 200 KB and 16 KB in
  dispatch.
- ``which()`` in both servers appends only ``.exe`` on Windows; the
  npm-installed CLIs are ``.cmd`` shims there.
- ``harness-log`` is 397 lines, all codex session location; no file under
  ``shared/`` parses token usage.
- copilot is named in 17 files under ``shared/``: the aside backend, main,
  params and errkind, the dispatch errkind, the aside prefs template, the
  installer's schema, migration, document parser and Kimi pre-approval list,
  and the installer's tests and fixtures.
- dispatch's guard: ``pdeath_guard.rs`` (215 lines, Linux and macOS) and
  ``winjob.rs`` (139 lines); aside relies on ``kill_on_drop`` alone, which
  covers a cancelled request but not a killed server.
- Token reporting, verified 2026-09-30 with codex-cli 0.157.0 and Claude Code
  2.1.285: codex ``exec --json`` emits ``turn.completed.usage`` with
  ``input_tokens``, ``cached_input_tokens``, ``cache_write_input_tokens``,
  ``output_tokens`` and ``reasoning_output_tokens``, and a rollout carries
  ``token_count`` events with ``last_token_usage`` and ``total_token_usage``;
  claude ``-p --output-format json`` reports
  ``usage.output_tokens_details.thinking_tokens``,
  ``usage.cache_creation.ephemeral_5m_input_tokens`` and
  ``ephemeral_1h_input_tokens``, ``usage.iterations[]`` and
  ``modelUsage.<model>.costBasis``; an opencode message carries
  ``tokens.input``, ``output``, ``reasoning``, ``cache.read`` and
  ``cache.write``. copilot: not verified.
- Bucket rules as codeburn 0.9.25 applies them (``src/models.ts``,
  ``src/providers/codex.ts``): reasoning tokens are inside output tokens for
  OpenAI and Anthropic; OpenAI ``input_tokens`` includes the cached part, so
  the uncached count is ``max(0, input - cached)``; codex
  ``cache_write_input_tokens`` is carved out of the uncached input; a rollout
  delta uses ``last_token_usage`` when present, else the change in
  ``total_token_usage``.
- Deliverable 8 of phase 2 fixes the crate's surface, the ``Usage`` buckets and
  the per-server parameters; deliverable 9 fixes the copilot removal (D-24,
  D-30).

Goals and non-goals
-------------------

Goals:

- One implementation of backend lookup, argv construction, spawning, capture,
  failure classification and fallback, used by every server that runs a
  backend.
- Token usage of every run available to the caller in one normalized form, with
  an unknown count distinguishable from zero.
- aside and dispatch behave as before: the same flags in the same order, the
  same isolation, the same caps and failure text, the same anti-recursion.
- codex and claude remain aside's backends; codex, claude and opencode remain
  dispatch's.
- A backend started by aside dies with aside, as one started by dispatch dies
  with dispatch.

Non-goals:

- New server behavior: dispatch's steer, unassociated-rollout watchdog, SQLite
  store and prompt rendering, and aside's transcript redaction and result
  rendering stay where they are.
- Cost computation, grids and result files: the bench server's record.
- Replacing copilot with another backend.

Requirements and invariants
---------------------------

- No server contains a second implementation of anything the crate exports.
- ``Usage`` buckets are disjoint counts of the same request:
  ``input_uncached``, ``cache_read``, ``cache_write_5m``, ``cache_write_1h``
  and ``output``; ``reasoning`` is a part of ``output``, never added to it; a
  count the source does not report is ``None``; every record names its
  ``usage_source``.
- The argv each server sends today is preserved as a set of flags and their
  order: for aside, codex ``exec --ignore-user-config`` after ``exec`` and
  claude in safe mode without session persistence with the read-only tool list;
  for dispatch, ``-c mcp_servers.dispatch.enabled=false`` before ``exec``, the
  sandbox to permission-mode mapping, and pinned and resumed sessions.
- The prompt travels on stdin for every backend.
- Failure text for classification is stderr followed by stdout, since claude
  prints the discriminating error on stdout.
- The crate depends on no C library; the SQLite store stays in dispatch.
- Both servers run their backends under the parent-death guard on Linux and
  macOS and the Job Object on Windows, and check for the guard re-invocation
  before anything else in ``main``.
- A prefs file that still holds copilot values installs; the values are
  reported in one warning line and treated as blank.

Design
------

The contract is ``spec/agent-exec.rst`` in this record's changeset: the crate's
types (``Backend``, ``Sandbox``, ``Isolation``, ``OutputMode``, ``RunSpec``,
``RunEvent``, ``RunRecord``, ``Usage``), its functions (``run``,
``run_with_fallback``, ``errkind``, ``which``, ``version``, ``install_hint``,
``reentry``, ``guard``), the per-server parameters, the usage buckets and their
mapping from each backend's fields, and what ``harness-log`` exports.

The differences between aside and dispatch are values of ``RunSpec``, not
branches: codex anti-recursion is ``Isolation`` (ignore the user config, or
disable one MCP server), claude isolation is the sandbox mapping plus a tool
allowlist and session flags, and capture caps are fields. opencode runs through
its server API as today; the runner reports its progress through ``RunEvent``
and the caller (dispatch) writes its store.

``harness-log`` keeps the session location functions and gains a parser per
usage source that returns ``Usage``: a codex ``--json`` stream, a codex
rollout, a claude ``--output-format json`` result and an opencode message.

copilot is removed rather than carried: its ``Backend`` variant, argv builder,
tool, prefs settings and schema entries go, and the consultation rule states
the two backends that remain.

Impact and compatibility
------------------------

- aside gains the guard: a killed aside server no longer leaves a backend
  running. Its startup path checks the guard argv first, as dispatch's does.
- The ``aside_copilot`` tool disappears; a call to it fails as an unknown tool.
  The aside prefs lose the three copilot settings; a user file that still has
  them installs with one warning line, and a ``copilot`` backend value is
  treated as blank, so the agent chooses the backend as it does without a prefs
  value.
- ``harness-log`` changes path; the workspace members, ``validate.sh``'s
  required paths and the architecture design document follow.
- Server binaries and argv are otherwise unchanged; no MCP tool of dispatch
  changes its parameters or output.
- Every server that runs a backend can now attach token usage to a run; the
  bench server is the first consumer.

Implementation and transition
-----------------------------

copilot leaves first, so that the extraction unifies two backends per server
(D-30). The crate is then extracted with aside and dispatch switched to it in
the same change, since a period with three copies would defeat the purpose.
``harness-log`` moves with the extraction. Both changes ship through the
release train: kit version bumps, changelogs, kit CI, tags.

Verification strategy
---------------------

- The argv tests of aside and dispatch keep asserting the flag set and order
  named in the requirements; the one aside test that asserts the argv prompt
  transport and names copilot is rewritten to assert stdin transport for codex
  and claude (X-1).
- Each usage parser has a synthetic fixture per source and a test that an
  absent field yields ``None``.
- Spawn, capture and cancellation tests use a stub executable on ``PATH``; the
  guard's tests keep running on Linux and macOS.
- ``cargo test``, ``clippy -D warnings`` and ``fmt --check`` pass on Linux,
  macOS and Windows; ``validate.sh`` and every kit's render pass with copilot
  in the retired terms.
- An install into a scratch ``HOME`` with a prefs file holding copilot values
  succeeds and prints one warning line.

Alternatives and costs
----------------------

- Keep the copies and add a third for bench: no extraction risk now, three
  places for every fix later, and usage parsing written twice.
- Make aside depend on dispatch as a library: pulls the SQLite store and the
  run registry into a read-only server.
- Keep copilot as a backend with ``Usage`` all ``None``: keeps a CLI the user
  does not run and whose token reporting no one has checked, and a third argv
  builder to carry through the extraction.

Open questions
--------------

- Where an OpenAI cache write is counted: codex reports
  ``cache_write_input_tokens`` without a duration, and the buckets carry
  5-minute and 1-hour cache writes. Proposal: ``cache_write_5m`` holds every
  cache write whose source does not state a duration, and ``cache_write_1h``
  only a write the source reports as one hour; the price table decides what
  each bucket costs per model.
- Whether ``which()`` finds ``.cmd`` shims on Windows. Today neither server
  finds an npm-installed CLI there; changing that is a new capability, not part
  of the extraction.

References
----------

- ``shared/mcp-servers/aside/src/{backend.rs,errkind.rs,main.rs}``
-
  ``shared/mcp-servers/dispatch/src/{backend.rs,errkind.rs,executor.rs,opencode.rs,pdeath_guard.rs,winjob.rs}``
- ``shared/mcp-servers/harness-log/src/codex.rs``
- codeburn 0.9.25, ``src/models.ts`` and ``src/providers/codex.ts`` (bucket
  rules)
- `design/architecture.rst <../design/architecture.rst>`_
