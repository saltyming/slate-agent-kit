agent-exec
==========

:Status: Contract
:Date: 2026-09-30

Scope and authority
-------------------

``agent-exec`` is the library crate ``shared/crates/agent-exec`` that runs one
headless invocation of a backend CLI process for the servers of this
workspace. It owns binary lookup, argv construction, spawning under the guard,
output capture under a policy, cancellation, failure classification and the
model fallback loop, and it returns one record per run with the backend's token
usage normalized. ``harness-log``, the library crate
``shared/crates/harness-log``, locates a backend's session files and parses
token usage from a backend's own output. The public items of the two crates
are authoritative for names and types; this document is authoritative for the
behavior behind them.

A server decides what to run and what to do with the result: dispatch composes
a consultation's prompt and renders its reply, and keeps its run store, its
watchdogs and its opencode runner for both kinds of run. The crates write no
file and no store of their own.

Definitions and model
---------------------

Backend
~~~~~~~

``Backend`` is ``Codex``, ``Claude`` or ``Agy``: a CLI spawned as a process
with the prompt on stdin. Each names its binary, an install hint and a
``--version`` probe. ``Agy`` is the Antigravity CLI (``agy``) and serves
execution only; dispatch refuses it for a consultation. Each backend also
fixes how the prompt is written and how the outcome is decided:
``Backend::input_encoding()`` is ``InputEncoding::Verbatim`` for codex and
claude and ``InputEncoding::AgyStreamJson`` for agy (the prompt serialised as
the one line ``{"event":"user","message":{"role":"user","content":<prompt>}}``,
a newline, then end of input, by ``InputEncoding::encode``);
``Backend::result_normaliser()`` is ``ResultNormaliser::ExitStatus`` for codex
and claude and ``ResultNormaliser::AgyStreamJson`` for agy (Outcome and
RunRecord). ``Backend::sandbox_refusal(sandbox)`` names the reason a backend
cannot run under a sandbox level, before anything is spawned (agy: the
read-only level, because headless agy has no read-only execution). Opencode is
not a process backend: dispatch drives ``opencode serve`` over HTTP in its own
runner and reports through the types below.

Sandbox
~~~~~~~

``Sandbox`` is ``ReadOnly``, ``WorkspaceWrite`` or ``DangerFullAccess``. Codex
receives it as ``-s``; Claude as a permission mode (``plan``; ``acceptEdits``
with ``Bash`` allowed; ``--dangerously-skip-permissions``); Agy always runs
with ``--dangerously-skip-permissions`` (headless agy denies every tool that
needs a permission otherwise), adds ``--sandbox`` (its terminal sandbox) for
``WorkspaceWrite``, adds nothing for ``DangerFullAccess``, and refuses
``ReadOnly`` through ``Backend::sandbox_refusal`` before the spawn. The
ceiling that refuses ``DangerFullAccess`` belongs to the server.

Isolation
~~~~~~~~~

``Isolation`` states how a spawned backend is kept from reaching back into the
server that spawned it. For Codex: ``IgnoreUserConfig`` (``exec
--ignore-user-config``, no MCP server at all), ``DisableMcpServer(name)``
(``-c mcp_servers.<name>.enabled=false`` before ``exec``, every other server
kept) or ``ServerEnv`` (``-c mcp_servers.<server>.command="<command>"``, ``-c
mcp_servers.<server>.env.<var>="<value>"`` and, for each tool the server
pre-approves, ``-c mcp_servers.<server>.tools.<tool>.approval_mode="approve"``,
in that order before ``exec``; every server kept and the named server started
with that variable set). The command is given so the entry is complete when
the user's ``config.toml`` does not register the server, since Codex refuses
an entry with an environment and no command; the approvals are given because
a backend run with ``-a never`` cannot call a tool the configuration has not
approved; every value is a TOML string, since Codex rejects an integer there.
For Claude: ``SafeMode { tools }`` (``--safe-mode --no-session-persistence
--permission-mode plan --tools <list>``) or ``Permissions`` (the sandbox
mapping alone). For Agy: ``Permissions`` only; agy has no configuration
override for one run and passes its own environment to the MCP servers it
starts, so the re-entry marker on the process environment reaches a nested
dispatch, and a nested agy sees every server the user registered. Every
spawned process also carries the re-entry marker (see Re-entry).

