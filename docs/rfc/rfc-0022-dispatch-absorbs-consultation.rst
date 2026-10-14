RFC-0022: dispatch absorbs consultation; every run is asynchronous and waited on through the harness
====================================================================================================

:Status: Accepted
:Implementation: complete — dispatch consult kind and wait, aside crate removed,
  agent-exec ServerEnv, installer retirement and prefs rename, rules, inserts,
  specs, CHANGELOGs
:Verification: runtime — 2026-10-13; 2026-10-14, workspace tests, clippy and fmt
  on rustc 1.99.0, live codex consultations waited on through Claude background
  Bash and, in codex exec, through exec_command and one empty write_stdin poll
  that returned on exit (a 100 s run, poll of 86.9 s), a dispatched codex
  called dispatch_consult and was refused dispatch_submit, scratch-HOME upgrade
  retired aside in all three harnesses, the Kimi live wait was not run (account
  balance)
:Areas: MCP servers; dispatch; aside; installer; prefs; rules; specs
:Authors: Hamin Sung; Claude (Fable 5.1)
:Reviewers: codex gpt-6-astra (aside, 2026-10-14)
:Implementers: Claude (Fable 5.1) with two Opus 5.5 delegates
:Accepted: Hamin Sung (2026-10-13T15:50Z)
:Date: 2026-10-13
:Revised: 2026-10-13 — Changes names the renamed rule and insert files
  (consult.md, consult-native-advisor.md) now that the sources were moved;
  2026-10-13 — Changes adds design/architecture.rst Purpose and scope, which
  names the servers; 2026-10-13 — Changes names every installer spec section
  that mentions the aside server (Scope and authority, Components, Install
  steps, Binaries, Uninstall) and drops Native configuration, which does not;
  2026-10-13 — the ServerEnv override also sets the dispatch server's command,
  because codex 0.162.1 refuses an entry that has an environment and no command
  when the user's config.toml does not register dispatch (probe, 2026-10-14);
  2026-10-13 — Changes adds the installer spec's Ownership and ordering (the
  step order gains retired servers and prefs renames) and Conformance;
  2026-10-13 — the ServerEnv override also pre-approves dispatch_consult and the
  read tools on the codex command line, because a backend run with -a never
  cannot call an unapproved tool (live run, 2026-10-14); 2026-10-13 — Changes
  names the agent-exec sections whose per-server wording became per-kind
  (OutputMode, CapturePolicy, FailureTextPolicy, Guard) and the glossary's
  rule-file list; 2026-10-14 — Codex waiting means corrected from
  the installed CLI's source: exec_command yields at 30 s with the process
  running, an empty write_stdin poll waits up to background_terminal_max_timeout
  (300 s by default) and returns on exit, the insert, support matrix and
  verification name that instead of a 25 s wait loop
:Depends: RFC-0008 (Backend, RunSpec, Isolation, CapturePolicy and the fallback
  loop of agent-exec); RFC-0002 (direct use of the kit descriptor, the prefs
  files and the server registration the installer owns); RFC-0001 (direct use of
  the consultation and dispatch levels the prefs set)
:Supersedes: RFC-0008 (in part: aside as a separate server with its own capture
  policy, failure text and re-entry marker)
:Related: RFC-0014 (the guard, which dispatch requires and aside preferred)
:Changes: spec/dispatch-server.rst (created); spec/agent-exec.rst (Scope and
  authority; Isolation; OutputMode; CapturePolicy; FailureTextPolicy; Re-entry;
  Guard); spec/installer.rst (Scope and authority; Components; Descriptor;
  Install steps; Binaries; Payload files and signatures; Server registration;
  Uninstall; Legacy cleanup; Ownership and ordering; Conformance);
  spec/prefs.rst (Scope and authority; Settings; Migration);
  spec/support-matrix.rst (Contract); design/architecture.rst (Purpose and
  scope; Sources; Execution and concurrency; Interfaces and dependencies);
  glossary.rst (Direction and authority; Actors); shared/rules/constitution.md
  (§ 15 (3): "consults aside" becomes "consults another model family"; the file
  map names the consult rule file); shared/rules/implementing/mcp/consult.md
  (was aside.md: the consultation rule names dispatch_consult and the prefs file
  consult-prefs); shared/rules/implementing/mcp/dispatch.md (the waiting bullet:
  a run is waited on with the harness's background means running dispatch wait;
  the status-polling and follow-up sentences go);
  adapters/claude/inserts/dispatch-notify.md (names background Bash as the means
  and drops ScheduleWakeup; the codex and kimi inserts gain their means);
  adapters/claude/inserts/consult-native-advisor.md (was
  aside-native-advisor.md, marker renamed; the text says the consultation runs
  first); adapters/codex/inserts/delegation-surfaces.md (names consultation
  through dispatch_consult instead of aside); adapters/codex/surface.md (the
  servers list and the consulting sentence name dispatch and palette; the aside
  server entry goes); adapters/kimi/surface.md (the plugin server list and the
  tool-prefix example name dispatch and dispatch_consult; the aside entry goes)
