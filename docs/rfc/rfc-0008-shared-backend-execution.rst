RFC-0008: Shared backend execution layer
========================================

:Status: Accepted
:Implementation: complete — the agent-exec crate, harness-log moved beside it
  with usage parsers, aside and dispatch rebuilt on both, copilot removed from
  servers, installer, prefs and rules
:Verification: runtime — 2026-09-30; cargo test --workspace on Linux, macOS and
  Windows (slate CI run 36726523883), aside and dispatch run end to end on macOS
  against a stub backend, with and without agent-guard, real codex, claude and
  opencode CLIs not run
:Areas: MCP servers; aside; dispatch; installer; rules; prefs
:Authors: Claude Fable 5.1
:Reviewers: Hamin Sung
:Implementers: Claude Fable 5.1 with Claude Sonnet and Claude Opus subagents
  (2026-09-30)
:Accepted: Hamin Sung (2026-09-30T05:45Z)
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
under a guard, captures output under a policy the server chooses, classifies a
failure and retries along a model fallback chain, and returns a ``RunRecord``
whose ``Usage`` holds token counts in one normalized set of buckets.
``harness-log`` moves to ``shared/crates/`` and gains the parsers that read
usage from each backend's own output. aside and dispatch keep their behavior
and differ only in the parameters they pass; dispatch's opencode runner stays
in dispatch and reports through the crate's ``RunEvent``. The copilot backend
leaves aside, the installer, the prefs and the rules. aside adopts dispatch's
parent-death guard.

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

copilot is aside's third backend, the one whose token reporting is unverified
and the one the user no longer runs. Removing it before the extraction leaves
two backends in aside (codex, claude) beside dispatch's three (codex, claude,
opencode).

dispatch runs its backends under a parent-death guard so that a killed dispatch
takes its subtree with it; aside does not, so a backend that aside started
outlives an aside server that was killed.

Evidence
--------

Measured on the ``next`` branch at 9cd43eb, 2026-09-30, unless dated otherwise:

- ``errkind.rs`` is 248 lines in both servers and identical except for two
  comment lines that name the other server.
- ``aside/src/backend.rs`` (550 lines) and ``dispatch/src/backend.rs`` (773
  lines) each define ``Backend``, ``build_command``, ``which``,
  ``install_hint``, ``version`` and a re-entry depth marker; the fallback loop
  is in ``aside/src/main.rs`` (``dispatch`` method) and
  ``dispatch/src/executor.rs`` (``run``), both iterating primary model then
  chain and advancing on ``errkind::classify(...).is_retry_worthy()``.
- The two servers differ in more than argv. Capture: aside keeps the first 50
  KB of stdout and the last 2 KB of stderr, sliced by character, and on failure
  the first 2 KB of stdout; dispatch keeps the first 200 KB of stdout and 16 KB
  of stderr by byte and drains the rest. Failure text: both send stderr to the
  classifier and add stdout only for claude (aside all of what it kept,
  dispatch the last 2,000 characters), because claude prints the discriminating
  error on stdout; a codex or opencode answer that mentions a rate limit must
  not trigger a retry. Fallback history keeps 2,000 characters of each failure.
  aside classifies a spawn failure, dispatch a ``WaitFailed``. dispatch's
  ``version()`` does not stamp the re-entry marker; aside's does. ``which()``
  in both tests ``is_file()``, not executability, and appends only ``.exe`` on
  Windows, where the npm-installed CLIs are ``.cmd`` shims.
- Prompt transport today: aside passes the codex prompt as a positional
  argument and the claude prompt on stdin; dispatch passes both on stdin and
  sends the opencode prompt over HTTP. aside's claude argv carries
  ``--input-format text --output-format text``; dispatch's claude argv carries
  neither.
- dispatch prepares each fallback attempt before it spawns: a fresh prompt
  nonce and a cleared session association (``executor.rs`` ``run_attempt``), a
  fresh pinned claude session id, and a rollout snapshot; its watchdog decides
  exit versus restart from the live child handle.