OutputMode
~~~~~~~~~~

``OutputMode`` is ``Default`` (no format flag; the backend's final message on
stdout), ``Text`` (Claude ``--input-format text --output-format text``, as a
consultation sends), ``Json`` (Claude ``--output-format json``) or
``JsonStream`` (Codex ``exec --json``). ``Json`` and ``JsonStream`` are what
usage is parsed from; the other modes yield a record whose ``usage`` is
``None`` unless the caller later attaches usage parsed from a session file.
Agy takes no output mode flag from the spec: its argv always carries
``--input-format stream-json --output-format stream-json --print-timeout 0``,
its stdout is a stream of NDJSON events (``init``, ``step_update``,
``result``) that the agy result normaliser reads, and its usage comes from the
``result`` event (Usage mapping).

CapturePolicy
~~~~~~~~~~~~~

How much of stdout and stderr is kept and how: ``stdout_cap`` and
``stderr_cap``, each with ``Keep::Head`` or ``Keep::Tail`` and ``Unit::Bytes``
or ``Unit::Chars``, and ``failure_stdout_cap`` for stdout on a non-zero exit.
Overflow is drained so the child never blocks, discarded, and marked in the
record. The consultation policy: stdout 50 KB head by character, stderr 2 KB
tail by character, failure stdout 2 KB head by character. The execution
policy: stdout
200 KB head by byte, stderr 16 KB head by byte, failure stdout the same as
stdout. For agy the policy governs the retained diagnostic text only: the
stream is read to its end whatever the cap, the ``init`` and ``result`` events
are parsed as they arrive and kept apart from the retained capture (a
``result`` line over 8 MB is not read and the run fails saying so), and the
retained ``stdout`` holds the raw event lines under the cap, so the fields
that decide the outcome are never lost to the cap.

FailureTextPolicy
~~~~~~~~~~~~~~~~~

What the classifier and the user see when a run fails: stderr always;
stdout for the backends listed in ``stdout_for`` (both kinds: Claude only,
because Claude prints the discriminating error on stdout and exits non-zero
with only warnings on stderr, and a codex answer that mentions a rate limit
must not read as one), clipped to ``stdout_tail`` characters when set
(execution: 2,000; consultation: unset, the failure capture cap applies). For
agy the result normaliser supplies ``RunRecord.failure_detail`` first: the
result's ``error``, the ``AGY_ERROR`` line (stdout, else the last stderr line
that names it), ``agy ended without a result event``, ``result event was not
well-formed``, ``agy reported status <status>``, the over-bound text, or the
denied-actions text (each denied action's ``display_name`` and that it needed
a permission headless agy cannot grant); stderr follows it. A successful agy
reply is never classified, so an error-like word inside ``response`` does not
trigger a fallback.

RunSpec
~~~~~~~

One attempt: ``backend``, ``prompt``, ``working_dir`` (optional; set as the
process directory and, for Codex, ``-C``), ``sandbox``, ``isolation``,
``output_mode``, ``capture``, ``failure_text``, ``model``,
``reasoning_effort``, ``resume_session`` (Codex ``exec resume``, Claude
``--resume``, Agy ``--conversation``), ``pin_session`` (Claude
``--session-id``; with ``resume_session`` it adds ``--fork-session``),
``skip_git_repo_check`` (Codex), ``env`` (extra variables), ``reentry`` (the
marker name and ceiling) and ``guard`` (``Required``, ``Preferred`` or
``Off``; see Guard). The input encoding and the result normaliser are not
fields: the backend fixes them (Backend).

RunEvent
~~~~~~~~

