RFC-0021: Generation and derived header fields on demand
========================================================

:Status: Accepted
:Implementation: complete — two write tools in the palette server, the spec's
  Write tools and Ownership and ordering sections, the palette rule's tool list
:Verification: build — 2026-10-13; 2026-10-13, cargo test on macOS (251 tests in
  the palette crate), clippy and fmt on stable 1.99.0, ADR-0001 refreshed and
  palette_generate answered nothing to generate in a scratch copy of this
  repository, Linux and Windows through slate CI on the next push
:Areas: palette; MCP servers
:Authors: Claude Fable 5.1
:Reviewers: Hamin Sung
:Implementers: Claude Opus 5.5 subagent (server and tests) and Claude Fable 5.1
  (spec, palette rule) (2026-10-13)
:Accepted: Hamin Sung (2026-10-13T13:38Z)
:Date: 2026-10-13
:Revised: 2026-10-13 — a given implementation value applies whether or not edits
  remain, "some edit promoted" means the current token is partial
:Depends: RFC-0004 (direct use of the write transaction, the generated files and
  the server-owned header fields); RFC-0019 (the Write tools section as its
  changeset rewrites it, which this record's changeset edits in turn)
:Supersedes: RFC-0004 (in part: regeneration only from the command line, and
  Implementation written only by promotion)
:Related: RFC-0013 (a record whose Changes names only a .md document has no
  changeset and therefore no promotion, which is the case the refresh tool
  serves)
:Changes: spec/palette-server.rst (Write tools; Ownership and ordering);
  shared/workflows/palette/rules.md (the write tools the rule names)
:Description: A palette_generate tool runs the command line's regeneration from
  a session, and a palette_record_refresh tool recomputes a record's derived
  header fields (Implementation, Superseded) from the documents as they stand.

Summary
-------

Regeneration of indexes and staging exists only as ``palette generate`` on the command line, and the server-owned header fields are written only by ``palette_record_create`` (initial values), ``palette_record_update`` (``Revised`` on a clarification) and ``palette_changeset_promote`` (``Implementation``, ``Implementers``, ``Verification``). A session with the MCP server has to leave it to regenerate, and a record that has no changeset, because its ``Changes`` names only documents a changeset cannot edit, keeps ``Implementation: not-started`` for ever. Two write tools close the gap: ``palette_generate`` runs the same transaction as the command line and returns the diff; ``palette_record_refresh`` is the call an agent makes when a record's change has landed outside a changeset: it sets ``Implementation`` to ``complete`` when no edit remains in the record's changeset, to ``partial`` when some edits have been promoted and some remain, and refuses when edits remain and none has been promoted (promotion is the way then); a non-complete value (``partial``, ``in-progress``, ``abandoned``, ``not-applicable``) may be given for the exceptions. It sets ``Superseded`` from the whole ``Supersedes`` entries of newer records and never writes ``Revised``, ``Verification`` or ``Accepted``, which record events, not derived state.

Problem and context
-------------------

ADR-0001 names ``shared/rules/constitution.md`` in ``Changes`` and nothing else, so it has no changeset and ``changeset_promote`` never runs for it; its ``Implementation`` stays ``not-started`` after the change has shipped, and the only way to correct it is a hand edit of a server-owned field, which the next lint reports as drift. The same holds for every record whose change lives in source or in ``.md`` documents. Separately, ``palette generate`` repairs drift in staging and indexes after a hand edit, but the server exposes no tool for it, so a session has to run the binary from a shell.

Evidence
--------

- ``docs/adr/adr-0001-2-names-the-data-channel-and-the-corpus-ceilings-rise-by-2-500-bytes.rst``: ``Changes: shared/rules/constitution.md (...)``, ``Implementation: not-started``, no changeset; the change shipped on 2026-10-13.
- ``shared/mcp-servers/palette/src/ops/records.rs`` (``//! ... are written only here``): ``Revised`` at the clarification path; ``Implementation``, ``Implementers``, ``Verification`` in ``changeset_promote``; read on 2026-10-13.
- ``shared/mcp-servers/palette/src/cli.rs`` ``generate()`` and ``ops::generate``: the regeneration transaction; ``server.rs`` lists no tool for it.
- ``docs/spec/palette-server.rst`` *Ownership and ordering*: the server is the only writer of the four fields, the indexes and staging.

Goals and non-goals
-------------------

Goals:

- A session regenerates indexes and staging through the server, in the same transaction the command line runs.
- A record's ``Implementation`` and ``Superseded`` can be brought in line with the documents without a hand edit of a server-owned field.

Non-goals:

- Recomputing ``Revised``, ``Verification``, ``Implementers`` or ``Accepted``.
- Changing what the lint reports about hand-edited text.
- Refreshing inside other write tools beyond what they already regenerate.

Requirements and invariants
---------------------------

- ``palette_generate`` takes ``project`` and ``dry_run``; it regenerates every index and staging document under the project lock as ``palette generate`` does, returns the diff and the file list, and returns an empty file list with the message ``nothing to generate`` when nothing differs. It is a write tool and is not pre-approved.
- ``palette_record_refresh`` takes ``project``, ``record``, optional ``implementation`` (``partial``, ``in-progress``, ``abandoned`` or ``not-applicable``; ``complete`` is the default and need not be given), optional ``implementation_scope``, optional ``implementers`` and ``dry_run``. The call states that the record's change has landed. Without a given value: no changeset or no edit remaining sets ``Implementation`` to ``complete``; edits remaining with the current token ``partial`` (the sign that promotion has run) keeps ``partial``; edits remaining with any other token is refused, naming ``palette_changeset_promote``. A given value is written as given whether or not edits remain; ``complete`` with edits remaining is refused. It sets the record's status to ``Superseded`` when a newer record names it in a whole ``Supersedes`` entry. It writes ``Implementers`` only when given. It never writes ``Revised``, ``Verification`` or ``Accepted``; a Draft or Proposed record is refused; an unknown record is ``not_found``.
- Both tools regenerate the indexes they affect and lint the files they touch, as every write tool does.

Design
------

``server.rs`` gains the two tools; ``ops/mod.rs`` exposes ``generate`` as a tool handler returning ``WriteResult``; ``ops/records.rs`` gains ``record_refresh`` beside ``changeset_promote``, reusing the field writers and the analysis's ``superseded_by`` computation. The palette rule's list of write tools names both; the tool descriptions follow the existing form.

Impact and compatibility
------------------------

- No existing tool changes its input or output.
- ADR-0001 and every record without a changeset can state ``Implementation: complete`` through the server after their change ships.
- ``palette_generate`` makes the command line optional for a session; the command line stays for CI and for a checkout without a session.

Implementation and transition
-----------------------------

One change to the palette server, its spec and the palette rule; it ships with slate v26.10.1. The server code is implemented after RFC-0019's change lands in the same crate, by one writer at a time.

Verification strategy
---------------------

- ``palette_generate`` on a project with drifted staging and on one with none (``nothing to generate``), dry run and real, LF and CRLF.
- ``palette_record_refresh``: a record without a changeset becomes ``complete`` with no argument and ``abandoned`` with the argument; a record with remaining edits and nothing promoted is refused; one with a partial promotion becomes ``partial``; a record superseded by a newer whole entry; a Draft record refused; ``Revised``, ``Verification`` and ``Accepted`` byte-identical before and after.
- ``palette check .`` on this repository after refreshing ADR-0001.

Alternatives and costs
----------------------

- Allow hand edits of the four fields: the lint would have to stop reporting drift on them, and the fields would stop meaning what the server computed.
- Refresh automatically inside every write tool: a status change in one record would rewrite others' headers in the same transaction, which the diff would then carry for every operation.
- Keep the command line only: a session cannot regenerate without a shell, and nothing at all corrects a changeset-less record.

Open questions
--------------

None.

References
----------

- `RFC-0004 <rfc-0004-palette-server.rst>`_
- `RFC-0019 <rfc-0019-changesets-under-the-document-root.rst>`_
- ``shared/mcp-servers/palette/src/{cli.rs,ops/mod.rs,ops/records.rs}``
