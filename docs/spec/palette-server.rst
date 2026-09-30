palette server
==============

:Status: Contract
:Date: 2026-09-30

Scope and authority
-------------------

The palette server is an MCP server (stdio) and a command-line checker built
from the slate crate ``shared/mcp-servers/palette`` as the binary ``palette``.
It reads, checks and writes the palette documents of one project at a time.
The document templates in ``shared/workflows/palette/templates/`` are
authoritative for document structure; the server embeds them at build time and
derives its structural checks from them.

Definitions and model
---------------------

Project
~~~~~~~

A folder containing ``_palette/layout.rst``, or, for the check and generate
commands and the lint tool, any folder whose documents identify their families
(see Families and locations). Every tool takes the project's absolute path as
``project``. The path is canonicalized and accepted only
inside the server's project root or one of its extra roots:

- project root: ``SLATE_PROJECT_DIR``, else ``AGENT_KIT_PROJECT_DIR``, else
  ``CLAUDE_PROJECT_DIR``, else the working directory when it is plausibly a
  project (not the filesystem root, not the home folder itself, not under a
  harness home or the Slate state home), exactly as dispatch resolves it;
- extra roots: ``PALETTE_EXTRA_ROOTS``, an OS path list.

With neither, every tool fails with ``no_project_root`` and says how to set the
roots.

Families and locations
~~~~~~~~~~~~~~~~~~~~~~

``layout.rst`` places each family (backlog, phase, deliverable, state, rfc,
adr, changeset, staging, design, spec, principles, glossary) ``internal`` or at
a path relative to the project. ``internal`` resolves to:

Without a layout, the families are inferred from the documents: every ``.rst``
file outside ``_palette/``, ``.git``, ``target``, ``node_modules``, ``fixtures``
and hidden folders is classified by its title and ``:Status:`` field (``RFC-NNNN:``,
``ADR-NNNN:``, ``Changeset:``, ``Glossary —``, ``Principles —``, ``Backlog —``,
``State —``, ``Phase N —`` in a ``phase-N`` folder, ``Deliverable N:`` in a
``deliverables`` folder, ``:Status: Contract`` for a specification, ``:Status:
Maintained`` with a title ending in ``design`` for a design document); a
document whose title carries a template placeholder is skipped; the staging
family is the folder named ``staging`` whose ``spec`` and ``design`` subfolders
hold the mirrors. Each family is placed where its documents were found; a
family found in two places keeps the first and reports the second (P012); a
family with no documents is internal. This is how a checkout without
``_palette/`` is checked.

- ``_palette/backlog.rst``, ``_palette/state.rst``;
- ``_palette/phase-<N>/phase.rst`` and
  ``_palette/phase-<N>/deliverables/deliverable-<N>-<slug>.rst``;
- ``_palette/<family>/`` for the record and maintained families.

A project path is used as given: ``<path>/rfc-0001-<slug>.rst``,
``<path>/adr-0001-<slug>.rst``, ``<path>/rfc-0001.rst`` (changeset),
``<path>/design/<topic>.rst`` and ``<path>/spec/<topic>.rst`` (staging),
``<path>/<topic>.rst`` (design, spec), and a single file for principles and
glossary. Every file name is lowercase ASCII kebab-case. Each record and
changeset folder has a generated ``index.rst``.

Templates as schema
~~~~~~~~~~~~~~~~~~~

For each family the server reads its template and derives:

- the title pattern: literal text, with ``<NNNN>`` a four-digit number and any
  other ``<...>`` free text;
- the required header fields, in order, and each field's allowed values: a
  value written ``a | b | c`` allows exactly those tokens; ``<...>`` allows free
  text; a value followed by `` — <...>`` requires the token and then free text;
  a field whose value may repeat (``Depends``, ``Supersedes``, ``Related``,
  ``Changes``, ``Revised``) takes entries separated by ``;`` or continuation
  lines, and ``<x>; <x>`` in a template means one or more;
- when every alternative before `` — <...>`` is a plain token, the suffix
  belongs to each of them; when an alternative carries a placeholder, a suffix
  belongs to that alternative alone and a plain token such as ``none`` stands
  bare; alternatives may also follow the dash (``<link> — active | closed``);
- ``<YYYY-MM-DD>`` is checked as an ISO date; any other ``<...>`` is free text;
- a field name containing ``<N>`` (``:phase-<N>:``) stands for zero or more
  fields of that shape;
- the required sections at the second level, in order. A document may add
  subsections under them; a required section with no content states ``None.``

Records and relations
~~~~~~~~~~~~~~~~~~~~~

- A record's number is the one in its file name and title; they agree.
- A record links, in its header or body, only to older records: of its own
  kind, a lower number; of the other kind, an earlier or equal ``Date``. A
  cycle is rejected in every case.
- ``Depends`` lists direct uses; the parenthetical says what is used.
- ``Supersedes`` is stored only on the newer record; the older record's status
  is ``Superseded``.