What a run reports while it lasts, to the channel the caller supplies:
``Started { pid, argv, backend_version, guarded }`` right after the spawn
(``guarded`` is true when the process runs under the parent-death guard and
false for ``Off`` and for a ``Preferred`` run that found none);
``Session { session_id, port }`` when a backend reports a session (dispatch's
opencode runner sends it after creating or resuming the session);
``Progress(line)`` for each normalized JSONL event a runner produces, with
the wire names dispatch writes today (``session_meta``, ``user_message``,
``agent_message``, ``reasoning``, ``custom_tool_call``,
``custom_tool_call_output``, ``patch_apply_end``, ``task_started``,
``task_complete``; a line normalized from an opencode event carries that
event as ``native``, and a line from a message part carries its ``partID``
when the part has one); ``Finished(Outcome)``
for every terminal outcome, including ``Cancelled`` and a startup failure.

Outcome and RunRecord
~~~~~~~~~~~~~~~~~~~~~

``Outcome`` is ``Record(RunRecord)`` when the process was spawned and awaited
(success or failure), ``NotFound { binary, hint }`` when ``which`` finds
nothing, ``Spawn(message)`` when the process could not be started, writing
its stdin failed for a reason other than a broken pipe, or the backend refused
the sandbox level before the spawn (``Backend::sandbox_refusal``),
``WaitFailed(message)`` when awaiting its exit failed, ``Cancelled`` when the
token fired. A broken pipe on stdin (the child exited before reading the
prompt) is not a failure of its own: the exit status and the captured output
decide the outcome, so the child's own error text reaches the classifier. The
prompt is written while stdout and stderr are read, so neither side can block
the other.

The backend's result normaliser decides ``success``. ``ExitStatus`` (codex,
claude): the exit status, as before; ``final_text`` is the retained stdout
under ``Default`` and ``Text``, on success and on failure alike, and the
backend's last assistant message when a JSON mode gives one.
``AgyStreamJson`` (agy): success is exit 0 and a well-formed ``result`` event
with ``status`` ``SUCCESS``, no ``error``, and not ``denied_actions`` beside an
empty ``response``; ``final_text`` is ``response``, with a note naming the
denied actions appended when ``denied_actions`` is present and the response
is not empty; the ``init`` event's ``conversation_id`` is reported as
``RunEvent::Session`` the moment it is read and stored as ``session_id``; an
exit-0 run the normaliser fails keeps its exit code as observed.

``RunRecord`` is the serializable result of one attempt: ``backend``,
``argv``, ``model``, ``reasoning_effort``, ``started_at``, ``wall_ms``,
``exit_code``, ``success``, ``stdout`` and ``stderr`` as kept by the capture
policy with ``stdout_total`` and the two truncation flags, ``final_text`` (as
above), ``failure_detail`` (the normaliser's own account of a failure, when it
has one), ``session_id`` when the backend reports one, ``backend_version``,
and ``usage: Option<Usage>``.

Usage
~~~~~

Token counts of one run. Five disjoint buckets, each ``Option<u64>``:
``input_uncached``, ``cache_read``, ``cache_write_5m``, ``cache_write_1h`` and
``output``; and ``reasoning``, the part of ``output`` spent on reasoning or
thinking, which overlaps ``output`` and is never added to it. A count the
source does not report is ``None``; a bucket computed from a ``None`` operand
is ``None``; zero means the source reported zero. ``usage_source`` names where
the counts came from: ``codex_json_stream``, ``codex_rollout``,
``claude_json_result`` or ``opencode_message``. When a run has several turns
or iterations, the buckets are sums over them, and a sum with a ``None`` term
is ``None``.

Re-entry
~~~~~~~~

A server names an environment variable and a ceiling (dispatch:
``DISPATCH_REENTRY_DEPTH``, ceiling 1). Every spawned process, including the
``--version`` probe, carries the variable at the server's depth plus one; a
server refuses a request when its own depth is at or above its ceiling, except
the request kinds it declares safe past the ceiling (dispatch: a consultation,
whose backend sees no MCP server). An unset or empty value is depth 0; a
malformed or non-Unicode value is past every ceiling. A backend that starts
its MCP servers with a clean environment receives the marker through
``Isolation::ServerEnv`` instead.