:Description: The aside server is removed; dispatch gains a read-only
  consultation kind with aside's transcript, framing and isolation, returns an
  id for every run, and ships a wait subcommand that each harness blocks on
  through its own background means.

Summary
-------

The aside MCP server is removed. dispatch gains a second run kind, ``consult``:
a read-only question to codex or claude that carries the redacted harness
transcript, aside's role framing and independence reminder, and aside's
isolation. ``dispatch_consult`` returns an id at once as ``dispatch_submit``
does; ``dispatch_status`` returns the reply. The dispatch binary gains
``dispatch wait <id>``, which another process runs to block until the run is
terminal, and the rules say that a run is waited on with the harness's own
background means running that command instead of polling or a scheduled
wake-up. Re-entry is decided per kind, so a dispatched backend may still
consult. The installer removes aside's registration and binary on upgrade,
renames the aside prefs file to the consultation prefs file keeping its values,
and pre-approves the consultation tool where the harness grants approval per
tool.

Problem and context
-------------------

A consultation blocks the harness's tool call for the whole backend run, which
at high reasoning effort is minutes. The agent cannot do anything meanwhile,
Codex needs a 1,800 s tool timeout and direct-call namespaces written into its
configuration to survive it, and nothing records the run: a killed consultation
leaves no status to read. dispatch solved the same problems for execution with
a run store, detached execution, cancellation, boot and read-time
reconciliation of stranded rows, and a status tool; B-3 was approved as giving
aside the same shape. Doing that in aside would copy that lifecycle into a
second server (store, state directory, registry, liveness, reconciliation,
wait), and the two copies would drift. The two servers already share everything
below the lifecycle: ``agent-exec`` runs both backends from a ``RunSpec`` and
the servers differ in the values they pass (RFC-0008). What remains distinct in
aside is the transcript forwarding, the prompt framing and the read-only
isolation, which are properties of a run, not of a server.

Waiting is the other half. ``dispatch_status`` does not block, no notification
arrives, and the Claude rule's answer was ``ScheduleWakeup``, which wakes the
session on a timer whether or not the run has finished. Each harness has a
background means that blocks on a process or re-invokes the agent when the
process exits; a CLI command that blocks on the run is what those means can
run.

Evidence
--------

- aside today (``shared/mcp-servers/aside/src/main.rs``, 2026-10-14): one tool
  per backend, the call blocks in ``dispatch`` until ``run_with_fallback``
  returns; a progress ticker every 15 s keeps the client's timeout alive; the
  transcript is rendered before the spawn (``transcript/``: 80-message tail,
  100 KB cap, tool calls and results redacted); ``spec.rs`` runs codex
  ``exec --ignore-user-config`` under ``-s read-only`` and claude in safe mode
  with ``--permission-mode plan`` and the tools Read, Grep, Glob, WebFetch;
  capture keeps 50 KB of stdout by character; ``main()`` goes straight to
  ``serve`` with no store.
