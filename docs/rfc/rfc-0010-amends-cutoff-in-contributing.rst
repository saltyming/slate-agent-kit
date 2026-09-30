RFC-0010: Amends cutoff in the contributing document
====================================================

:Status: Accepted
:Implementation: complete — the contributing template's Records section, the
  layout template, the palette server's cutoff reader and lint, this
  repository's documents
:Verification: build — 2026-09-30; cargo test on macOS (172 tests), palette
  check on this repository with and without _palette/; Linux and Windows
  through slate CI on the next push
:Areas: palette; templates; MCP servers
:Authors: Claude Fable 5.1
:Reviewers: Hamin Sung
:Implementers: Claude Fable 5.1 (2026-09-30)
:Accepted: Hamin Sung (2026-09-30T05:29Z)
:Date: 2026-09-30
:Revised: none
:Depends: RFC-0009 (direct use of the Amends relation and its cutoff); RFC-0007
  (direct use of the check without a layout, which the cutoff must survive)
:Supersedes: RFC-0009 (in part: the layout setting ``amends-until``)
:Related: none
:Changes: spec/palette-server.rst (Records and relations; Lint rules; Write
  tools)
:Description: The date until which a record may carry Amends moves from the
  personal layout to the committed contributing document, so a checkout without
  _palette/ checks it the same way.

Summary
-------

RFC-0009 put the Amends cutoff in ``_palette/layout.rst`` as ``amends-until``.
The layout is internal and never committed, so a checkout without it (the case
RFC-0007 made checkable) had no cutoff and reported every ``Amends`` as an
error. The cutoff moves to the contributing document, which is a palette
family and states how records are written: a ``Records`` section with
``:Amends: none | until <YYYY-MM-DD>``. The layout setting is removed.

Problem and context
-------------------

A project rule (until when a record may amend another) was recorded in a
personal setting file. Every other checkout of the project, including this
repository's own CI, lacked the value and could not apply the rule. RFC-0007
already rejected committing the layout and a second configuration file; the
committed palette documents are the only remaining home.

Evidence
--------

- RFC-0009's implementation (palette 0.2.0): ``Layout::infer`` sets no
  cutoff, and ``lint/relations.rs`` reads a missing cutoff as ``none``, so
  ``palette check`` on a checkout without ``_palette/`` reported an error on
  every record with ``Amends`` (found on the saltyos migration, 2026-09-30).
- RFC-0007, Goals: a checkout with no ``_palette/`` is checked with the same
  findings a layout would give; Alternatives: a committed layout and a second
  configuration file were rejected.
- RFC-0003 makes the contributing family the place for how people and agents
  contribute; a rule about writing records belongs there.

Goals and non-goals
-------------------

Goals:

- The Amends cutoff is read from a committed palette document, so every
  checkout applies the same rule.
- One place for the fact: the layout setting goes.

Non-goals:

- Any other layout setting; ``checker`` stays where it is.

Requirements and invariants
---------------------------

- A record may carry ``Amends`` only when the contributing document's
  ``Records`` section says ``until <date>`` and the record's ``Date`` is on or
  before it; ``none``, a missing section or no contributing document admits no
  record.
- The check gives the same result with and without a layout.
- ``palette_init`` writes no cutoff; the template's ``Records`` section is
  filled by the project.

Design
------

The contributing template gains a last section ``Records`` with the field
``:Amends: none | until <YYYY-MM-DD>`` and prose for the project's other record
conventions. The server reads the field from the contributing document of the
snapshot (placed by the layout, or inferred without one); P005 applies the
cutoff; P002 checks the section and the value like every template field. The
layout template loses ``amends-until``, and P012 checks ``checker`` only. The
contract is ``spec/palette-server.rst``.

Impact and compatibility
------------------------

- A contributing document without a ``Records`` section lints with a P002
  error until the section is added; a layout that still carries
  ``amends-until`` lints with P012 until the line is removed. Both edits are
  one line.
- A project without a contributing document admits no ``Amends``, as before
  when the layout said ``none``.

Implementation and transition
-----------------------------

Templates first, then the reader and the lint, then this repository's
contributing document and layout; the change ships as a patch release of the
palette server and the kits.

Verification strategy
---------------------

- The valid fixture carries the section; tests cover ``none``, ``until`` before
  and after a record's date, a missing section, a value outside the grammar,
  and the same cutoff read after ``_palette/`` is removed.
- ``palette check`` on this repository with and without ``_palette/``.

Alternatives and costs
----------------------

- Skip the cutoff when there is no layout: local and CI results differ, against
  RFC-0007's goal.
- Keep the layout setting and add contributing prose: two places for one fact.
- Write the cutoff into each record: edits every older record.

Open questions
--------------

None.

References
----------

- `RFC-0009 <rfc-0009-record-header-rules.rst>`_
- `RFC-0007 <rfc-0007-checking-without-the-layout.rst>`_
- ``shared/workflows/palette/templates/{contributing,layout}.rst``
- ``shared/mcp-servers/palette/src/records.rs`` (``amends_cutoff``)
