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
adr, changeset, staging, design, spec, principles, glossary, contributing)
``internal`` or at a path relative to the project. ``internal`` resolves to:

- ``_palette/backlog.rst``, ``_palette/state.rst``;
- ``_palette/phase-<N>/phase.rst`` and
  ``_palette/phase-<N>/deliverables/deliverable-<N>-<slug>.rst``;
- ``_palette/<family>/`` for the record and maintained families.

A project path is used as given: ``<path>/rfc-0001-<slug>.rst``,
``<path>/adr-0001-<slug>.rst``, ``<path>/rfc-0001.rst`` (changeset),
``<path>/<topic>.rst`` (design, spec), and a single file for principles,
glossary and contributing. Every file name is lowercase ASCII kebab-case. Each
record and changeset folder has a generated ``index.rst``.

The staging folder holds the mirrors: ``<staging>/design/<topic>.rst`` and
``<staging>/spec/<topic>.rst`` for design and spec documents,
``<staging>/<family>.rst`` for principles, glossary and contributing, and
``<staging>/<path relative to the document root>`` for any other document a
changeset edits. Every ``.rst`` under the staging folder is a staging document.
The document root is the folder that holds the staging folder; a project
whose staging is internal has ``_palette/`` as its document root. A staging
placement that contains another family's placement, or a family placed inside
the staging folder, is an error (P012).

Without a layout, the families are inferred from the documents: every ``.rst``
file outside ``_palette/``, ``.git``, ``target``, ``node_modules``, ``fixtures``
and hidden folders is classified by its title and ``:Status:`` field (``RFC-NNNN:``,
``ADR-NNNN:``, ``Changeset:``, ``Glossary —``, ``Principles —``, ``Backlog —``,
``State —``, ``Phase N —`` in a ``phase-N`` folder, ``Deliverable N:`` in a
``deliverables`` folder, ``:Status: Contract`` for a specification, ``:Status:
Maintained`` with a title ending in ``design`` for a design document); a
document whose title carries a template placeholder is skipped. The staging
family is the folder named ``staging`` that holds a ``design`` or ``spec``
subfolder or lies beside an inferred record, design or spec folder; its
contents are staging documents and are never classified. Each family is placed
where its documents were found; a family found in two places keeps the first
and reports the second (P012); a family with no documents is internal. This is
how a checkout without ``_palette/`` is checked.

Templates as schema
~~~~~~~~~~~~~~~~~~~

For each family the server reads its template and derives:

- the title pattern: literal text, with ``<NNNN>`` a four-digit number and any
  other ``<...>`` free text;
- the required header fields, in order, and each field's allowed values: a
  value written ``a | b | c`` allows exactly those tokens; ``<...>`` allows free
  text; a value followed by `` — <...>`` requires the token and then free text;
  a field whose value may repeat (``Depends``, ``Supersedes``, ``Related``,
  ``Changes``, ``Revised``, and ``Amends`` where present) takes entries
  separated by ``;`` or continuation lines, and ``<x>; <x>`` in a template
  means one or more;
- when every alternative before `` — <...>`` is a plain token, the suffix
  belongs to each of them; when an alternative carries a placeholder, a suffix
  belongs to that alternative alone and a plain token such as ``none`` stands
  bare; alternatives may also follow the dash (``<link> — active | closed``);
- ``<YYYY-MM-DD>`` is checked as an ISO date and ``<YYYY-MM-DDTHH:MMZ>`` as an
  ISO date and 24-hour time in UTC with the ``Z`` suffix; any other ``<...>``
  is free text;
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
- ``Supersedes`` is stored only on the newer record. A whole entry
  ``RFC-<N> (<what is replaced>)`` replaces the record, whose status is then
  ``Superseded``; a partial entry ``RFC-<N> (in part: <what is replaced>)``
  replaces the part named and neither sets nor forbids the older record's
  status. A record is ``Superseded`` exactly when at least one whole entry
  names it.
- ``Amends`` is a relation of older records: ``RFC-<N> (<what changed>)`` or
  ``ADR-<N> (<what changed>)``, one or more entries, placed after
  ``Related``, under the same age rule as every other relation. An RFC may
  amend an RFC or an ADR; an ADR may amend only an ADR, since an ADR decides
  within an RFC's contract and cannot change it. No template lists the field
  and no tool writes it. The contributing document bounds it: its
  ``Records`` section states ``:Amends: none`` or ``:Amends: until
  <YYYY-MM-DD>``, and a record may carry ``Amends`` only when its ``Date`` is
  on or before that date; ``none``, a missing section or no contributing
  document admits no record. Being a document, the cutoff is checked the same
  way with and without a layout.