Guard
~~~~~

A guarded backend dies with the server that started it, together with
everything it spawned. On Linux and macOS the backend is started through
``agent-guard``, a small executable of its own built from this crate:
``agent-guard <parent pid> -- <program> <args...>`` leads a new process
group, starts the program in it, kills the group when the parent dies and
otherwise exits with the program's status. The crate finds ``agent-guard``
beside the running executable, else on ``PATH``. No server re-invokes itself
and no server's ``main`` takes part: an executable that links the crate needs
no code for the guard. On Windows the backend is placed in a Job Object that
kills on close, inside the server process.

``RunSpec.guard`` is ``Required`` (without ``agent-guard`` the attempt is
``Spawn`` with a message saying how to install it), ``Preferred`` (without it
the backend runs unguarded and the ``Started`` event says so) or ``Off``.
an execution uses ``Required``; a consultation uses ``Preferred``.

The crate sets ``AGENT_EXEC_GUARD`` on the guard process, and ``agent-guard``
removes it from the program's environment. ``run`` refuses with ``Spawn`` when
it finds the variable in its own environment: a process that was started as a
guard and is not one must not start backends.

Contract
--------

Functions
~~~~~~~~~

- ``which(binary)`` returns the first file named ``binary``, or ``binary.exe``
  on Windows, on ``PATH``; it does not test executability or find ``.cmd``
  shims.
- ``version(backend, reentry)`` runs ``<binary> --version`` with the given
  re-entry marker stamped and returns its first non-empty output, else
  ``None``; ``version_of(binary, reentry)`` does the same for a CLI named by
  its binary.
- ``install_hint(backend)`` returns one line telling how to install the CLI.
- ``run(spec, events, ct)`` performs one attempt and returns its ``Outcome``,
  after sending ``Started`` and ``Finished``.
- ``run_with_fallback(build, models, events, ct)`` calls ``build(index,
  model)`` to obtain the ``RunSpec`` of each attempt, so the server prepares
  what an attempt needs before the spawn (dispatch: a fresh prompt nonce, a
  cleared association, a fresh pinned session id, a rollout snapshot); it runs
  the attempts in order while the previous outcome's failure text classifies as
  retry-worthy and an attempt remains, and returns the final ``Outcome``, the
  model that produced it and the discarded attempts (model, kind, the first
  2,000 characters of the failure text). A ``Cancelled`` outcome ends the
  chain.
- ``fallback_chain(attempt, models, events, ct)`` is the same chain for an
  attempt that is not a ``RunSpec`` (dispatch's opencode runner): ``attempt(
  index, model)`` performs one attempt, reports through ``events`` itself and
  returns its outcome with its failure text; ``run_with_fallback`` is built on
  it.
- ``errkind::classify(text)`` returns ``BackendErrorKind``: ``RateLimited``,
  ``QuotaOrBilling``, ``ModelUnavailable``, ``AuthOrPermission`` or ``Other``;
  the first three are retry-worthy. The text of a ``Spawn`` or ``WaitFailed``
  outcome is classified like a failed exit; ``NotFound`` is never retried.
- ``reentry::depth(name)``, ``reentry::stamp(command, name)`` and
  ``reentry::refused(name, ceiling)`` implement Re-entry.
- ``guard::locate()`` returns the path of ``agent-guard`` or ``None``;
  ``guard::wrap(command, guard_path)`` returns the guarded form of a command.
- ``guard::spawn_contained(command, mode, label, configure)`` starts a command
  under a ``GuardMode``: on Linux and macOS wrapped in ``agent-guard`` in a
  new process group, on Windows created suspended, placed in its Job Object
  and resumed. ``run`` and dispatch's opencode runner start processes only
  through it.

Argv
~~~~