- dispatch today (``shared/mcp-servers/dispatch/src/``): SQLite store
  ``dispatch.db`` under ``resolve_state_dir`` (``DISPATCH_STATE_DIR``, else
  ``SLATE_AGENT_STATE_HOME``/``AGENT_KIT_STATE_HOME``, else
  ``~/.slate-agent-kit``, then ``projects/<slug>/dispatch``), idempotent
  ``ALTER TABLE`` upgrades, statuses queued, running, succeeded, failed,
  cancelled, interrupted, ``reconcile`` at boot and ``graceful_shutdown`` on
  signal, ``liveness::process_alive`` by pid. Codex execution backends run with
  ``Isolation::DisableMcpServer("dispatch")`` because codex starts MCP servers
  with a clean environment, so the ``DISPATCH_REENTRY_DEPTH`` marker never
  reaches a codex-booted dispatch (``backend.rs`` header); a dispatch backend
  calling aside is allowed today.
- codex 0.162.1 probe (2026-10-14):
  ``codex exec -s read-only -c mcp_servers.probe.command="<script>" -c mcp_servers.probe.env.DISPATCH_REENTRY_DEPTH="1" "Reply ok."``
  started the script with ``DISPATCH_REENTRY_DEPTH=1`` in its environment, so a
  ``-c`` override reaches a stdio MCP server's environment table. The merge of
  a ``-c`` key into a server table that ``config.toml`` defines is what the
  existing ``-c mcp_servers.dispatch.enabled=false`` relies on; the end-to-end
  check against the installed dispatch registration is in the verification
  strategy.
- The installer registers dispatch with ``SLATE_AGENT_STATE_HOME`` set to the
  Claude home or ``<home>/slate-agent-kit`` (``spec/installer.rst`` Server
  registration), so a command run from the harness's shell does not see the
  server's state home; ``serve.rs`` registers and ``uninstall.rs`` unregisters
  from the manifest, and no step unregisters a server that a new kit no longer
  lists. Codex gets ``tool_timeout_sec = 1800``,
  ``default_tools_approval_mode = "approve"`` and code-mode namespace entries
  for aside (``harness/codex.rs``). Kimi's registry text lists expected tool
  prefixes and grants nothing (``harness/kimi.rs``). Claude gets
  ``permissions.allow`` entries for the palette read tools
  (``harness/claude.rs``).
- Codex MCP configuration (learn.chatgpt.com/docs/extend/mcp, read 2026-10-14):
  ``default_tools_approval_mode`` takes ``auto``, ``prompt``, ``writes``,
  ``approve``; ``tools.<tool>.approval_mode`` overrides it per tool;
  ``tool_timeout_sec`` defaults to 60.