- dispatch's opencode runner (``opencode.rs``, 924 lines) starts ``opencode
  serve`` on a free port, writes ``mark_running`` right after the spawn and
  ``set_session`` after creating or resuming the session, initializes a log
  file before posting the prompt and stops when that fails, sends ``title`` and
  ``metadata.dispatch_task`` on session creation, and normalizes SSE into JSONL
  events named ``session_meta``, ``user_message``, ``agent_message``,
  ``reasoning``, ``custom_tool_call``, ``custom_tool_call_output``,
  ``patch_apply_end``, ``task_started`` and ``task_complete``, each carrying
  ``native`` and ``partID``. It depends on ``reqwest`` with ``rustls-tls``,
  which pulls ``ring`` (C and assembly); dispatch also depends on ``rusqlite``
  with the bundled C library (``cargo tree``, 2026-09-30).
- ``harness-log`` is 397 lines, all codex session location; no file under
  ``shared/`` parses token usage.
- copilot is named in 17 files under ``shared/``: the aside backend, main,
  params and errkind, the dispatch errkind, the aside prefs template, the
  installer's schema, migration, document parser and Kimi pre-approval list,
  and the installer's tests and fixtures. ``validate.sh`` greps rendered rules
  and root Markdown, not Rust sources or fixtures.
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
  ``cache.write``.
- opencode ``packages/opencode/src/session/session.ts`` at ``dev`` (read
  2026-09-30, ``getUsage``): ``output = outputTokens - reasoningTokens`` and
  ``reasoning`` separately, ``input = inputTokens - cacheRead - cacheWrite``;
  its cost adds reasoning at the output rate. So an opencode ``tokens.output``
  excludes reasoning, unlike codex and claude.
- codeburn 0.9.25 (``src/models.ts``, ``src/providers/codex.ts``, read
  2026-09-30): reasoning is inside output for ``claude`` and ``codex``
  (``REASONING_INCLUDED_IN_OUTPUT``); ``uncached = max(0, input - cached)``;
  ``cache_write = max(0, min(cache_write, uncached))`` and plain input is
  ``uncached - cache_write``, with the tokens moved to the cache-write bucket
  only when the price table states an explicit cache-write rate for the model;
  rollout deltas use ``last_token_usage`` when present, else the change in
  ``total_token_usage``, the cumulative baseline advances on every event, and a
  byte-identical consecutive ``token_count`` event is a re-emission and is
  skipped.
- Deliverable 8 of phase 2 fixes the crate's surface, the ``Usage`` buckets and
  the per-server parameters; deliverable 9 fixes the copilot removal (D-24,
  D-30). X-1 records that deliverable 8's "argv tests unchanged" and "prompts
  on stdin" conflict in one aside test.

Goals and non-goals
-------------------

Goals:

- One implementation of backend lookup, argv construction, spawning, capture,
  failure classification and fallback, used by every server that runs a backend
  process.
- Token usage of every run available to the caller in one normalized form, with
  an unknown count distinguishable from zero and reasoning counted the same way
  for every backend.
- aside and dispatch behave as before: the same flags in the same order, the
  same isolation, the same capture and failure-text policies, the same
  anti-recursion, the same per-attempt preparation in dispatch.
- codex and claude remain aside's backends; codex, claude and opencode remain
  dispatch's.
- A backend started by aside dies with aside, as one started by dispatch dies
  with dispatch.

Non-goals:

- New server behavior: dispatch's steer, unassociated-rollout watchdog, SQLite
  store, prompt rendering and opencode runner, and aside's transcript redaction
  and result rendering stay where they are.
- Cost computation, grids and result files: the bench server's record.
- Replacing copilot with another backend; finding ``.cmd`` shims on Windows.

Requirements and invariants
---------------------------

