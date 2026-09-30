bench
=====

:Status: Contract
:Date: 2026-09-30

Scope and authority
-------------------

``bench`` is an MCP server (stdio) built from the slate crate
``shared/mcp-servers/bench`` as the binary ``bench``, registered by the
installer in every harness like aside, dispatch and palette. It runs a grid
of (backend, model, effort) cells over a task set through agent-exec, each
run in an isolated backend home, and records one JSONL line per run. The
record schema in this document and the crate's public types are
authoritative; charts, scoring and analysis are outside the server.

Definitions and model
---------------------

Cell
~~~~

A ``Backend`` of agent-exec (``codex`` or ``claude``), a ``model`` and a
``reasoning_effort`` as the backend CLI takes them, and a harness condition:
``kit`` (the kit under test, pinned to a version, rendered and installed for
that backend's harness in the run's isolated home) or ``none`` (no kit).

Task
~~~~

A directory in a task set: ``task.toml`` (name, the benchmark it belongs
to, the prompt file, the working-tree manifest, the scorer command, the
timeout), the prompt, the files the manifest assembles into the run's working
tree, and optionally ``documents/`` for the document variant. A task set is
a directory of tasks; the request names it by path.

Variant
~~~~~~~

``plain`` runs the task as it is; ``documents`` also places the task's
``documents/`` in the working tree and names them in the prompt, so document
use is measured. A task without ``documents/`` has only the plain variant.

Run
~~~

One (cell, task, variant, repeat) executed as one ``RunSpec`` of agent-exec:
``JsonStream`` output for codex and ``Json`` for claude so that usage is
parsed from the run itself; ``WorkspaceWrite`` sandbox in the run's working
tree; ``IgnoreUserConfig`` isolation for codex and ``Permissions`` for
claude; the guard on; the capture policy of dispatch; the prompt on stdin.

Record
~~~~~~

One JSONL line per run with these fields: ``cell`` (backend, model, effort,
harness, kit_version or ``none``), ``task``, ``benchmark``, ``variant``,
``repeat``, ``cli_versions`` (the backend CLI and the harness CLI as
``--version`` printed them), ``argv``, ``started_at`` (UTC), ``wall_ms``,
``exit_code``, ``success``, ``cancelled``, ``run_dir``, ``final_text_path``,
``usage`` (the ``Usage`` buckets of agent-exec, each ``null`` when unknown),
``usage_source``, ``reasoning_tokens``, ``cost_usd`` (``null`` when the model
is not priced), ``price_snapshot`` (the date of the table used) and
``schema_version``.

Isolation
~~~~~~~~~

Every run gets ``<output>/runs/<record-id>/`` with ``home/`` holding the
backend's own configuration store (``CODEX_HOME`` for codex,
``CLAUDE_CONFIG_DIR`` for claude), the kit installed there when the cell says
``kit``, the assembled working tree in ``work/``, and ``stdout``,
``stderr`` and ``final.md``. The user's own ``~/.codex`` and ``~/.claude``
receive nothing. Hidden tests are never in ``work/`` while the run lasts.

Price table
~~~~~~~~~~~

LiteLLM's ``model_prices_and_context_window.json``, fetched at most once per
24 hours into the server's state directory, with a snapshot vendored in the
binary as the fallback. A model is looked up by the name the CLI was given,
then by the table's aliases; a model in neither table yields ``cost_usd:
null`` and one warning line in the grid's log.

Cost
~~~~

``cost_usd`` is the sum over buckets of tokens × the table's per-token price:
``input_uncached`` at the input rate, ``cache_read`` at the cache-read rate,
``cache_write_5m`` at the cache-creation rate and ``cache_write_1h`` at the
1-hour rate when the table states them (an OpenAI cache write without an
explicit rate is priced as input), ``output`` at the output rate. Reasoning
tokens are inside ``output`` and are not priced again. A ``None`` bucket
that the run's usage source reports contributes nothing and marks the record
``cost_partial: true``; a bucket the source never reports (for the codex
sources, the 1-hour cache write) counts as zero and marks nothing.