- Waiting means, each from its primary source: Claude Code's Bash tool contract
  (this session's tool description, 2026-10-14): ``run_in_background`` runs the
  command detached, it keeps running across turns and re-invokes the agent when
  it exits; a foreground call times out at 600 s at most. Kimi Code 2.1.1
  (``dist/main.mjs`` of the installed package): a background Shell call returns
  a task id, the agent is notified automatically when the task completes,
  ``TaskOutput`` is a non-blocking snapshot, foreground timeout at most 300 s,
  background default 600 s and ``disable_timeout=true`` for no timeout. Codex
  (``codex-rs/core/src/unified_exec/mod.rs`` and ``process_manager.rs``,
  ``core/src/tools/handlers/shell_spec.rs``, ``config/src/config_toml.rs`` at
  ``rust-v0.162.1``, the installed CLI, read 2026-10-14): ``exec_command`` and
  a non-empty ``write_stdin`` return after ``yield_time_ms`` clamped to 250–30
  000 ms (``MAX_YIELD_TIME_MS``) with the process kept running as a background
  terminal; an empty ``write_stdin`` is a poll whose window is clamped to 5 000
  ms–``background_terminal_max_timeout`` (300 000 ms by default, a
  ``config.toml`` key) and returns when the process exits; the ``sleep`` tool
  ends early only on new user input and the exit watcher emits a UI event, so
  no call brings the agent back on exit. Measured with ``codex exec`` 0.162.1
  on 2026-10-14: a poll of 240 000 ms on an 80 s process returned after 67.2 s
  with exit code 0, a poll of 120 000 ms on a 45 s process after 30.4 s with
  exit code 0.
- RFC-0008 (Alternatives) rejected making aside depend on dispatch as a library
  because it would pull the SQLite store and the TLS stack into aside; this RFC
  removes aside instead.

Goals and non-goals
-------------------

Goals: one server for consultation and execution; consultation keeps every
property aside had (transcript, framing, isolation, capture, fallback note,
transcript warning); every run returns an id at once and is read through
``dispatch_status``; a process other than the server can block on a run; the
rules name each harness's means from its primary source; the installer moves a
live home from aside to the merged server without losing the user's
consultation preferences.

Non-goals: no change to execution semantics (sandbox ceiling, working-directory
containment, one run per directory, the unassociated-log restart, steer, the
opencode runner); no MCP wait tool; no change to the harness-native advisor
rule beyond the server name; no fourth kit and no agy backend (B-4).

Requirements and invariants
---------------------------

- A consultation run is read-only: codex under ``-s read-only`` with
  ``exec --ignore-user-config``, claude in safe mode with
  ``--permission-mode plan`` and only Read, Grep, Glob and WebFetch; the
  opencode backend is refused for it. It takes no working-directory slot: the
  per-directory guard counts execution rows only, in both directions.
- The prompt of a consultation is aside's: role framing, optional context, the
  redacted transcript when available, the question, the independence reminder
  last; the transcript is rendered at submit time from the harness's own log,
  with the same tail and cap. A missing transcript is a note in the result,
  never a failure. The harness and the transcript-directory override that aside
  read from ``ASIDE_HARNESS`` and ``ASIDE_TRANSCRIPT_DIR`` are read from
  ``SLATE_HARNESS`` and ``DISPATCH_TRANSCRIPT_DIR``, and the installer sets
  ``SLATE_HARNESS`` on the dispatch registration.
- ``dispatch_status`` of a consultation returns, in ``result``, the text aside
  returned: the ``[backend]`` header, the reply, the truncation note, the
  fallback note and the transcript note; a failed run carries aside's failure
  text in ``error``. Capture limits are aside's for consultation and dispatch's
  for execution. The guard stays ``Preferred`` for a consultation and
  ``Required`` for an execution, as RFC-0014 set them.
- A run's terminal status, result or error, final model and fallback history
  are written in one transaction, so a reader that sees a terminal status sees
  the whole outcome.
- Re-entry is per kind. Inside a dispatch-spawned backend, ``dispatch_consult``
  is accepted and ``dispatch_submit`` and ``dispatch_steer`` are refused. A
  consultation backend sees no MCP server at all, so nothing it starts can
  reach dispatch. A codex execution backend keeps the dispatch server enabled
  and receives, on its command line, the dispatch entry's command (this
  binary), the marker ``DISPATCH_REENTRY_DEPTH="<depth>"`` in that entry's
  environment (a string; codex rejects an integer) and
  ``approval_mode = "approve"`` for ``dispatch_consult`` and the read tools,
  because the backend runs with ``-a never`` and could not call an unapproved
  tool; this replaces ``DisableMcpServer`` for that backend. The end-to-end
  check against the installed registration is part of verification, and if it
  fails the backend keeps ``DisableMcpServer`` and the support matrix states
  that a codex execution backend cannot consult.
- Every run row has a ``kind`` (``execute`` or ``consult``); an existing
  ``dispatch.db`` opens and reads as ``execute`` rows; the contracts of the
  seven existing tools do not change, except that ``dispatch_list`` accepts a
  ``kind`` filter, ``dispatch_status`` shows the kind, and ``dispatch_logs`` of
  a consultation shows the backend's own session log when one is located and
  otherwise says that a consultation has no live log.
- ``dispatch wait <id>`` blocks until the run is terminal or
  ``--timeout <seconds>`` elapses; exit code 0 when terminal (the line names
  the status and, for an interrupted row that was restarted, its successor), 2
  when still running, 3 for an unknown id, 1 for a usage or store error; it
  prints one line (id, kind, status, final model when a fallback answered) and
  never the result body. When the row is active and its recorded owner pid is
  set and dead, ``wait`` marks the row ``interrupted`` through the same
  conditional update the server's reconciliation uses and reads the row again
  before reporting; a row whose owner pid is unset is waited on. Liveness is by
  pid, as the server's own reconciliation; a reused pid is a known limit,
  bounded by ``--timeout``. ``wait`` opens the existing database only, creates
  nothing, reconciles no other row, probes no backend and cancels nothing. The
  submit and consult responses carry ``wait``: the exact command line with the
  binary's absolute path and ``--db``.