- ``Accepted`` is ``none``, ``<who>`` or ``<who> (<YYYY-MM-DDTHH:MMZ>)``: the
  person who accepted the record and when, in UTC. The contributing
  document's ``Records`` section may state ``:Acceptance: who-and-time`` or
  ``:Acceptance: who`` after ``Amends``; the field is optional. With
  ``who-and-time`` the form without the parenthetical is an error; with
  ``who`` it is not reported; without the field, a missing section or no
  contributing document it is a warning. The setting is read the same way
  with and without a layout. The server writes the full form on acceptance
  whatever the setting.
- ``Implementation`` is one of ``not-started``, ``in-progress`` (work under
  way), ``partial`` (a part implemented, no work under way), ``complete``,
  ``abandoned``, ``unassessed`` (an inherited claim that has not been
  re-verified) or ``not-applicable``, followed by `` — <scope>``. Only
  ``complete`` is complete: a record whose changeset still holds edits cannot
  be ``complete`` (P007), and promote sets it only when every edit has
  landed.
- Incoming links, the dependency closure, "superseded by" (with the part when
  the supersession is partial) and "amended by" are computed.

Changesets and staging
~~~~~~~~~~~~~~~~~~~~~~

A changeset holds one record's edits to maintained documents. A target is
written as a ``Changes`` entry is: ``design/<topic>.rst`` or
``spec/<topic>.rst``; ``principles.rst``, ``glossary.rst`` or
``contributing.rst``; or the project-relative path of another ``.rst`` file
under the document root that lies outside every family folder, the staging
folder and ``_palette/``, spelled as its path components are spelled on disk.
A family document is a target wherever its family is placed, internal
included. A ``.md`` document, a record, an index, a
changeset, a staging document, a work document, a document outside the
document root, a family document written by its real path, and a document
whose mirror would land in the ``design`` or ``spec`` mirror folder or equal
a single-file mirror are refused, each with the rule it fails. One resolution
of a target, its maintained file and its mirror serves the parser, the tools,
staging and promotion, so a refused target is never applied.

The edits are ``Replace:``, ``Replace body:``, ``Insert after:``, ``Insert
into:`` and ``Delete:`` against an existing section (named by its title,
prefixed by parent titles and " / " when not unique), and ``Create:`` for a
new design or spec document. ``Replace:`` replaces the section together with
its subsections; the body restates the subsections that stay. ``Replace
body:`` replaces the section's heading and body up to its first subsection, or
to its end when it has none, and leaves the subsections in place; its body is
one heading at the section's level, which may rename the section, followed by
text with no further heading, and a body of the heading alone is accepted. A
later edit names a renamed section by its new title. ``Delete:`` removes the
section with its subsections.

Staging is each maintained document with the edits of every accepted record's
changeset applied in dependency order. Changesets of draft and proposed
records are checked, not applied. Staging is only ever generated: a staging
document no changeset produces is reported (P008) and removed by the next
generation. A document that belongs to no family is read when an edit names
it and is not a palette document: no document rule applies to it or to its
mirror, and only its edits (P007) and the links in them (P004) are checked. A
``Changes`` entry keeps its meaning whether or not a changeset edits the
document it names.

The text of an edit reads as the maintained document will read, so a link in
an edit body is relative to the edit's target document (for ``Create:``, the
new document's path), not to the changeset file, and it holds unchanged in
staging and after promotion. Such a link may point at a document or a section
that the same record's edits, or the changesets of the record's ``Depends``
closure, create. Text outside an edit body is read from the changeset file's
own location.

Contract
--------

Lint rules
~~~~~~~~~~

Each finding has a rule id, a severity (``error`` or ``warning``), a file, a
line and a message.

``P001`` RST subset (error)
  A table, a directive other than a comment, an overlined title, an underline
  shorter than its title, a heading level out of the house-style order, a
  substitution, a footnote or a citation. ``.. list-table::`` and
  ``.. code-block::`` are not reported in RFC, ADR, changeset, staging,
  design, spec, principles, glossary and contributing documents; ``P017``
  checks them there. The mirror of a document outside the families is not
  checked by either.

``P002`` Structure (error)
  A missing, extra or out-of-order required section; a missing or out-of-order
  required field; a value outside the field's allowed values. The optional
  fields are ``Amends`` on a record (absent or in its place after
  ``Related``) and ``Acceptance`` in the contributing document's ``Records``
  section (absent or in its place after ``Amends``); a field the template
  places in a section, found in the header or another section, is reported.

``P003`` Identity (error)
  File name, number and title disagree; a file name that is not lowercase
  kebab-case; a duplicate number.

``P004`` Links (error)
  A link whose target file or anchor does not exist; a link from a project path
  into ``_palette/``. Links in table cells are checked like any other; a link
  whose text and target wrap over lines within one block is one link; a link
  in a changeset's edit body is read from the edit's target document, whatever
  family or none it belongs to, against the documents as the record's edits
  and its ``Depends`` closure leave them.

