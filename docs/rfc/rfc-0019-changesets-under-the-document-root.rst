RFC-0019: Changesets edit any document under the document root, and Replace body: keeps subsections
===================================================================================================

:Status: Accepted
:Implementation: complete — the palette server (targets, the document root as
  the staging folder's parent, non-family mirrors by path, Replace body:, lint
  and regeneration), the changeset template, the spec, the palette-record skill
:Verification: build — 2026-10-13; 2026-10-13, cargo test on macOS (240 tests in
  the palette crate), clippy and fmt on stable 1.99.0, saltyos and SaltyMail
  check and generate output identical before and after, with and without a
  layout, in scratch copies, Linux and Windows through slate CI on the next push
:Areas: palette; templates; MCP servers
:Authors: Claude Fable 5.1
:Reviewers: Hamin Sung
:Implementers: Claude Opus 5.5 subagent (server and tests) and Claude Fable 5.1
  (template, spec, skill) (2026-10-13)
:Accepted: Hamin Sung (2026-10-13T12:42Z)
:Date: 2026-10-13
:Revised: 2026-10-13 — a dropped staging placement refuses every write, a
  target's spelling must match the on-disk path, no document rule runs on a
  non-family mirror
:Depends: RFC-0004 (direct use of the server's changeset tools, lint rules and
  generated staging); RFC-0013 (the three forms of a Changes entry, which a
  changeset target now shares); RFC-0007 (the inference of families without a
  layout, which the document root follows)
:Supersedes: RFC-0013 (in part: changesets and staging limited to design and
  spec documents); RFC-0004 (in part: the edit kinds and the design-and-spec
  limit of changesets and staging)
:Related: RFC-0003 (the changeset and staging families as first defined)
:Changes: spec/palette-server.rst (Families and locations; Changesets and
  staging; Lint rules; Write tools);
  shared/workflows/palette/templates/changeset.rst (the Replace body: edit, the
  target forms, and the note that Replace: replaces subsections);
  shared/workflows/palette/skills/palette-record/SKILL.md (the changeset targets
  and the two Replace kinds)
:Description: A changeset may edit every .rst document under the project's
  document root, not only design and spec documents, and a new Replace body:
  edit changes a section's body while leaving its subsections in place.

Summary
-------

A changeset edits design and spec documents only (``CHANGESET_TARGETS``),
although records in two projects change other documents by section. A changeset
may now target every maintained family document wherever it is placed (design
and spec by logical path; ``principles.rst``, ``glossary.rst`` and
``contributing.rst`` by logical name) and any other ``.rst`` document under the
project's document root by its project-relative path, the same three spellings
a ``Changes`` entry takes (RFC-0013). The document root is the parent folder of
the staging placement. A ``.md`` document stays a ``Changes`` name; records,
indexes, changesets, staging and work documents are never targets. Family
documents keep today's mirror paths; a non-family document is mirrored at the
staging folder plus its path relative to the document root. ``Replace:`` keeps
replacing a section together with its subsections and now says so; a new
``Replace body:`` edit replaces a section's heading and body up to its first
subsection and leaves the subsections in place.

Problem and context
-------------------

Two limits of the changeset mechanism showed in use.

Records change documents outside design and spec by section. saltyos ADR-0033
changes ``docs/coding-style.rst`` (*Architecture conditions*) and SaltyMail
records change ``docs/documentation-policy.rst`` by section. RFC-0013 let
``Changes`` name them but left changesets and staging limited to design and
spec, so those edits are written straight into the document before the
implementation lands, outside the record's changeset, with no staging and no
P007 check.

``Replace:`` replaces the whole extent of a section. ``apply_edit`` splices
``start..end`` and ``Heading.end`` is the exclusive end including subsections,
so a ``Replace:`` written for a parent section's body removes every subsection
unless the body restates them. The test
``replace_changes_only_the_named_section`` fixes this behavior; the changeset
template, the tool description and the spec do not mention it. A changeset
written from the template drops subsections silently.