- The common rule says a run is waited on by running that command with the
  harness's background means; a wait is repeated only on exit code 2, exit
  codes 1 and 3 are reported; execution is done only at ``succeeded`` with its
  result reviewed, consultation at ``succeeded`` with its reply read, and an
  ``interrupted`` row with a successor is followed to the successor. Each
  harness's insert names its means from its primary source; no insert names
  another harness's tool.
- On upgrade the installer unregisters a server the previous manifest recorded
  and the new kit no longer lists, removes its binary unless another kit's
  manifest still lists it, restores only the configuration keys recorded for
  that server, and rewrites Kimi's plugin registration to the remaining
  servers; it renames ``<kit>--aside-prefs.md`` to ``<kit>--consult-prefs.md``
  before the prefs states are read, keeping its values and free text and
  rewriting only the signature line, and when both files exist it keeps the
  consult file, leaves the aside file untouched and reports it; it pre-approves
  ``dispatch_consult`` and the dispatch read tools where the harness grants
  approval per tool (Claude ``permissions.allow``; Codex
  ``tools.<tool>.approval_mode``); Kimi's registry text lists the new prefixes.
- No rule, insert, spec, glossary entry, workflow, render or validate source of
  the repository names aside after the change; the installer's migration code
  and fixtures, the records and the CHANGELOG history keep the name.

Design
------

Kinds. The store gains ``kind TEXT NOT NULL DEFAULT 'execute'`` through the
existing idempotent upgrade; a consult row stores the server's directory as
``working_dir``, ``sandbox = read-only``, the rendered prompt, and
``spec_json`` holding the question, context, transcript settings and the
transcript note. ``active_for_dir`` counts rows of kind ``execute`` only. The
executor runs both kinds; the ``RunSpec`` differs by kind (``consult``: aside's
``spec::attempt`` values, ``CapturePolicy::aside``,
``FailureTextPolicy::aside``, ``GuardMode::Preferred``; ``execute``: today's
``run_spec``). The consult kind skips the working-directory guard, the rollout
watchdog and the unassociated restart; it keeps the fallback chain and the
fallback history. ``finish`` writes the terminal status, result or error, final
model and fallback history in one transaction.

Tools.
``dispatch_consult(backend?, question, context?, include_transcript?, transcript_tail?, model?, reasoning_effort?, model_fallback?, title?)``:
``backend`` defaults to ``codex`` and accepts ``codex`` or ``claude``; the
response is the submit response with ``kind``, the transcript note, and
``wait`` (the command line). ``dispatch_submit`` returns the same ``wait``
field. ``dispatch_status`` adds ``kind``; ``dispatch_list`` adds the ``kind``
filter; ``dispatch_cancel`` applies to both kinds and is the only way to stop a
consultation once submitted (aside's cancellation through the request token and
its progress ticker go with the blocking call); ``dispatch_logs`` of a
consultation reads the backend's own session log when one is located, else
reports that a consultation has no live log. ``dispatch_steer`` refuses a
consult (``invalid_params``). ``dispatch_backends`` lists codex, claude and
opencode with which kinds each serves. The transcript readers move into
dispatch unchanged except for their environment names (``SLATE_HARNESS``,
``DISPATCH_TRANSCRIPT_DIR``).

Re-entry. The marker stays ``DISPATCH_REENTRY_DEPTH``; ``reentry_refusal``
takes the kind: at depth 1 or more it refuses ``execute`` and ``steer`` and
accepts ``consult``. For a codex execution backend, ``Isolation`` gains
``ServerEnv { server, command, var, value }``, rendered as
``-c mcp_servers.<server>.command="<command>"`` and
``-c mcp_servers.<server>.env.<var>="<value>"`` before ``exec``, which replaces
``DisableMcpServer("dispatch")`` for that backend; the command is this dispatch
binary's absolute path, so the entry is complete when the user's codex
configuration does not register dispatch (codex refuses an entry with an
environment and no command), and merges into the registered entry when it does.
The agent-exec spec records the variant beside ``DisableMcpServer``. Consult
backends keep ``IgnoreUserConfig`` and safe mode, so no marker is needed there.
The ``ASIDE_REENTRY_DEPTH`` marker is retired.