- No server contains a second implementation of anything the crate exports.
- ``Usage`` holds five disjoint counts of one request, ``input_uncached``,
  ``cache_read``, ``cache_write_5m``, ``cache_write_1h`` and ``output``, and
  one overlapping count, ``reasoning``, which is the part of ``output`` spent
  on reasoning or thinking; it is never added to ``output``. A count the source
  does not report is ``None``, and a bucket derived from a ``None`` operand is
  ``None``; every record names its ``usage_source``.
- The argv each server sends today is preserved as a set of flags and their
  order: for aside, codex ``exec --ignore-user-config`` after ``exec`` and
  claude in safe mode without session persistence with the read-only tool list
  and the explicit text formats; for dispatch, ``-c
  mcp_servers.dispatch.enabled=false`` before ``exec``, the sandbox to
  permission-mode mapping, pinned and resumed sessions, and no format flags.
- The prompt travels on stdin for codex and claude; the one exception to
  today's argv is aside's codex prompt, which leaves the positional argument
  (X-1). opencode receives it over HTTP as today.
- Capture and failure text are policies the server passes, not one winner:
  caps, head or tail, byte or character slicing, which backends contribute
  stdout to the failure text and how much of it; the classifier sees a spawn
  failure and a wait failure as well as a non-zero exit.
- A fallback attempt is built by the server before it is spawned, so dispatch's
  nonce, session id and rollout snapshot per attempt are unchanged.
- The crate depends on no C library; the SQLite store and the opencode HTTP
  client stay in dispatch.
- Both servers run their process backends under the parent-death guard on Linux
  and macOS and the Job Object on Windows, and check for the guard
  re-invocation before anything else in ``main``.
- No file under ``shared/`` names copilot; a prefs file that still holds
  copilot values installs, the values are reported in one warning line and
  treated as blank.

Design
------

The contract is ``spec/agent-exec.rst`` in this record's changeset: the crate's
types (``Backend``, ``Sandbox``, ``Isolation``, ``OutputMode``,
``CapturePolicy``, ``FailureTextPolicy``, ``RunSpec``, ``RunEvent``,
``Outcome``, ``RunRecord``, ``Usage``), its functions (``run``,
``run_with_fallback``, ``errkind``, ``which``, ``version``, ``install_hint``,
``reentry``, ``guard``), the per-server parameters, the usage buckets and their
mapping from each backend's fields, and what ``harness-log`` exports.

The differences between aside and dispatch are values of ``RunSpec``, not
branches: codex anti-recursion is ``Isolation`` (ignore the user config, or
disable one MCP server), claude isolation is the sandbox mapping plus a tool
allowlist and session flags, the format flags are ``OutputMode`` (``Default``
sends none), and capture and failure text are the two policy values.
``run_with_fallback`` takes a function that builds the ``RunSpec`` of each
attempt from its index and model, so a server prepares what it needs before the
spawn.

The opencode runner stays in dispatch: it is an HTTP client, not a process
backend, and its dependencies carry C code the crate must not have. It reports
through the crate's ``RunEvent`` and ``Outcome`` types so dispatch's executor
writes the store for every backend in one place; its wire event names,
``title`` and ``metadata`` are unchanged.

``harness-log`` keeps the session location functions and gains a parser per
usage source that returns ``Usage``: a codex ``--json`` stream, a codex rollout
from a baseline, a claude ``--output-format json`` result and an opencode
message. An opencode ``output`` is ``tokens.output + tokens.reasoning`` so that
reasoning is inside output for every source.

copilot is removed rather than carried: its ``Backend`` variant, argv builder,
tool, prefs settings and schema entries go, and the consultation rule states
the two backends that remain.

Impact and compatibility
------------------------

- aside gains the guard: a killed aside server no longer leaves a backend
  running. Its startup path checks the guard argv first, as dispatch's does.
  aside's codex prompt moves from argv to stdin; dispatch's ``version()`` probe
  stamps the re-entry marker as aside's does.