``P005`` Relations
  A link to a record created later (error); a relation cycle (error); a
  ``Depends`` entry reachable through another listed entry whose parenthetical
  does not state a direct use (warning); the same target in ``Depends`` and
  ``Related`` (warning); a record named in a whole ``Supersedes`` entry whose
  status is not ``Superseded``, or a record whose status is ``Superseded``
  while no newer record names it in a whole ``Supersedes`` entry (error);
  ``Amends`` on a record whose ``Date`` is later than the contributing
  document's cutoff, or when that document admits none (error).

``P006`` Changes (error)
  A ``Changes`` entry names a document the record changes, in one of three
  forms. ``design/<topic>.rst`` or ``spec/<topic>.rst`` with its sections, or
  ``created``: an error when the document or a section neither exists, nor is
  created by the record's own changeset, nor exists once the changesets of the
  record's ``Depends`` closure are applied; ``created`` needs the record's own
  changeset to create it. ``principles.rst``, ``glossary.rst`` or
  ``contributing.rst`` with its sections: an error when the family's document
  is not on disk, when a section neither exists, nor is added by the record's
  own changeset, nor exists once the changesets of the record's ``Depends``
  closure are applied, or when the entry says ``created`` (only design and
  spec documents are created by a changeset). Any other project document, as
  a project-relative path to a ``.md`` or ``.rst`` file with what changes in
  the parenthetical: an error when the file does not exist, the path is
  absolute, has ``..`` or a backslash, lies inside ``_palette/``, names a
  source file, names a record or another palette document, or names a family
  document by its real path instead of its logical name. The parenthetical of
  such an entry is free text; whether a changeset may edit the document is
  decided when a changeset names it, not here.

``P007`` Changesets (error)
  A target a changeset may not edit (the rule the target fails is named); an
  edit that does not resolve against its target with only its record's
  dependency closure applied; a ``Replace body:`` whose body has no heading, a
  heading at another level, or more than one heading; a ``Create:`` of a
  document outside design and spec; two changesets editing one section with no
  dependency between them; a changeset with no record; a record whose
  ``Implementation`` is ``complete`` while its changeset still has edits.

``P008`` Generated files (error)
  A staging document or an index that differs from what the server would
  generate; a staging document no changeset produces, anywhere under the
  staging folder.

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
  project, or a project path inside ``_palette/``; a staging placement that
  contains another family's placement, or a family placed inside the staging
  folder, or placed at the same path as another family, in which case the
  staging placement is dropped and every write tool and generation refuse
  until the layout is corrected; a missing ``checker``. ``checker`` is a
  setting, not a family.

``P013`` Backlog (error)
  A duplicate item id; ``in-phase-<N>`` without a phase ``<N>`` file; a
  deliverable link to a missing file; a ``done`` item whose ``Outcome`` is
  ``none`` (warning).

``P014`` Time-varying fields (error)
  ``Implementation``, ``Verification``, ``Implementers`` or ``Revised`` not in
  its template format.

``P015`` Stray files (error)
  A file under ``_palette/`` other than ``layout.rst``, ``.gitignore``,
  ``.palette.lock`` and the documents the layout places; a file or folder
  inside a placed family folder that is not a document of that family, the
  staging folder's mirrors excepted. Research and evidence belong inside the
  record that relies on them; a decision or question in state; anything else
  outside the palette folders.

``P016`` Acceptance
  An ``Accepted`` value that names who accepted the record but not the date
  and time: an error when the contributing document's ``Records`` section
  states ``:Acceptance: who-and-time``, nothing when it states
  ``:Acceptance: who``, a warning when the field, the section or the document
  is absent.