Wait. ``dispatch`` with the first argument ``wait`` runs the CLI before any
server initialization: no tracing setup beyond stderr, no state-directory
creation, no reconciliation, no backend probe.
``dispatch wait <id> [--timeout <seconds>] [--db <path>]``: the store path
comes from ``--db``, else from the same resolution the server uses; the submit
and consult responses print ``wait`` with the binary's absolute path and
``--db`` filled in, each quoted when it contains whitespace or a shell-special
character, because the harness's shell carries neither the server's
``SLATE_AGENT_STATE_HOME`` nor its binary folder on ``PATH``. The loop reads
the row every 500 ms outside any transaction; when the row is active and its
owner pid is set and dead it runs ``mark_interrupted`` and reads the row again;
it exits 0 on a terminal row, 2 at the timeout, 3 when the id is unknown, 1 on
a usage or store error. Output is one line on stdout; errors go to stderr.

Rules. The dispatch rule's waiting bullet becomes: a run is waited on by
running the returned ``wait`` command with the harness's background means,
which hands the turn back meanwhile and brings the agent back when the command
exits; a wait that exits 2 is repeated, 1 or 3 is reported; done only when
``dispatch_status`` shows ``succeeded`` and the result or reply is reviewed; an
``interrupted`` row with a successor is followed. The Claude insert: Bash with
``run_in_background``, which re-invokes the agent when the command exits,
without ``--timeout``. The Kimi insert: Shell with ``run_in_background`` and
``disable_timeout``, which notifies the agent on completion; ``TaskOutput``
only for a snapshot. The Codex insert: ``exec_command`` starts the wait line
without ``--timeout`` and yields at 30 s with it still running; empty
``write_stdin`` polls with ``yield_time_ms`` 300000 follow until the process
exits, each returning the moment it does; the user's messages are read after
the running poll returns. The consultation rule keeps its content and names
``dispatch_consult``, the consult prefs and the per-kind re-entry;
``validate.sh`` adds ``TaskOutput`` and ``disable_timeout`` to the leak terms
outside the kimi render and ``write_stdin`` and ``yield_time_ms`` outside the
codex render, keeps ``ScheduleWakeup`` outside claude (where it no longer
appears either), and its required paths, prefs list, insert checks and render
mappings drop aside, as does ``release.yml``.