Evidence
--------

- ``shared/mcp-servers/palette/src/layout.rs`` ``CHANGESET_TARGETS`` and
  ``split_logical``; ``changeset.rs`` ``apply_edit``
  (``target.splice(t.start..t.end, ...)``) and ``rst.rs`` ``Heading.end``
  ("Exclusive end of the section including subsections"), read on 2026-10-13.
- ``replace_changes_only_the_named_section`` in ``changeset.rs`` asserts that
  the subsection ``Sub`` of the replaced section is absent from the result.
- saltyos
  ``docs/adrs/adr-0033-entry-proof-and-architecture-selected-options.rst``
  ``Changes: docs/coding-style.rst (Architecture conditions ...)``; SaltyMail
  records name ``docs/documentation-policy.rst`` with sections
  (``Purpose and authority``, ``Document roles``); both read on 2026-10-13.
- RFC-0013, *Goals and non-goals*: "Changesets or staging for documents outside
  design and spec" is a non-goal; *Alternatives and costs*: extending them to
  every document is "a second copy of documents the project edits directly".
- D-1 and D-4 in the project state, 2026-10-13: keep ``Replace:`` and add a
  body-only kind; widen the targets to the document root and no further.

Goals and non-goals
-------------------

Goals:

- Every document a record changes by section can be edited through its
  changeset, staged and promoted, with the same checks design and spec
  documents get.
- A ``Replace:`` edit says what it replaces, and an author who wants to change
  only a body has an edit kind that does that.
- Existing changesets regenerate the same staging as before, with and without a
  layout.

Non-goals:

- Editing ``.md`` documents through changesets: the server parses RST sections
  only.
- A changeset target among records, indexes, changesets, staging documents or
  work documents (backlog, state, phase, deliverable).
- Creating a document outside design and spec through ``Create:``.
- Changing the meaning of ``Replace:``, ``Insert after:``, ``Insert into:`` or
  ``Delete:``, or of a ``Changes`` entry.
- Linting non-family documents: they are read as edit targets, not as palette
  documents.
- The pre-existing P007 limit that ``Child`` and ``Parent / Child`` naming one
  section, or a parent edit and a child edit, are not seen as a conflict.

Requirements and invariants
---------------------------

- The document root is the parent folder of the staging family's placement,
  from the layout or from the inference of RFC-0007; a project whose staging is
  internal has ``_palette/`` as its root and therefore no non-family target. No
  new layout setting is added.
- A target is written in one of the three ``Changes`` spellings. A family
  spelling (``design/<topic>.rst``, ``spec/<topic>.rst``, ``principles.rst``,
  ``glossary.rst``, ``contributing.rst``) resolves through the family wherever
  it is placed, internal included, and keeps the kebab-case ``.rst`` rule the
  parser applies today. A project-relative spelling is accepted only for an
  existing ``.rst`` file under the document root that is outside every family
  folder, the staging folder and ``_palette/``, and only when each path
  component is spelled as the directory entry on disk is spelled, so that one
  file has one spelling on every filesystem; a family document written by its
  real path is refused as it is in ``Changes``.
- ``Create:`` is accepted for design and spec documents only, in
  ``apply_changeset`` and in ``changeset_promote`` alike.
- The mirror of a family document is unchanged:
  ``<staging>/<family key>/<name>`` for design and spec,
  ``<staging>/<family key>.rst`` for a single-file family. The mirror of a
  non-family document is
  ``<staging>/<its path relative to the document root>``; a target whose mirror
  would land in the ``design`` or ``spec`` mirror folder or equal a single-file
  mirror is refused.
- Every ``.rst`` under the staging folder is a staging document: the snapshot
  assigns the staging role by location before any classification, P008 reports
  a staging document no changeset produces, and regeneration removes it.
  Without a layout, the staging folder is the folder named ``staging`` that
  holds a ``design`` or ``spec`` subfolder or lies beside an inferred record,
  design or spec folder; its contents are never classified.