Contract
--------

Tools
~~~~~

``bench_grid``
  Input: ``cells`` (one or more), ``task_set`` (path), ``variants`` (``plain``,
  ``documents``, or both; default both where available), ``repeats``
  (default 3), ``output`` (``scratchpad``, ``harness`` or a project path,
  D-25), ``kit`` (the kit version and checkout to install for ``kit`` cells),
  optional ``tasks`` (a subset by name), optional ``grid_id``, ``dry_run``.
  Starts the grid in the background and returns its identifier and the JSONL
  path. Without ``grid_id`` the identifier is the task set's directory name
  and a hash of the cells, task set, variants, task subset, kit and output;
  ``repeats`` is outside the hash, so raising it extends the same grid. A
  grid whose JSONL already exists resumes: a run whose (cell, task, variant,
  repeat) is present is skipped, a timed-out run included; a run recorded
  as cancelled is run again.

``bench_status``
  Input: ``grid``. Output: runs done, running, skipped and remaining per
  cell, the current run, the last warning, and whether the grid is alive.

``bench_cancel``
  Input: ``grid``, optional ``cell``. Kills the current run of the grid (or
  of the cell) through the guard and marks the remaining runs of the grid or
  cell cancelled; the JSONL keeps every finished record.

``bench_list``
  Output: the grids the server knows with their state and JSONL path.

Output layout
~~~~~~~~~~~~~

``<output>/<grid-id>/grid.toml`` (the request as accepted, the kit version,
the price snapshot date), ``results.jsonl``, ``log.txt`` (warnings and
progress), ``runs/<record-id>/`` per run. ``scratchpad`` resolves to the
harness's scratchpad directory for the session, ``harness`` to the bench
folder in the harness's own configuration store, and a project path to that
path inside the project root or an extra root.

Ordering
~~~~~~~~

Cells run one after another and repeats within a cell one after another;
wall time is a measured quantity and the backends share rate limits. Within
a cell the order is task, then variant, then repeat.

Errors and edge cases
---------------------

- A backend not found, a task set without ``task.toml`` files, a kit
  checkout that does not render, or an output path outside the roots fails
  ``bench_grid`` with ``invalid_params`` before any run.
- A run that exceeds the task's timeout is killed through the guard and
  recorded with ``exit_code: null`` and ``timed_out: true``.
- A run whose usage cannot be parsed keeps ``usage: null`` and
  ``usage_source: null``; the grid goes on.
- A second ``bench_grid`` on the same JSONL while the first is alive is
  refused with ``locked``.

Ownership and ordering
----------------------

- The server owns ``grid.toml``, ``results.jsonl``, ``log.txt`` and the run
  directories; a scorer writes only its own result file inside the run
  directory after the run ends.
- Records are appended, never rewritten; a resumed grid reads them once at
  start.
- The task set, its fixtures, scorers and hidden tests belong to the
  benchmark suite.

Compatibility
-------------

``schema_version`` starts at 1. Within a kit major version a field is added,
never removed or renamed; a chart script that reads a record checks the
version and ignores fields it does not know.

Conformance
-----------

- Synthetic-fixture tests cover the record schema, the cost of a 1-hour
  Anthropic cache write, an OpenAI cached input, a model missing from both
  tables, and resumption after a stopped grid.
- A one-cell dry run against a stub backend on ``PATH`` leaves the user's
  harness stores byte-identical.
- ``cargo test``, ``clippy -D warnings`` and ``fmt --check`` pass on Linux,
  macOS and Windows.

References
----------

- ``spec/agent-exec.rst`` (``RunSpec``, ``RunRecord``, ``Usage``)
- ``design/benchmark-suite.rst``
- LiteLLM ``model_prices_and_context_window.json``