Prefs and installer. The prefs template ``consult-prefs.md.tmpl`` carries
aside's settings under the signature ``<kit>-custom:consult-prefs``; keys are
``consult.level``, ``consult.backend``, ``consult.<backend>.model``,
``.effort``, ``.fallback``. The kit's prefs discovery reads an existing aside
file under the consult name (aside file present, consult file absent), so the
wizard sees the user's values and plans no defaults; the file itself is moved
at apply, after the confirmation, with only its signature line rewritten; when
both exist both stay and the summary says the consult file is used.
``KNOWN_SERVERS`` becomes dispatch and palette, ``KNOWN_PREFS`` replaces aside
with consult, and the plan gains a retirement step: for each server the
previous manifest recorded (for Kimi, each server the existing plugin manifest
lists) and the kit no longer lists, restore the configuration keys recorded for
that server only (Codex's timeout, approval mode and namespaces), unregister it
in the harness (``claude mcp remove``, ``codex mcp remove``, the Kimi plugin
registration rewritten to the remaining servers), remove its binary unless
another kit's manifest lists it, and update the manifest only for what
succeeded; a removal the harness CLI ran and refused is a note with the command
to run by hand, and the registration is dropped. Codex gets
``[mcp_servers.dispatch.tools.dispatch_consult] approval_mode = "approve"`` and
the same for the read tools status, list, logs and backends; Claude gets the
matching ``permissions.allow`` entries. The dispatch registration gains
``SLATE_HARNESS=<harness>``. The kit descriptor lists
``servers = ["dispatch", "palette"]``.

Documents. ``spec/dispatch-server.rst`` is created with the tool contracts, the
kinds, the store, re-entry, wait and the error codes; the agent-exec,
installer, prefs and support-matrix specs, the design and the glossary lose
aside and gain the consult kind, the isolation variant and a Waiting capability
row.

Impact and compatibility
------------------------

A consultation no longer blocks the harness; the agent submits, hands the turn
back while the background wait runs, and reads the reply from
``dispatch_status``. Stopping a consultation after submission is
``dispatch_cancel``; aside's cancellation through the tool call and its
progress notifications go with the blocking call. Codex loses the 1,800 s tool
timeout for consultation because no call blocks; dispatch's own timeout stays.
An agent that calls ``aside_codex`` after the upgrade gets an unknown-tool
error from the harness; the rules it loads no longer name it. dispatch's
execution contracts are unchanged and its existing database opens as before.
The dispatch binary grows by the transcript readers (it already depends on
``harness-log``); aside's archive disappears from the release and the kit's
download list. The design statement that dispatch alone carries SQLite stays
true. Each kit's standing corpus changes by the consultation rule's renames and
the three waiting inserts, within the ceilings of ADR-0001.

Implementation and transition
-----------------------------

Order: the consult kind and the ``wait`` subcommand in dispatch, with the aside
modules moved in and the aside crate deleted in the same change so the
workspace never carries two copies of the transcript readers; then agent-exec's
isolation variant with its codex check; then the installer (descriptor, prefs
rename, server removal, Codex and Claude approvals); then rules, inserts, prefs
template, render and validate; then the specs, design, glossary and CHANGELOGs
through the changeset. The kit release that carries this change is the one that
removes aside from live homes; until it is installed, the live session keeps
the old servers.

Verification strategy
---------------------

- Unit and integration tests in dispatch: a consult row's RunSpec equals
  aside's argv and flags for codex and claude; the prompt order; the rendered
  result and failure texts; ``kind`` default on an old database;
  ``dispatch_list`` by kind; re-entry per kind; ``wait`` exit codes on a
  terminal, a running, an unknown and an orphaned row, with the orphan written
  ``interrupted``.
- agent-exec: the argv of the new isolation variant; the spec's Isolation
  section states it.
- Installer tests: a manifest with an aside registration and binary is
  unregistered, removed and its Codex keys restored; an aside prefs file is
  renamed with its values and free text kept and its signature rewritten; the
  approvals are written for Codex and Claude.
- End to end in each harness with the installed kit: one consultation and one
  execution submitted, waited on through the insert's means, collected; in
  Codex a run longer than 30 s collected through ``exec_command`` and an empty
  ``write_stdin`` poll that returns when the wait exits; a dispatched codex
  backend calls ``dispatch_consult`` and is refused ``dispatch_submit``.
- ``validate.sh`` within the ceilings; no ``aside`` outside records and
  CHANGELOGs; workspace tests, clippy and fmt on the latest stable; a
  scratch-HOME upgrade over a home with aside registered.

Alternatives and costs
----------------------

- aside goes asynchronous on its own, copying dispatch's lifecycle or
  extracting it into a shared crate: keeps aside's small binary and the install
  stable, but either duplicates the store, reconciliation and wait or adds a
  crate and a refactoring of the working executor; rejected because the
  lifecycle is one thing and the install cost of removing a server is paid
  once.
- A single merged server under a new name: nothing gained over keeping
  ``dispatch`` and every registration, rule and prefs name would change.
- An MCP wait tool that blocks: reintroduces the blocking call and the Codex
  timeout; the harness means exist for exactly this.
- Printing the reply from ``wait``: the harness's shell output truncates a 50
  KB reply; ``dispatch_status`` returns it whole.
- Keeping ``DisableMcpServer`` for codex execution backends: simpler, but a
  delegate could no longer consult; kept only as the fallback when codex does
  not take the environment on the command line.

Open questions
--------------

None.

References
----------

- RFC-0008, RFC-0002, RFC-0001, RFC-0014; ADR-0001 (corpus ceilings).
- ``spec/agent-exec.rst``, ``spec/installer.rst``, ``spec/prefs.rst``,
  ``spec/support-matrix.rst``, ``design/architecture.rst``.
- Codex MCP configuration: learn.chatgpt.com/docs/extend/mcp (2026-10-14);
  ``codex-rs/core/src/unified_exec/mod.rs`` (2026-10-14).
- Kimi Code 2.1.1, ``dist/main.mjs`` of the installed package (2026-10-14);
  Claude Code Bash tool contract (2026-10-14).
