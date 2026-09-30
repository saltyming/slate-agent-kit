RFC-0013: Changes names any project document
============================================

:Status: Accepted
:Implementation: complete — the rfc and adr templates, the palette server's
  Changes parser, P006 and the record tools, the palette-record skill
:Verification: build — 2026-09-30; cargo test on macOS (542 tests in the
  workspace) and palette check on this repository; Linux and Windows through
  slate CI on the next push
:Areas: palette; templates; MCP servers
:Authors: Claude Fable 5.1
:Reviewers: Hamin Sung
:Implementers: Claude Fable 5.1 with a Claude Opus subagent (2026-09-30)
:Accepted: Hamin Sung (2026-09-30T09:40Z)
:Date: 2026-09-30
:Revised: none
:Depends: RFC-0003 (direct use of the record header and the families it
  defines); RFC-0004 (direct use of P006 and the record tools)
:Supersedes: RFC-0003 (in part: Changes names only design and spec documents)
:Related: RFC-0007 (the check without a layout, which the new forms survive)
:Changes: spec/palette-server.rst (Changesets and staging; Lint rules)
:Description: A record's Changes field may name the single-file families and
  any other project document beside design and spec documents, so that what a
  decision changes stays in the header.

Summary
-------

``Changes`` records which documents a decision changes. It accepted only
``design/<topic>.rst`` and ``spec/<topic>.rst``. It now takes three forms:
those two with their sections or ``created``; ``principles.rst``,
``glossary.rst`` and ``contributing.rst`` with their sections; and any other
project document by its project-relative path, a ``.md`` or ``.rst`` file,
with a free-text parenthetical saying what changes. Changesets and staging
stay limited to design and spec documents. In the same change P006 accepts a
document or section that the record's ``Depends`` closure creates, as P007
already did, and a link in a changeset's edit body is read from the edit's
target document, as the changeset template says.

Problem and context
-------------------

A decision often changes documents outside the design and spec families: the
principles, the contributing rules, a repository's ``AGENTS.md`` or build
notes. With ``Changes`` limited to two families, a record either left those
out of its header or moved them to prose in its references, where nothing
checks them. A project moving 124 RFCs into palette had four such entries
(``AGENTS.md``, ``BUILDING.md``, ``TOOLCHAIN.md``, ``principles.rst``).

Two defects showed in the same migration. P006 accepted a ``Changes`` target
only when it existed or the record's own changeset created it, while P007
resolves edits against the record's dependency closure, so a record naming a
document an earlier, not yet promoted record creates was an error. And P004
read a link in an edit body from the changeset file, although the changeset
template tells authors to write links relative to the target document; 66
links written as the template says were reported.

Evidence
--------

- saltyos, counted in its session on 2026-09-30: RFC-0112 lists ``AGENTS.md``,
  ``BUILDING.md`` and ``TOOLCHAIN.md`` and RFC-0114 lists ``principles.rst``
  in ``Changes``; ``spec/hierarchy.rst`` is created by RFC-0106 and named by
  RFC-0111 and RFC-0114, ``spec/nt-service.rst`` by RFC-0109 and named by three
  later records; 66 changeset links are target-relative.
- Reproduced on the fixture with palette 0.3.1 before the fix: a changeset
  editing ``spec/thing.rst`` with the link ``<thing.rst>`` gave P004; the code
  of P006 looked only at the record's own changeset (``lint/relations.rs``).
- ``shared/workflows/palette/templates/changeset.rst`` says "links relative to
  the target document"; ``spec/palette-server.rst`` says P007 resolves an edit
  "with only its record's dependency closure applied".

Goals and non-goals
-------------------

Goals:

- What a decision changes is stated in its header for every kind of document
  and checked as far as the server can check it.
- ``Changes`` stays a list of documents, never a list of source files.
- P004, P006 and P007 judge a record against one document state.

Non-goals:

- Changesets or staging for documents outside design and spec.
- Checking sections of a document the server does not parse as a palette
  document.

Requirements and invariants
---------------------------

- A design or spec entry keeps its meaning, and ``created`` still needs the
  record's own changeset.
- A single-file family is named by its logical name and found through the
  layout, or the inferred layout of a checkout without ``_palette/``.
- Another project document is a relative path to an existing ``.md`` or
  ``.rst`` file outside ``_palette/``; a family document written by its real
  path, a record, a generated or internal document and a source file are
  errors.
- A link in an edit body resolves the same way in the changeset, in staging
  and after promotion.

Design
------

The contract is the P004 and P006 paragraphs and the "Changesets and staging"
section of ``spec/palette-server.rst``. The template's ``Changes`` line reads
``none | <document> (<section or what changes>; <section or what changes>)``.
The server computes, once per check, the documents as each record's
``Depends`` closure leaves them and as its own edits leave them; P006 reads
the first, links in edit bodies the second, P007 the failures of the second.

Impact and compatibility
------------------------

- Every ``Changes`` value valid before stays valid.
- A record that named a document created by a dependency, or wrote edit-body
  links as the template says, stops being reported.
- A changeset that wrote edit-body links relative to the changeset folder is
  now reported by P004 unless the two locations coincide.

Implementation and transition
-----------------------------

One change to the palette server and the two record templates; it ships in
palette 0.3.1 with the kits' next patch.

Verification strategy
---------------------

- Synthetic-fixture tests per form and per refusal, with and without a
  layout, on LF and CRLF.
- The three rules on a two-record fixture where the older record creates a
  document the newer one names and links to.
- ``palette check`` on this repository stays clean.

Alternatives and costs
----------------------

- Move such entries to prose in References: nothing checks them and the
  header no longer says what the decision changes.
- Accept any path: the field turns into a list of files to edit.
- Extend changesets and staging to every document: a second copy of documents
  the project edits directly.

Open questions
--------------

None.

References
----------

- `RFC-0003 <rfc-0003-palette-document-system.rst>`_
- `RFC-0004 <rfc-0004-palette-server.rst>`_
- `RFC-0007 <rfc-0007-checking-without-the-layout.rst>`_
- ``shared/workflows/palette/templates/{rfc,adr,changeset}.rst``
