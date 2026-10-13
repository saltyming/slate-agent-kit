dispatch server
===============

:Status: Contract
:Date: 2026-10-14

Scope and authority
-------------------

The ``dispatch`` MCP server (``shared/mcp-servers/dispatch``) runs one backend
CLI per run on the user's behalf in two kinds: an execution, which is
write-capable in a working directory, and a consultation, which is a read-only
question to another model family carrying the harness's own transcript. Every
run is asynchronous: a submit returns an id, the run continues in the
background, and its status, result and log are read by id. The tool parameter
structs and the ``get_info`` text are authoritative for names and types; this
document is authoritative for the behavior behind them. Spawning, capture,
cancellation and the model fallback loop are ``agent-exec``'s
(`agent-exec.rst <agent-exec.rst>`_); what each harness supports is in
`support-matrix.rst <support-matrix.rst>`_.

Definitions and model
---------------------

Kind
~~~~

``execute``: the backend (codex, claude or opencode) runs in ``working_dir``
under the run's sandbox and may modify files there. ``consult``: the backend
(codex or claude) runs read-only in the server's directory with no MCP server
of its own, answering a question; its prompt is the role framing, the optional
context, the redacted transcript when one was found, the question and the
independence reminder, in that order.

Run
~~~

A row of the store with an id (``d-<n>``), a kind, a backend, a status, the
rendered prompt, the model and effort, the fallback chain and its history, the
owning server's pid and instance, the child pid, the exit code, the result or
the error, and timestamps. Statuses: ``queued``, ``running``, ``succeeded``,
``failed``, ``cancelled``, ``interrupted``. The first two are active; the rest
are terminal. ``interrupted`` is a run whose owning server went away without
ending it, or that the server killed to restart; a restarted run names its
successor in ``restarted_as`` and the successor names it in ``restart_of``.

Store
~~~~~

One SQLite database ``dispatch.db`` per project under the state directory:
``DISPATCH_STATE_DIR`` when set, else ``<state home>/projects/<slug>/dispatch``
where the state home is ``SLATE_AGENT_STATE_HOME``, ``AGENT_KIT_STATE_HOME`` or
``~/.slate-agent-kit`` and the slug is the canonical project path with ``/``,
``\`` and ``:`` flattened to ``-``. Several server processes share one database
(WAL, a 5 s busy timeout). Columns are added by idempotent ``ALTER TABLE``
statements when a server opens the store, so a database from an earlier
release opens; a row without a kind reads as ``execute``.

Transcript
~~~~~~~~~~

For a consultation the server reads the invoking harness's own session log:
Claude Code project transcripts, Codex rollouts (interactive sessions only) or
Kimi Code wire logs, chosen by ``SLATE_HARNESS`` when set, else the newest
candidate; ``DISPATCH_TRANSCRIPT_DIR`` names a directory of Claude-schema logs
instead. The last ``transcript_tail`` messages (default 80) are rendered within
100 KB, trimmed from the front; user and assistant text passes, tool calls
become ``[tool_use: <name>]``, tool results ``[tool_result]`` and reasoning
``[thinking]``. A missing or unreadable source is a note in the result, never a
failure.

Re-entry
~~~~~~~~

Every process the server spawns carries ``DISPATCH_REENTRY_DEPTH`` one above
the server's own depth. At depth 1 or more the server refuses ``execute`` and
``steer`` and accepts ``consult``. A codex execution backend receives, on its
command line, a complete ``dispatch`` server entry whether or not the user's
``config.toml`` registers one: the entry's command (this binary's absolute
path), the marker in its environment and ``approval_mode = "approve"`` for
``dispatch_consult``, ``dispatch_status``, ``dispatch_list``,
``dispatch_logs`` and ``dispatch_backends`` (``-c
mcp_servers.dispatch.command="<binary>"``, ``-c
mcp_servers.dispatch.env.DISPATCH_REENTRY_DEPTH="<depth>"``, ``-c
mcp_servers.dispatch.tools.<tool>.approval_mode="approve"``), because the
backend runs with ``-a never`` and could not call an unapproved tool;
``dispatch_submit`` and ``dispatch_steer`` stay unapproved there and the
server refuses them in any case. A consultation backend runs with no MCP
server (codex ``--ignore-user-config``, claude safe mode), so nothing it
starts reaches the server.

Contract
--------

Tools
~~~~~

``dispatch_submit(objective, working_dir, title?, target_files?,
constraints?, acceptance?, context?, details?, backend?, model?,
reasoning_effort?, sandbox?, plan_id?, skip_git_repo_check?, allow_concurrent?,
model_fallback?)`` starts an execution and returns ``id``, ``kind``, ``status``
(``queued``), ``backend``, the canonical ``working_dir``, ``sandbox``,
``plan_id``, ``wait`` and a note. ``working_dir`` must canonicalize inside the
project root or an extra root (``DISPATCH_EXTRA_ROOTS``). ``sandbox`` is
``read-only``, ``workspace-write`` (default) or ``danger-full-access``, the
last refused unless ``DISPATCH_ALLOW_DANGER`` is set. One active execution per
working directory unless ``allow_concurrent``; consultations count neither
way. A fresh codex execution whose log never associates within
``DISPATCH_RESTART_UNASSOCIATED_SECS`` (default about 30; 0 disables) and whose
directory shows no writes is killed and resubmitted once.

