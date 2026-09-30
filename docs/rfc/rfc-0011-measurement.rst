RFC-0011: Measurement — bench server and benchmark suite
========================================================

:Status: Accepted
:Implementation: not-started — the bench MCP server, its rule and prefs
  template, the benchmark fixtures, scorers and analysis of the first wave
:Verification: none — 2026-09-30; not verified yet
:Areas: MCP servers; installer; rules; prefs; benchmarks
:Authors: Claude Fable 5.1
:Reviewers: Hamin Sung
:Implementers: none yet
:Accepted: Hamin Sung (2026-09-30T08:38Z)
:Date: 2026-09-30
:Revised: none
:Depends: RFC-0008 (direct use of ``RunRecord``, ``Usage`` and the usage
  parsers); RFC-0002 (direct use of the server registration and the prefs
  schema, which gain bench)
:Supersedes: none
:Related: RFC-0001 (the levels and models the evidence will inform)
:Changes: spec/bench.rst (created); design/benchmark-suite.rst (created);
  design/architecture.rst (Sources; Interfaces and dependencies);
  spec/support-matrix.rst (Contract)
:Description: A kit-registered bench server runs a model × effort × harness grid
  over agent-exec and records each run's usage and list-price cost, and a
  benchmark suite on saltyos excerpts turns those runs into an error rate per
  model and effort, the evidence task-class routing needs.

Summary
-------

Deciding which model and effort fit which kind of work needs many controlled
runs whose token usage is kept. ``bench`` is a fourth kit-registered MCP
server: it takes a grid of (backend, model, effort) cells, a task set, a repeat
count and an output location, runs every cell through agent-exec in an isolated
backend home, and writes one JSONL record per run with the normalized usage,
the list-price cost from a LiteLLM price snapshot, wall time, exit and the
harness condition. A benchmark suite, outside the server, supplies the tasks:
excerpts of saltyos copied by path and built with plain ``rustc``, each
benchmark yielding one error-rate scalar with a deterministic scorer where the
task has ground truth. The first wave is eight such benchmarks; a document
variant of each supplies saltyos's RFCs and older palette documents so that
document use is measured. Charts and analysis read the JSONL; the server
records.

Problem and context
-------------------

Task-class routing (B-8) asks the kit to say which model and level suit complex
design, daily coding and debugging, and trivial work. Today every run that
could answer that is started by hand and its token usage is thrown away; the
servers keep no usage at all until agent-exec lands.

Public benchmarks do not settle it: they are contaminated or retired (OpenAI
stopped reporting SWE-bench Verified on 2026-02-23), none scores precedent use,
orchestration, creativity or temperament, and none measures the rules this kit
ships. The kit needs its own tasks, on code the models have not seen, with the
kit's articles as part of what is scored.

A measurement is only comparable when every cell runs the same tasks under the
same conditions: the same kit version rendered for the harness, the same
isolation, the same scorer, and a design that clusters on task rather than
treating repeats as independent.

Evidence
--------

- Deliverables 10 and 11 of phase 2 fix the record fields, the isolation, the
  output locations, the first-wave catalog and the analysis rules; decisions
  D-23, D-25, D-27, D-28, D-29 and D-31 of this repository's state (2026-09-30)
  fix that bench is a kit-registered server, where results go, how excerpts are
  taken, the document condition, the cost rule and the license of ``lib/``
  excerpts.
- Verified 2026-09-30: ``codex exec --json -o <file>`` runs with stdin closed
  and ``--skip-git-repo-check`` outside a trusted directory; ``claude -p
  --output-format json --effort <level>`` works with
  ``--no-session-persistence`` and reports ``total_cost_usd`` and
  ``duration_api_ms``; claude's ``modelUsage.<model>.costBasis`` is ``list`` on
  a subscription login, so a run's own cost figure is not the list price bench
  needs.
- codeburn 0.9.25's price snapshot: ``claude-fable-5.1`` at 10/50/12.5/0.25 USD
  per million tokens (input, output, cache write, cache read); ``gpt-6-*``
  entries need the live LiteLLM fetch (the snapshot stops at ``gpt-5.6-luna``
  and ``gpt-5.6-sol``). Anthropic's 1-hour cache write is priced at 1.6 × the
  5-minute write (2 × input against 1.25 ×). codeburn moves OpenAI cache-write
  tokens to the cache-write bucket only when the price table states an explicit
  rate (``src/providers/codex.ts``, read 2026-09-30).