``P017`` Directive structure (error)
  In a family that admits them, a ``list-table`` or ``code-block`` that
  docutils would reject or read differently. A ``list-table`` takes an
  optional title argument and only the options ``:header-rows:`` (a
  non-negative integer) and ``:widths:`` (``auto`` or positive integers, one
  per column), each at most once, directly after the directive line; a blank
  line, then a body that is one two-level list starting at the options'
  column: rows start with ``* -`` at the body's indent, further cells with
  ``-`` two columns deeper, one space separates a marker from the cell text,
  and cell content continues four columns deeper or more. Every row has as
  many cells as the first; ``:header-rows:`` greater than zero is less than
  the number of rows. A ``code-block`` takes exactly one language argument
  (one token of letters, digits and ``_ + . # -``) and no option; a blank
  line, then non-blank content indented deeper than the directive. Neither is
  indented with tabs. Each cell's content is scanned like the rest of the
  document, on its own; code-block content is not checked.

Read tools
~~~~~~~~~~

Read tools change nothing and carry the MCP read-only annotation.
``palette --read-only-tools`` prints their names, one per line.

``palette_lint``
  Input: ``project``; optional ``paths``. Output: the findings, errors first.

``palette_status``
  Input: ``project``; optional ``record``. Without ``record``: every phase
  with its status, item counts by status, open questions, discrepancies,
  decisions not yet graduated, lint error count. With ``record``: its header,
  incoming links, dependency closure, what supersedes it (with the part when
  the supersession is partial), what amends it, its pending changeset edits
  and the staging documents they affect. The output stays under 4,000
  characters and says what it omitted.

``palette_layout``
  Input: ``project``. Output: every family with its resolved absolute path
  and the settings.

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
  from any status; ``in-phase-<N>`` requires phase ``<N>`` to be active, and
  an item in one phase does not move to another.

``palette_phase_open``
  Creates ``phase-<N>/phase.rst`` from goal, reason, assumptions and exit
  criteria; adds the phase to the backlog as ``active``; moves the given items
  to ``in-phase-<N>``. Several phases may be active at once.

``palette_deliverable_create`` and ``palette_deliverable_update``
  Write a deliverable for one backlog item in an active phase and link it
  from the item.

``palette_phase_close``
  Takes, for every item in the phase, ``done`` with an outcome pointer or
  ``dropped``, plus new proposed items. Updates the backlog, marks the phase
  ``closed`` and removes the state pointers of graduated decisions. The phase
  and deliverable files stay as the record of what was approved.

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
  fields to their initial values. A ``supersedes`` entry takes ``partial:
  true`` to replace a part; only a whole entry sets the older record to
  ``Superseded``.

``palette_record_update``
  Changes a record. A draft or proposed record may change freely. An accepted
  record's body changes only with ``clarification: true`` (a mechanical
  correction), which adds a ``Revised`` entry. Status moves ``Draft`` →
  ``Proposed`` → ``Accepted`` (requires ``accepted_by`` and writes
  ``Accepted`` as ``<who> (<YYYY-MM-DDTHH:MMZ>)`` with the current UTC time),
  or to ``Rejected`` or ``Withdrawn``; ``Superseded`` is set only through a
  newer record's whole ``Supersedes`` entry. ``supersedes`` entries take
  ``partial`` as in ``palette_record_create``.

``palette_record_refresh``
  States that an Accepted record's change has landed outside a changeset and
  brings its derived fields in line. Input: ``record``, optional
  ``implementation`` (``partial``, ``in-progress``, ``abandoned`` or
  ``not-applicable``; ``complete`` is the default), ``implementation_scope``,
  ``implementers``. Without a given value: no changeset or no edit remaining
  sets ``Implementation`` to ``complete``; edits remaining with the current
  token ``partial`` keeps ``partial``; edits remaining with any other token
  is refused, naming ``palette_changeset_promote``. A given value is written
  as given whether or not edits remain; ``complete`` with edits remaining is
  refused. It sets the status to ``Superseded`` when a newer record's whole
  ``Supersedes`` entry names the record. It never writes ``Revised``,
  ``Verification`` or ``Accepted``; a Draft or Proposed record is refused.

``palette_changeset_edit``
  Adds, replaces or removes one edit in a record's changeset. ``kind`` is
  ``replace``, ``replace_body``, ``insert_after``, ``insert_into``, ``delete``
  or ``create``; ``document`` is a target as the changeset section writes it;
  a target the changeset may not edit is refused with the rule it fails.

``palette_changeset_promote``
  Merges implemented edits of a record into their maintained documents, removes
  them from the changeset, deletes an empty changeset, and sets the record's
  ``Implementation`` (``partial``, or ``complete`` when no edit remains),
  ``Implementers`` and ``Verification`` from the given values. ``Create:`` is
  promoted for design and spec documents only.

``palette_generate``
  Regenerates every index and staging document that is missing or differs
  from what the server would generate, as ``palette generate`` does, and
  returns the diff; with nothing to regenerate it returns an empty file list
  and says ``nothing to generate``.

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
``Implementers``, ``Revised``, generated indexes and staging. Promotion writes
``Implementation``, ``Implementers`` and ``Verification`` as edits land;
``palette_record_refresh`` writes ``Implementation``, ``Implementers`` and the
``Superseded`` status from the documents as they stand, for a record whose
change lands outside a changeset; ``palette_generate`` rewrites the indexes
and staging that drifted. Any other text in a palette document may also be
edited by hand; the next lint reports what drifted.

Compatibility
-------------

Tool names and input fields are stable within one contract number of the kit
(the descriptor's ``contract``). A new optional input field is compatible; a
removed or renamed one is not.

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