- P012 refuses a staging placement that is an ancestor of, or equal to, another
  family's placement and a family placement inside the staging folder; in that
  state the staging placement is dropped and every write tool and generation
  refuse with the P012 finding before writing anything, so no family document
  is ever read as staging.
- A non-family document is not a snapshot member: it is read when an edit,
  staging or promotion names it; P007 reports an edit that does not resolve in
  it and P004 resolves links in the edit body relative to it; no document rule
  (P001, P002, P003, P016, P017) applies to it or to its mirror.
- A ``Changes`` entry keeps its meaning: a project-document parenthetical stays
  free text, a ``.md`` or an out-of-root document stays a valid entry, and
  eligibility as a changeset target is checked only when a changeset names the
  document.
- ``Replace body:`` replaces the lines from the section's heading to the start
  of its first subsection, or to its end when it has none. Its body is exactly
  one heading at the target's level, which may rename the section, followed by
  text with no further heading; a body of the heading alone is accepted and
  leaves one blank line before the retained subsections or the next sibling. A
  later edit names the section by its new title. ``Replace:`` replaces the
  section and its subsections. P007's conflict rule, keyed by document and
  target, applies to ``Replace body:`` as to every kind.
- One resolution of a target is used by the parser, ``apply_changeset``, source
  lookup, ``held_at``, ``changeset_promote``, the staging plan and P004, so
  that lint, generation and promotion never disagree about one target; a
  refused target is never applied.