Codex, in this order: ``-C <working_dir>`` when set; ``-s <sandbox>``;
``-a never``; ``-m <model>`` when set; ``-c model_reasoning_effort=<effort>``
when set; ``-c mcp_servers.<name>.enabled=false`` for
``DisableMcpServer``; ``exec``; ``--ignore-user-config`` for
``IgnoreUserConfig``; ``resume <session>`` when resuming;
``--skip-git-repo-check`` when set; ``--json`` for ``JsonStream``. No
positional prompt: the prompt is written to stdin and stdin is closed.

Claude, in this order: ``-p``; the isolation and sandbox flags (``--safe-mode
--no-session-persistence --permission-mode plan --tools <list>`` for
``SafeMode``; ``--permission-mode plan``, ``--permission-mode acceptEdits
--allowedTools Bash`` or ``--dangerously-skip-permissions`` for
``Permissions`` by sandbox); the format flags of the output mode
(``Default``: none; ``Text``: ``--input-format text --output-format text``;
``Json``: ``--output-format json``); ``--model <model>`` when set; ``--effort
<effort>`` when set; ``--resume <session> --fork-session`` when resuming;
``--session-id <id>`` when pinned. The prompt is written to stdin.

Agy, in this order: ``--dangerously-skip-permissions --input-format
stream-json --output-format stream-json --print-timeout 0``; ``--sandbox``
unless the sandbox is ``DangerFullAccess`` (``ReadOnly`` never reaches the
spawn); ``--model <model>`` when set; ``--effort <effort>`` when set and the
model slug does not already name its effort (``<slug>-low``, ``-medium``,
``-high``); ``--conversation <session>`` when resuming. The prompt is written
to stdin as the stream-json ``user`` event and stdin is closed; the working
directory is the process directory.

Usage mapping
~~~~~~~~~~~~~

``harness-log`` exports one parser per source, each returning
``Option<Usage>``. Reasoning is inside ``output`` for every source:

- ``codex_json_stream(text)``: over every ``turn.completed`` event, with
  ``uncached = max(0, input_tokens - cached_input_tokens)`` and ``write =
  max(0, min(cache_write_input_tokens, uncached))``: ``input_uncached =
  uncached - write``, ``cache_read = cached_input_tokens``, ``cache_write_5m =
  write``, ``output = output_tokens``, ``reasoning =
  reasoning_output_tokens``.
- ``codex_rollout(path, baseline)``: the same buckets from the ``token_count``
  events after ``baseline`` (the line the caller recorded before the run, so a
  resumed session counts only its new turns); each delta is
  ``last_token_usage`` when present, else the change in ``total_token_usage``;
  the cumulative baseline advances on every event; a ``token_count`` event
  byte-identical to the previous one is a re-emission and is skipped.
- ``claude_json_result(text)``: from the result's ``usage``,
  ``input_uncached = input_tokens``, ``cache_read =
  cache_read_input_tokens``, ``cache_write_5m =
  cache_creation.ephemeral_5m_input_tokens``, ``cache_write_1h =
  cache_creation.ephemeral_1h_input_tokens``, ``output = output_tokens``,
  ``reasoning = output_tokens_details.thinking_tokens``; when
  ``usage.iterations`` is present the buckets are summed over it.
- ``opencode_message(value)``: ``input_uncached = tokens.input``,
  ``cache_read = tokens.cache.read``, ``cache_write_5m = tokens.cache.write``,
  ``output = tokens.output + tokens.reasoning`` (opencode reports output
  without reasoning), ``reasoning = tokens.reasoning``.
- agy (``agent-exec``'s normaliser, source ``agy_stream_json``, the
  ``UsageSource::AgyStreamJson`` value): from the ``result`` event's
  ``usage``, ``input_uncached = input_tokens``, ``cache_read =
  cache_read_tokens`` (a bucket beside input: it exceeded ``input_tokens`` in
  a probe), ``output = output_tokens`` (reasoning included: ``total_tokens =
  input_tokens + output_tokens`` and ``thinking_tokens`` never exceeded it),
  ``reasoning = thinking_tokens``; agy reports no cache write.