- The bucket rules and the reasoning-inside-output rule are those of
  ``spec/agent-exec.rst``; bench adds no arithmetic of its own to the counts.
- saltyos material, counted 2026-09-30 (paths under its working tree):
  ``userland/servers/fluxd-networkd/src/net/{socket/sequence.rs,
  socket/tcp/connection.rs, proto/tcp.rs, proto/ipv4.rs, checksum.rs,
  types.rs}`` with ``tests/tcp_sequence_host.rs`` (1,726 + 687 lines, no
  outside dependency, 21 tests); ``kernite/src/cap/cnode.rs`` with two
  leader-verified defects (``current-state.rst`` items 61 and 62);
  ``kernite/src/ipc/event_pair/state.rs`` (84 lines, 6 tests);
  ``lib/server/src/{executor.rs, pending.rs}`` with ``tests/host.rs`` (needs
  ``lib/system/kernel/src/core_types.rs``); further candidates in the
  scheduler, log ring, supervisor, time, identity, event, random and memory-map
  modules. Build facts: no Cargo; ``flake.toml`` calls ``rustc --edition=2024
  --test <root>.rs --extern uapi=libuapi.rlib``; ``uapi.rs`` is generated from
  ``kernite/include/uapi/*.h``; kernel host tests use ``#[path]`` from
  ``kernite/tests/host/`` with ``RUST_MIN_STACK=33554432 --test-threads=1``;
  the ``none`` targets need ``kernite/{x86_64,aarch64}-kernite.json``.
- Licenses: saltyos is GPL-2.0-only at the root; ``lib/basalt``, ``lib/c``,
  ``lib/nt``, ``lib/system`` and ``lib/win32`` carry an LGPL-2.1-or-later
  ``LICENSE.md``; ``lib/server`` has none and GPL-2.0-only headers (checked
  2026-09-30). D-26 and D-31: an excerpt takes its component's license,
  ``lib/`` excerpts LGPL-2.1-or-later, corrected in the extracted copies only.
- Measurement sources: arXiv 2605.18583 (OverEager-Bench), 2605.07769,
  2603.24755, 2602.03712, 2607.22585, 2603.23749, 2608.16956, 2411.00640
  (pass^k and the paired design).

Goals and non-goals
-------------------

Goals:

- One tool call runs a whole grid and leaves a JSONL any script can plot; a
  grid that stops resumes without repeating a run.
- No run touches the user's own harness session stores or configuration.
- Cost is reproducible from the record alone: the counts, the price snapshot
  date and the rule.
- Every first-wave benchmark has a deterministic scorer and one scalar where 0
  is best.
- Both harnesses run the same pinned kit version; a no-kit condition and a
  document condition are recorded per cell.

Non-goals:

- Charts, judging and analysis inside the server; a general-purpose evaluation
  harness.
- Wave-two benchmarks (synthesis, creativity, long-horizon, orchestration,
  calibration and the rest of the catalog).
- Fixing licenses or non-compiling code in the saltyos tree; committing
  fixtures or results to a public path.

Requirements and invariants
---------------------------

- A run is one ``RunSpec`` of agent-exec; bench adds the cell, the task, the
  repeat, the isolation directories and the price computation, and changes
  nothing in how a backend is invoked.
- Each run has its own ``CODEX_HOME`` and claude configuration directory under
  the output location; the kit under test is installed there at the pinned
  version, rendered for the harness, or absent for the no-kit condition.
- A record carries: cell (backend, model, effort), harness and kit version or
  ``none``, task name and variant (plain or with documents), repeat index, CLI
  versions, argv, started time, wall time, exit code, final text path, the
  ``Usage`` buckets and ``usage_source``, ``reasoning`` tokens, cost in USD
  with the price-snapshot date, and the run directory.
- Cost is tokens × list price per bucket: input, cache read, cache write
  (5-minute and 1-hour rates where the table states them; an OpenAI write
  without an explicit rate is priced as input), output; reasoning is inside
  output and never added. A model missing from the fetched table and the
  vendored snapshot yields cost ``None`` and a warning, never zero.
- The output location is what the request names (D-25): the scratchpad, the
  harness's own configuration store, or a project path; results are one JSONL
  plus one directory per run holding stdout, stderr and the final message.
- A grid is cancellable per cell and as a whole; a grid resumes by skipping
  records already present for (cell, task, variant, repeat).
- Hidden tests never enter a run's working directory before the run ends; the
  scorer copies them in afterwards.
- Every cell runs the same task set; the analysis clusters on task, reports
  pass^k beside mean pass@1, keeps tasks whose pilot pass rate is between 30
  and 70 %, and pre-registers its estimator before the first wave runs.
- A judge, where a benchmark needs one, is a model family not under test or an
  average across families, order-swapped, blinded to model identity and diff
  length, and reported with its agreement against a human-labelled sample.

Design
------

The server contract is ``spec/bench.rst`` in this record's changeset: the tools
(``bench_grid``, ``bench_status``, ``bench_cancel``, ``bench_list``), the grid
request, the isolation, the record schema, the price table and the cost rule,
the output layout, cancellation and resumption. The suite is
``design/benchmark-suite.rst``: the fixture extraction, the first-wave
benchmarks and their scalars, the document condition, the paired design and the
scoring and judge policy.

bench is a fourth binary in the workspace beside aside, dispatch and palette:
listed in ``KNOWN_SERVERS``, rendered and registered by the installer, built by
``release.yml``, with a rule file and a prefs template in each kit. It links
agent-exec and harness-log and adds a price module and a grid runner; the
SQLite store stays dispatch's, bench's state is the JSONL itself.

The suite lives outside the server as fixtures, manifests and scorers; the
server runs whatever task set it is pointed at. A task is a directory with a
prompt, a working tree assembled from the manifest, optional documents for the
document variant, and a scorer command that prints the scalar.

Impact and compatibility
------------------------

- The installer registers a fourth server in every harness; the release ships a
  fourth binary per platform; the kits gain a bench rule and prefs template
  (the level and the default output location).
- aside and dispatch are unchanged; bench is their first sibling built on
  agent-exec.
- The support matrix gains a bench entry per harness.
- Runs cost real quota: a first wave of eight benchmarks × about 30 tasks × 3
  repeats per cell; the grid request states the cells, and the rule keeps bench
  at ``on-request`` unless the prefs say otherwise.

Implementation and transition
-----------------------------

The server is written after agent-exec lands (B-9), since it is the first
consumer of ``RunRecord`` and the parsers. Fixtures are extracted next (B-12),
with the license headers corrected in the copies; the pilot run fixes the task
set by pass rate; the first wave then runs on every model and effort under
test, and its result goes into B-8 as evidence. The server ships through the
release train; fixtures and results stay outside any public path.

Verification strategy
---------------------

- Synthetic-fixture tests cover the record schema, the cost computation (a
  1-hour Anthropic cache write, an OpenAI cached input, a model missing from
  the table) and resumption after a stopped grid.
- A dry run of one cell against a stub backend on ``PATH`` leaves the user's
  ``~/.codex`` and ``~/.claude`` untouched (checked by snapshot before and
  after).
- Each first-wave scorer is run on a known-good and a known-bad solution and
  returns 0 and 1.
- One real grid of one cell produces a JSONL that a chart script plots as error
  rate against reasoning tokens with per-cell whiskers.
- ``cargo test``, ``clippy -D warnings``, ``fmt --check`` and ``validate.sh``
  pass on Linux, macOS and Windows; ``release.yml`` produces the bench artifact
  on every platform.

Alternatives and costs
----------------------

- A local script instead of a kit server: not reproducible across machines and
  harnesses, and the kit could not ship it with its rules (D-23 rejected it).
- Public benchmarks: contaminated or retired, and blind to the kit's own rules.
- Cost from each CLI's own figure: claude reports list cost only on some logins
  and codex reports none; one price table applied to normalized counts is
  comparable across backends.
- Fixtures from a public codebase: the models have seen it.

Open questions
--------------

- Whether the grid runner runs cells concurrently: the backends' rate limits
  argue for one run at a time per backend, and wall time is a measured
  quantity; the draft runs cells sequentially and repeats within a cell
  sequentially.

References
----------

- `RFC-0008 <rfc-0008-shared-backend-execution.rst>`_
- ``_palette/phase-2/deliverables/`` deliverables 10 and 11 (the approved scope)
- LiteLLM ``model_prices_and_context_window.json``
- codeburn 0.9.25, ``src/models.ts`` and ``src/providers/codex.ts``
- arXiv 2605.18583, 2605.07769, 2603.24755, 2602.03712, 2607.22585,
  2603.23749, 2608.16956, 2411.00640