- Incoming links, the dependency closure and "superseded by" are computed.

Changesets and staging
~~~~~~~~~~~~~~~~~~~~~~

A changeset holds one record's edits to maintained design and spec documents:
``Replace:``, ``Insert after:``, ``Insert into:``, ``Delete:`` against an
existing section (named by its title, prefixed by parent titles and " / " when
not unique), and ``Create:`` for a new document. Staging is each maintained
document with the edits of every accepted record's changeset applied in
dependency order. Changesets of draft and proposed records are checked, not
applied. Staging is only ever generated.

Contract
--------

Lint rules
~~~~~~~~~~

Each finding has a rule id, a severity (``error`` or ``warning``), a file, a
line and a message.

``P001`` RST subset (error)
  A table, a directive other than a comment, an overlined title, an underline
  shorter than its title, a heading level out of the house-style order, a
  substitution, a footnote or a citation.

``P002`` Structure (error)
  A missing, extra or out-of-order required section; a missing or out-of-order
  required field; a value outside the field's allowed values.

``P003`` Identity (error)
  File name, number and title disagree; a file name that is not lowercase
  kebab-case; a duplicate number.

``P004`` Links (error)
  A link whose target file or anchor does not exist; a link from a project path
  into ``_palette/``.

``P005`` Relations
  A link to a record created later (error); a relation cycle (error); a
  ``Depends`` entry reachable through another listed entry whose parenthetical
  does not state a direct use (warning); the same target in ``Depends`` and
  ``Related`` (warning); a record named in a ``Supersedes`` whose status is not
  ``Superseded``, or a record whose status is ``Superseded`` while no newer
  record names it in ``Supersedes`` (error).

``P006`` Changes (error)
  A ``Changes`` target document or section that neither exists nor is created
  by the record's own changeset.

``P007`` Changesets (error)
  An edit that does not resolve against its target with only its record's
  dependency closure applied; two changesets editing one section with no
  dependency between them; a changeset with no record; a record whose
  ``Implementation`` is ``complete`` while its changeset still has edits.

``P008`` Generated files (error)
  A staging document or an index that differs from what the server would
  generate; a staging document no changeset produces.

``P009`` Status placement (error)
  A ``Status`` field, a checkbox (``[ ]`` or ``[x]``) or a done/pending marker
  in a phase, deliverable or state document.

``P010`` Development stages (warning)
  In backlog, phase, deliverable or state: a dated result or progress sentence
  (a date followed by ``started``, ``landed``, ``completed``, ``results``,
  ``dispatched`` or ``merged``), a run or dispatch identifier, or a
  model-routing instruction (a model name with a count, a lane or a batch).

``P011`` Budgets (warning)
  state over 300 lines; a state entry over 3 lines; a phase or deliverable file
  over 120 lines; a backlog item body over 4 lines.

``P012`` Layout (error)
  A family missing from ``layout.rst``, an unknown family, a path outside the
  project, or a project path inside ``_palette/``. ``checker`` is a setting,
  not a family.

``P013`` Backlog (error)
  A duplicate item id; ``in-phase-<N>`` without a phase ``<N>`` file; a
  deliverable link to a missing file; a ``done`` item whose ``Outcome`` is
  ``none`` (warning).

``P014`` Time-varying fields (error)
  ``Implementation``, ``Verification``, ``Implementers`` or ``Revised`` not in
  its template format.

Read tools
~~~~~~~~~~

Read tools change nothing and carry the MCP read-only annotation.
``palette --read-only-tools`` prints their names, one per line.

``palette_lint``
  Input: ``project``; optional ``paths``. Output: the findings, errors first.

``palette_status``
  Input: ``project``; optional ``record``. Without ``record``: phases and their
  status, item counts by status, open questions, discrepancies, decisions not
  yet graduated, lint error count. With ``record``: its header, incoming links,
  dependency closure, what supersedes it, its pending changeset edits and the
  staging documents they affect. The output stays under 4,000 characters and
  says what it omitted.

``palette_layout``
  Input: ``project``. Output: every family with its resolved absolute path.

``palette_template``
  Input: ``family``. Output: the template text.

Write tools
~~~~~~~~~~~

Every write tool takes ``project`` and an optional ``dry_run``; returns the
unified diff of every file it changed (or would change) and the identifiers it
allocated; and runs as one transaction:

- it takes a per-project lock (``_palette/.palette.lock``), waiting at most 10
  seconds, else fails with ``locked``;
- it parses every file it will change and fails with ``parse_error`` (naming
  the file and line) when one does not parse, changing nothing;
- it changes only the lines its operation concerns and keeps each file's line
  endings;
- it writes every changed file to a temporary sibling and renames them into
  place; if any step fails, every file keeps its previous content;