- A refusal names the rule: ``.md`` documents are named in ``Changes`` and
  edited directly; records, indexes, changesets, staging and work documents are
  generated or frozen; a document outside the document root (the staging
  folder's parent) is not maintained by the project's documents; ``Create:``
  makes design and spec documents only.

Design
------

The contract is the *Families and locations*, *Changesets and staging* and
*Lint rules* sections of ``spec/palette-server.rst`` as the changeset of this
record writes them, and the changeset template.

Targets. ``layout.rs`` gains
``resolve_target(layout, spelling) -> Result<Target, String>`` where ``Target``
is ``Family(Family, name)`` or ``Document(PathBuf)`` with its maintained file
and its mirror; it reuses ``changes_doc`` for the spelling and adds the
eligibility checks above. ``Changeset::parse``, ``apply_changeset``,
``maintained_source``, ``held_at``, ``changeset_edit``, ``changeset_promote``,
``plan`` and P004's target-relative link base call it; ``split_logical``
remains for the family forms.

Document root. ``Locations::document_root()`` is the parent of
``dir_of(Family::Staging)``; the inferred layout of a checkout without
``_palette/`` yields the same because it places staging where it found it.

Staging. ``staging_file(target)`` keeps the family rule and adds the
relative-path rule for ``Target::Document``. ``Snapshot::load`` lists every
``.rst`` under the staging folder recursively as ``Role::Staging`` before the
family loop; ``lint/stray.rs`` accepts the nested mirrors;
``ops/mod.rs::regenerate`` keeps deleting staging documents the plan does not
produce.

Non-family documents. ``Snapshot`` gains a side map of referenced documents
loaded on demand (``referenced(path)``), outside ``files`` so that no document
rule runs on them.

Replace body. ``EditKind::ReplaceBody`` (template line
``Replace body: <section title>``, tool value ``replace_body``) splices
``t.start..t.body_end``; ``converted_body`` is checked to contain one heading
at the target level and none below it.

Messages. ``CHANGESET_TARGETS`` states the new rule; each refusal says which
part of it the target fails.

Impact and compatibility
------------------------

- Every changeset valid today stays valid and regenerates byte-identical
  staging, with and without a layout: family mirror paths do not change, and
  the inference of staging is unchanged for a staging folder that holds
  ``design`` or ``spec``.
- ``palette generate`` in saltyos and SaltyMail writes nothing before the first
  changeset that uses a new target (saltyos ``docs/staging`` holds only
  ``design`` and ``spec``; SaltyMail has no staging folder yet).
- A ``.rst`` lying under a staging folder outside ``design`` and ``spec`` was a
  stray file; it is now a staging document that P008 reports and the next
  ``generate`` removes. Neither live project has one.
- Internal design and spec changesets keep working; the work families remain
  outside every target rule.
- The changeset template gains one edit kind; a changeset that uses it, or a
  non-family target, is read by palette 26.10.1 and later only.

Implementation and transition
-----------------------------

One change to the palette server, the changeset template, the spec and the
palette-record skill; it ships with slate v26.10.1 and the kits' 26.10.1. The
server code and its tests (``layout.rs``, ``changeset.rs``, ``docs.rs``,
``ops/records.rs``, ``ops/mod.rs``, ``ops/init.rs`` where it rewrites links on
a family move, ``lint/{changes,relations,links,stray}.rs``, ``generated.rs``,
the tool descriptions in ``server.rs``) are implemented by a delegate from this
record; the template, the spec changeset and the skill text are written with
it.

Verification strategy
---------------------

- Synthetic fixtures with LF and CRLF: a changeset edits ``principles.rst``,
  ``contributing.rst``, an internal ``design/`` document and a non-family
  ``.rst`` under the document root through add, staging and promote; a ``.md``,
  a work document, a record, an index, a staging document, a document outside
  the root and a family document by its real path are refused with the message
  naming the rule; ``Create:`` on a non-design target is refused in both
  execution paths.
- ``Replace body:`` on a section with two subsections keeps both, a
  heading-only body keeps the separation, a body with a second heading is
  refused; ``Replace:`` on the same section removes the subsections; a renamed
  title is applied and a later ``Insert into:`` by the new name appends after
  the retained subsections.
- Document root and staging: staging placed at ``docs/staging`` and internal;
  staging beside the families without ``design``/``spec`` subfolders; no
  layout; a staging placement that contains a family placement is refused by
  P012.
- saltyos and SaltyMail: ``palette check`` findings and ``palette generate``
  output compared before and after the change, with and without the layout, and
  regeneration run twice for stability.
- Tests assert path components; ``cargo test`` and ``clippy -D warnings`` on
  the latest stable.

Alternatives and costs
----------------------

- Keep the design-and-spec limit and edit other documents directly: the state
  RFC-0013 chose; the edits a record makes to those documents are then
  unchecked and unstaged, which is the problem.
- A document root inferred as the common ancestor of the family placements: a
  single-file family at the repository root widens it to the repository, and a
  layout setting that narrows it is invisible to a checkout without
  ``_palette/`` (RFC-0007). The staging folder's parent is one rule that both
  sides compute.
- Any ``.rst`` in the repository: no boundary at all; source-adjacent RST
  becomes a changeset target by accident.
- Mirror every document by its path relative to the root, families included:
  changes the mirror of a design family placed at ``docs/architecture`` from
  ``staging/design/`` to ``staging/architecture/``, which breaks the
  compatibility promise.
- Load non-family documents into the snapshot: P001, P003 and P004 would start
  reporting documents the project never offered to the lint.
- Change ``Replace:`` to body-only: every existing changeset whose body
  restates subsections would duplicate them.
- Mark the Replace semantics in documentation only: the author still has no way
  to change a body without restating the subsections.

Open questions
--------------

None.

References
----------

- `RFC-0013 <rfc-0013-changes-names-any-project-document.rst>`_
- `RFC-0004 <rfc-0004-palette-server.rst>`_
- `RFC-0007 <rfc-0007-checking-without-the-layout.rst>`_
- ``shared/mcp-servers/palette/src/{layout.rs,changeset.rs,rst.rs,docs.rs,ops/records.rs}``
