RFC-0007: Checking shared documents without the layout
======================================================

:Status: Accepted
:Implementation: complete — ``Layout::infer`` in the palette server, the check
  and generate commands, the lint tool
:Verification: build — 2026-09-30; ``cargo test`` on macOS (156 tests) and
  ``palette check`` on a checkout of this repository without ``_palette/``, 0
  findings; slate CI on ``next`` green (run 36659672290)
:Areas: palette; MCP servers; CI
:Authors: Claude Fable 5.1
:Reviewers: none yet
:Implementers: Claude Fable 5.1
:Accepted: 2026-09-30, Hamin Sung (decision made in conversation)
:Date: 2026-09-30
:Revised: none
:Depends: RFC-0004 (the check and generate commands and the document model)
:Supersedes: none
:Related: RFC-0003 (the families and their placement)
:Changes: none
:Description: The palette check and generate commands find the shared families
  from the documents themselves, so a checkout without the internal ``_palette/``
  folder, which is what collaborators and CI have, can be checked.

Summary
-------

``palette check`` and ``palette generate`` no longer require
``_palette/layout.rst``. When it is absent, the server walks the project,
classifies every RST document by its title and ``:Status:`` field, and places
each family where its documents were found; the internal families are then
empty. The layout keeps its role for the session's write tools, which need to
know where to create the next record. Nothing under ``_palette/`` is read by a
check.

Problem and context
-------------------

``_palette/`` is the user's personal record and is git-ignored in full, the
layout included. The layout was the only way the server located the shared
families (``docs/rfc``, ``docs/spec`` and so on), so a checkout without
``_palette/``, which is every collaborator's and CI's, could not be checked:
slate's own CI failed with ``no_layout`` on 2026-09-30 the first time it ran
``palette check``. The 2026-09-29 design conflated this repository's use of
palette (where the developer's machine always has ``_palette/``) with the
product every project gets.

Evidence
--------

- slate CI run 36656166560 (2026-09-30): ``palette check: no_layout``.
- ``_palette/.gitignore`` as ``palette_init`` writes it: ``*``.
- The templates (``shared/workflows/palette/templates/``, 2026-09-30): every
  family has a title or status signature that identifies it (``RFC-NNNN:``,
  ``ADR-NNNN:``, ``Changeset:``, ``Glossary —``, ``Principles —``, ``:Status:
  Contract`` for a specification, ``:Status: Maintained`` with a title ending in
  ``design`` for a design document, ``Backlog —``, ``State —``, ``Phase N —``
  in a ``phase-N`` folder, ``Deliverable N:`` in a ``deliverables`` folder).
  Staging mirrors carry the same signature as the document they mirror, so the
  staging family is recognized by its folder name and its ``spec`` and
  ``design`` subfolders.

Goals and non-goals
-------------------

Goals:

- A checkout with no ``_palette/`` is checked and regenerated from its
  documents alone, with the same findings a layout would give for the shared
  families.
- No file under ``_palette/`` is required for a check; a layout is never
  committed for CI's sake.

Non-goals:

- Inferring a layout for the write tools; they keep requiring the layout.
- Recognizing a staging folder by any name other than ``staging``.

Requirements and invariants
---------------------------

- Without a layout, the inference walks every folder except ``_palette``,
  ``.git``, ``target``, ``node_modules``, ``fixtures`` and hidden folders, and
  skips a document whose title carries a template placeholder (``<...>``).
- A family found in two places keeps the first and reports the second as a
  P012 finding on that document.
- A family with no documents is internal, which is empty in such a checkout.
- The lock lives in ``_palette/``; regeneration without that folder runs
  without a lock and creates no folder.

Design
------

``Layout::infer`` in ``layout.rs`` returns a ``Layout`` whose placements are
the found folders and files, so every other part of the server (``Locations``,
the snapshot, the lint, the generators) is unchanged. ``Snapshot`` records
``inferred``; the layout file is not part of the snapshot when inferred.

Impact and compatibility
------------------------

- ``palette check`` and ``palette generate`` work in CI and for collaborators.
  ``palette_lint`` in a session behaves the same.
- The ``no_layout`` error remains for the write tools and ``palette_status``.
- A project whose staging folder is not named ``staging`` and has no layout is
  checked as if the mirrors were a second spec or design family, and gets P012
  findings; naming the folder ``staging`` or adding a layout resolves it.

Implementation and transition
-----------------------------

Shipped with slate v0.7.0. No migration.

Verification strategy
---------------------

- Unit tests: inference of every shared family and the staging mirror from a
  tree; skipping of templates, fixtures and build folders; P012 on a family
  found twice.
- CLI test: ``check`` exits 0 and ``generate`` succeeds without creating
  ``_palette/`` on the valid fixture with its ``_palette/`` removed.
- slate CI: ``validate.sh`` runs ``palette check`` on the checkout.

Alternatives and costs
----------------------

- Commit ``_palette/layout.rst``. Rejected: the layout is internal, and a
  project's check must not depend on one person's folder.
- A second, committed configuration file naming the shared paths. Rejected:
  the documents already identify themselves; a second file is one more thing
  to keep in step.

Open questions
--------------

None.

References
----------

- `RFC-0004 <rfc-0004-palette-server.rst>`_
- ``shared/mcp-servers/palette/src/layout.rs``