- it regenerates the indexes and staging it affects in the same transaction;
- it runs lint on the files it touched and fails with ``invariant_violation``,
  changing nothing, if the result would contain an error.

Tools:

``palette_init``
  Creates ``_palette/``, ``_palette/.gitignore`` (``*``), ``layout.rst`` from the
  given placements, and empty backlog and state documents. Fails with
  ``already_initialized`` when a layout exists.

``palette_layout_set``
  Moves one family to a new placement, moving its files and rewriting every
  link to them. Requires ``confirmed_by_user: true``; carries the destructive
  annotation.

``palette_backlog_add``
  Adds an item (title, type, source, priority signal, depends, body); returns
  ``B-<n>``.

``palette_backlog_update``
  Changes an item's fields. Status moves only
  ``proposed`` → ``approved`` → ``in-phase-<N>`` → ``done``, or to ``dropped``
  from any status; ``in-phase-<N>`` requires phase ``<N>`` to be active.

``palette_phase_open``
  Creates ``phase-<N>/phase.rst`` from goal, reason, assumptions and exit
  criteria; adds the phase to the backlog as ``active``; moves the given items
  to ``in-phase-<N>``. Fails when another phase is active.

``palette_deliverable_create`` and ``palette_deliverable_update``
  Write a deliverable of the active phase for one backlog item and link it from
  the item.

``palette_phase_close``
  Takes, for every item in the phase, ``done`` with an outcome pointer or
  ``dropped``, plus new proposed items. Updates the backlog, marks the phase
  ``closed``, removes the state pointers of graduated decisions, and deletes
  the phase folder. Carries the destructive annotation.

``palette_state_record``
  Adds a decision (text, source, target), an open question (text, affects,
  optional proposal) or a discrepancy (text, evidence kind, date); returns
  ``D-<n>``, ``Q-<n>`` or ``X-<n>``.

``palette_state_resolve``
  Resolves an entry: a decision ``graduated`` to a record or document (the entry
  becomes a one-line pointer), a question ``answered`` (it becomes a decision)
  or ``withdrawn``, a discrepancy ``fixed`` (removed).

``palette_record_create``
  Allocates the next number of ``rfc`` or ``adr``, writes the record from the
  template with the given fields and sections, and sets the time-varying
  fields to their initial values.

``palette_record_update``
  Changes a record. A draft or proposed record may change freely. An accepted
  record's body changes only with ``clarification: true`` (a mechanical
  correction), which adds a ``Revised`` entry. Status moves ``Draft`` →
  ``Proposed`` → ``Accepted`` (requires the accepting person), or to
  ``Rejected`` or ``Withdrawn``; ``Superseded`` is set only through a newer
  record's ``Supersedes``.

``palette_changeset_edit``
  Adds, replaces or removes one edit in a record's changeset.

``palette_changeset_promote``
  Merges implemented edits of a record into their maintained documents, removes
  them from the changeset, deletes an empty changeset, and sets the record's
  ``Implementation`` (``partial`` or ``complete``), ``Implementers`` and
  ``Verification`` from the given values.

Command line
~~~~~~~~~~~~

- ``palette`` with no arguments serves MCP over stdio.
- ``palette check <project>`` prints the lint findings and exits 1 when there
  is an error, 0 otherwise; it resolves the project without the root rules and
  needs no ``_palette/``.
- ``palette generate <project>`` regenerates every index and staging document
  of the project under the same lock and transaction as the write tools (without
  the lock when ``_palette/`` does not exist), prints the paths it wrote, and
  exits 1 on an error; it is how a repository without an MCP session, or a CI
  checkout, produces its generated files.
- ``palette --read-only-tools`` prints the read tool names.
- ``palette --version``.

Errors and edge cases
---------------------

Every error carries a stable ``code`` and a message that says how to fix it:
``invalid_params``, ``no_project_root``, ``outside_roots``, ``no_layout``,
``already_initialized``, ``not_found``, ``parse_error``, ``locked``,
``invariant_violation``, ``conflict`` (the file changed between read and write),
``io_error``.

Ownership and ordering
----------------------

The server is the only writer of ``Implementation``, ``Verification``,
``Implementers``, ``Revised``, generated indexes and staging. Any other text in
a palette document may also be edited by hand; the next lint reports what
drifted.

Compatibility
-------------

Tool names and input fields are stable within a major version of the kit. A
new optional input field is compatible; a removed or renamed one is not.

Conformance
-----------

``cargo test`` covers every lint rule with synthetic fixtures, including the
failures that motivated the document system (an oversized state, a status
recorded in two files, a dependency cycle) reproduced at small scale; every write tool's
success, dry run, and failure partway (every file unchanged); CRLF files; and
the project resolution rules. The tests pass on Linux, macOS and Windows.

References
----------

- ``RFC-0003`` for the families, placement and relation rules.
- ``shared/workflows/palette/templates/`` for the document structure the
  server derives its checks from.