A field absent from the source leaves its bucket ``None``; a parser never
substitutes zero. Whether a ``cache_write_5m`` count costs more than plain
input is the price table's business (an OpenAI model without an explicit
cache-write rate prices it as input); the bucket only carries the count.

Session location
~~~~~~~~~~~~~~~~

``harness-log`` keeps the location functions the servers use today: the
Codex home and rollout listing, session metadata, the newest interactive
rollout for a directory, the rollout for a nonce or a session id, and the
Claude session file for a session id. An agy run's session is its
``conversation_id`` from the ``init`` event, and its log is
``<home>/.gemini/antigravity-cli/brain/<conversation_id>/.system_generated/logs/transcript.jsonl``
(``home`` from ``HOME``, ``USERPROFILE`` on Windows); dispatch resolves it
(``dispatch-server.rst``, Transcript), the crate only reports the identity.

Errors and edge cases
---------------------

- A missing binary is ``NotFound`` with the install hint.
- Cancellation kills the process group (or the Job Object) and returns
  ``Cancelled``; no record is produced for that attempt and no further attempt
  is made. An exit already observed when the token fires yields its record.
  After the exit the process tree stays owned until stdout and stderr are
  drained; a cancellation while a descendant still holds one of them open
  kills the whole tree and yields ``Cancelled``.
- A ``Spawn`` or ``WaitFailed`` outcome carries its message as the failure
  text; it is retried when the message classifies as retry-worthy.
- A record whose output mode gives no parseable usage carries ``usage:
  None``; a server may attach usage parsed from a session file afterwards.

Ownership and ordering
----------------------

- The server owns the ``RunSpec``: its isolation, sandbox, output mode,
  capture and failure-text policies and re-entry marker are the server's
  policy; the crate applies them.
- The crate owns argv construction: a server never appends to the argv it
  gets back.
- The server owns everything written to disk or a store; the crate reports
  through ``RunEvent`` and its return value only.
- ``agent-guard`` is installed beside the servers on Linux and macOS and is
  the only executable that runs the guard.
- Fallback attempts are built and run in chain order; each attempt sends its
  own ``Started`` and ``Finished``.

Compatibility
-------------

The crates are workspace-internal and versioned with the workspace. Within
one kit contract number a ``RunRecord`` or ``Usage`` field is added, never
removed or renamed, so a record written by one server is readable by another; a new
``usage_source`` value is added when a new source is parsed.

Conformance
-----------

- argv tests per backend, isolation and output mode assert the flag set and
  order above, including agy's per sandbox level, with a model slug carrying
  its effort, and when resuming.
- Capture and failure-text tests reproduce each server's policy numbers.
- Spawn, capture, truncation and cancellation tests run against a stub
  executable placed on ``PATH``; a fallback test shows the attempt builder
  called before each spawn.
- The agy normaliser is tested on recorded fixtures and synthetic streams: the
  stdin event line with quotes, newlines and non-ASCII; success; denials with
  an empty and with a non-empty response; ``status`` ``ERROR``; exit 3 with an
  ``AGY_ERROR`` line; exit 0 without a ``result``; a malformed ``result``; a
  ``result`` over the bound; events split across reads and across UTF-8
  boundaries; output beyond the capture cap with the ``result`` still read;
  the exit code kept on an exit-0 failure; the usage mapping and an absent
  field; the identity reported once from ``init``; the read-only refusal; an
  error word inside a successful response left unclassified; and, for codex
  and claude, ``final_text`` equal to the retained stdout before and after.
- Each usage parser has a synthetic fixture and tests for an absent field,
  the codex clamp, the opencode output sum and the rollout baseline.
- Guard tests run on Linux and macOS; the Job Object is exercised on
  Windows CI.
- ``cargo test``, ``clippy -D warnings`` and ``fmt --check`` pass for the
  workspace on Linux, macOS and Windows.

References
----------

- `design/architecture.rst <../design/architecture.rst>`_
- opencode ``packages/opencode/src/session/session.ts`` (``getUsage``)
- codeburn 0.9.25, ``src/models.ts`` and ``src/providers/codex.ts``