``dispatch_consult(question, backend?, context?, include_transcript?,
transcript_tail?, model?, reasoning_effort?, model_fallback?, title?)`` starts a
consultation and returns ``id``, ``kind``, ``status`` (``queued``),
``backend``, the echoed ``model``, ``reasoning_effort``, ``model_fallback`` and
``include_transcript``, ``transcript_note`` (set when no transcript was
forwarded), ``wait`` and a note. ``backend`` is ``codex`` (default) or
``claude``; ``opencode`` is refused (``invalid_params``). ``include_transcript``
defaults to true; an empty question is ``invalid_params``.

``dispatch_status(id, include_spec?)`` returns the row: id, kind, backend,
status, working directory, sandbox, model, effort, final model and fallback
history when a fallback answered, ``restarted_as`` or ``restart_of`` when set,
the pids and timestamps, and, once terminal, the result or the error. An
active row also reports ``child_process_alive``, ``log_associated`` and
``log_last_write_age_seconds``. The result of a consultation is the
``[<backend>]`` header, the reply, a truncation note when the reply exceeded
50 KB, the fallback note and the transcript note; the result of an execution
is the captured stdout with its truncation note. ``include_spec`` adds the
accepted spec, the rendered prompt and the argv.

``dispatch_list(plan_id?, status?, kind?)`` lists rows in insertion order
without results. ``dispatch_cancel(id | plan_id)`` fires the cancellation
token, kills the backend's process group and moves the run to ``cancelled``;
it applies to both kinds and is the only way to stop a consultation after
submission. ``dispatch_logs(id, ...)`` shows the backend's own log for an
execution (codex rollout, claude session log, dispatch's opencode JSONL); for a
consultation it shows the backend's session log when one is located and
otherwise says that a consultation has no live log. ``dispatch_steer(id,
instruction, ...)`` cancels an execution and resumes its backend session with
the instruction as a new linked run; a consultation is refused
(``invalid_params``). ``dispatch_backends()`` lists codex, opencode and claude
with availability, version and the kinds each serves, plus the containment
configuration.

The ``wait`` field
~~~~~~~~~~~~~~~~~~

Every submit, consult and steer response carries ``wait``: the command line
``<absolute path of this binary> wait <id> --db <absolute path of the store>``,
each path quoted when it contains whitespace or a shell-special character and
without a Windows verbatim prefix. The harness runs it with its background
means; the absolute paths are given because the harness's shell carries
neither the server's state home nor its binary folder.

The wait subcommand
~~~~~~~~~~~~~~~~~~~

``dispatch wait <id> [--timeout <seconds>] [--db <path>]`` runs when the
binary's first argument is ``wait``, before any server initialization: it opens
the existing store named by ``--db`` (else the store the server would resolve
from the environment and the current directory), creates nothing, runs no
schema upgrade (a store from before this release gives exit 1), probes no
backend and reconciles no other row. ``--timeout`` is a non-negative finite
number of seconds; any other value is a usage error. It reads the row every
500 ms outside any transaction until the row is terminal or the timeout
elapses. When the row is active and its owner pid is set and no longer
running, it marks the row ``interrupted`` through the same conditional update
the server's own reconciliation uses and reads the row again; a row whose
owner pid is unset is waited on. It prints one line on stdout, ``<id> <kind>
<status>`` followed by ``model=<final model>`` when a fallback answered and
``restarted_as=<id>`` when set, and never the result. Exit codes: 0 terminal,
2 still active at the timeout (the line is printed), 3 unknown id, 1 usage or
store error (stderr says which). Liveness is by pid; a reused pid is a known
limit, bounded by ``--timeout``.

Run outcome
~~~~~~~~~~~

A run's terminal status, exit code, result or error, final model and fallback
history are written in one statement; a reader that sees a terminal status
sees the whole outcome. ``succeeded`` means the backend exited 0, not that its
work is correct.

Errors and edge cases
---------------------

Tool errors are JSON ``{"error": {"code", "message"}}`` with the codes
``invalid_params``, ``no_such_task``, ``invalid_working_dir``,
``no_project_root``, ``sandbox_forbidden``, ``dir_busy``,
``session_not_ready``, ``unknown_backend``, ``rollout_unreadable``,
``db_error`` and ``reentrant``. A reentrant refusal names the depth. When the
server starts it marks every active row owned by a process that is not
running, or by no process, ``interrupted``; a read of an active row owned by a
dead process does the same. On a termination signal the server cancels its
own runs, which end ``cancelled``. A quiet log on a live process is
inconclusive: a backend inside a long tool call emits nothing until it
returns.

Ownership and ordering
----------------------

The server owns ids, statuses and the store; ``wait`` writes only the
``interrupted`` transition described above. A row is written by the server
that owns it; another server reads it and reconciles it only when the owner is
gone. Terminal rows are never changed. ``dispatch_steer`` creates a new row
linked by ``parent_id``; an automatic restart creates a new row linked by
``restart_of``.

Compatibility
-------------

A store from an earlier release opens; its rows read as ``execute``. The
``aside`` server, its tools and its ``ASIDE_*`` environment variables are
retired with this contract; the installer removes its registration. A caller
that treats a terminal status as success must check for ``succeeded``.

Conformance
-----------

The crate's tests cover the argv of each kind and backend, the prompt order of
a consultation, the result and failure texts, the kind default on an old
store, listing by kind, re-entry per kind, the atomic outcome write, and the
``wait`` exit codes on a terminal, an active, an unknown and an orphaned row.
An end-to-end check in each harness submits one run of each kind, waits
through the harness's background means and reads the result.

References
----------

- RFC-0022 (this contract), RFC-0008 (agent-exec), RFC-0014 (the guard).
- `agent-exec.rst <agent-exec.rst>`_, `installer.rst <installer.rst>`_,
  `prefs.rst <prefs.rst>`_, `support-matrix.rst <support-matrix.rst>`_.