- The ``aside_copilot`` tool disappears; a call to it fails as an unknown tool.
  The aside prefs lose the three copilot settings; a user file that still has
  them installs with one warning line, and a ``copilot`` backend value is
  treated as blank, so the agent chooses the backend as it does without a prefs
  value.
- ``harness-log`` changes path; the workspace members, ``validate.sh``'s
  required paths, ``release.yml``, ``install-mcp.sh``, ``descriptor.rs``,
  ``binaries.rs`` and the architecture design document follow.
- Server binaries and argv are otherwise unchanged; no MCP tool of dispatch
  changes its parameters or output.
- Every server that runs a backend can now attach token usage to a run; the
  bench server is the first consumer.

Implementation and transition
-----------------------------

copilot leaves first, so that the extraction unifies two backends in aside and
three in dispatch (D-30). The crate is then extracted with aside and dispatch
switched to it in the same change, since a period with three copies would
defeat the purpose. ``harness-log`` moves with the extraction. Both changes
ship through the release train: kit version bumps, changelogs, kit CI, tags.

Verification strategy
---------------------

- The argv tests of aside and dispatch keep asserting the flag set and order
  named in the requirements; the one aside test that asserts the argv prompt
  transport and names copilot is rewritten to assert stdin transport for codex
  and claude (X-1), the single exception to "unchanged".
- Capture and failure-text tests reproduce today's numbers per server: aside 50
  KB head and 2 KB tail by character, dispatch 200 KB and 16 KB by byte, stdout
  in the failure text only for claude with dispatch's 2,000-character tail, and
  a rate-limit phrase in a codex answer that does not trigger a retry.
- A fallback test shows the attempt builder called once per attempt before the
  spawn, and a cancelled outcome ending the chain.
- Each usage parser has a synthetic fixture per source and tests that an absent
  field yields ``None``, that a codex cache write is clamped to the uncached
  input, that an opencode output adds reasoning, and that a rollout parsed from
  a baseline after a resumed run counts only the new turns and skips a
  byte-identical repeated event.
- Spawn, capture and cancellation tests use a stub executable on ``PATH``; the
  guard's tests keep running on Linux and macOS.
- ``cargo test``, ``clippy -D warnings`` and ``fmt --check`` pass on Linux,
  macOS and Windows; ``validate.sh`` and every kit's render pass with copilot
  in the retired terms, and a grep of ``shared/`` for copilot is empty.
- An install into a scratch ``HOME`` with a prefs file holding copilot values
  succeeds and prints one warning line.

Alternatives and costs
----------------------

- Keep the copies and add a third for bench: no extraction risk now, three
  places for every fix later, and usage parsing written twice.
- Make aside depend on dispatch as a library: pulls the SQLite store and the
  run registry into a read-only server.
- Move the opencode runner into the crate: needs an HTTP client without C code
  (``reqwest`` with ``rustls`` pulls ``ring``), for one backend that only
  dispatch runs.
- Keep copilot as a backend with ``Usage`` all ``None``: keeps a CLI the user
  does not run and whose token reporting no one has checked, and a third argv
  builder to carry through the extraction.

Open questions
--------------

- Whether ``which()`` should find ``.cmd`` shims on Windows. Today neither
  server finds an npm-installed CLI there; changing that is a new capability,
  not part of the extraction.

References
----------

- ``shared/mcp-servers/aside/src/{backend.rs,errkind.rs,main.rs}``
- ``shared/mcp-servers/dispatch/src/{backend.rs,errkind.rs,executor.rs}``
- ``shared/mcp-servers/dispatch/src/opencode.rs``
- ``shared/mcp-servers/dispatch/src/{pdeath_guard.rs,winjob.rs}``
- ``shared/mcp-servers/harness-log/src/codex.rs``
- opencode ``packages/opencode/src/session/session.ts`` (``getUsage``)
- codeburn 0.9.25, ``src/models.ts`` and ``src/providers/codex.ts``
- `design/architecture.rst <../design/architecture.rst>`_
