RFC-0009: Record header rules and several active phases
=======================================================

:Status: Accepted
:Implementation: complete — the rfc, adr and layout templates, the palette
  server's parser, lint, index, status and record and phase tools, the palette
  rule and skills, this repository's records
:Verification: build — 2026-09-30; cargo test on macOS (169 tests) and palette
  check on this repository; Linux and Windows through slate CI on the next push
:Areas: palette; templates; MCP servers
:Authors: Claude Fable 5.1
:Reviewers: Hamin Sung
:Implementers: Claude Fable 5.1 (2026-09-30)
:Accepted: Hamin Sung (2026-09-30T04:49Z)
:Date: 2026-09-30
:Revised: 2026-09-30 — the earlier Accepted form lints as a P016 warning, not
  a P002 error; 2026-09-30 — ``palette_status`` keeps no implementation count,
  so "only complete is complete" is P007 and promote
:Depends: RFC-0003 (direct use of the record header fields and relation rules
  this record extends); RFC-0004 (the server, its lint rules and its write
  tools this record changes)
:Supersedes: none
:Related: none
:Changes: spec/palette-server.rst (Templates as schema; Records and relations;
  Lint rules; Read tools; Write tools)
:Description: Records may supersede part of an older record, carry more
  implementation states, keep a legacy Amends relation and name who accepted
  them and when; a project may have several active phases.

Summary
-------

Four header conventions of a project moving to palette become part of the
record contract, and one limit of the work documents is lifted. ``Supersedes``
may name part of an older record, which then keeps its status.
``Implementation`` gains ``in-progress``, ``abandoned`` and ``unassessed``.
``Amends``, a relation older records carry, is parsed and indexed but admitted
only up to a date the layout names. ``Accepted`` names who accepted the record
and when. More than one phase may be active at a time; an item still belongs to
one phase. The templates, the server, the palette rule and skills, and the
palette-server spec change together.

Problem and context
-------------------

saltyos is moving 124 RFCs and 27 ADRs into palette, and its header rules were
meant to be adopted in full (the palette design session of 2026-09-29). Four of
them were never discussed and are missing:

- A record that replaces only part of an older record. Today ``Supersedes``
  forces the older record to ``Superseded`` (P005), which is wrong for the
  parts that still hold; 14 saltyos records replace a part.
- An implementation that is under way, abandoned, or inherited from an older
  "Implemented" claim that nobody re-verified. Today the only values are
  ``not-started``, ``partial``, ``complete`` and ``not-applicable``, and a
  record with an unverified claim would have to say ``complete``; saltyos uses
  "In progress" on 5 records and "Unassessed" on 3.
- ``Amends``: 61 saltyos RFCs name an older record they amend, and the older
  records carry the reverse. saltyos stopped writing either on new records,
  because with many records an amendment cascades over dozens of them; the past
  relations stay as history.
- ``Accepted`` today takes a date and a name in a form the tools do not check
  beyond the date; who accepted a record and when it happened were what the
  saltyos records needed to say.

Separately, the server allows one active phase. Work that has to run beside a
phase without belonging to it, such as this change beside phase 2 of this
repository, has nowhere to go but the active phase.

Evidence
--------

- saltyos, counted 2026-09-30 in its session: 14 partial supersessions
  (RFC-0120 over RFC-0104 among them), Implementation "In progress" 5 and
  "Unassessed" 3 ("Abandoned" in the policy, unused), 61 RFCs with ``Amends``.
- This repository at 9cd43eb: the eight records write ``Accepted`` as ``<date>,
  <who> (<how>)``; the templates and ``schema.rs`` allow ``not-started |
  partial | complete | not-applicable`` for ``Implementation``;
  ``lint/relations.rs`` rejects a ``Supersedes`` target that is not
  ``Superseded`` and ``ops/records.rs`` sets ``Superseded`` on the older record
  when a record is created.
- The single active phase is enforced in ``ops/backlog.rs`` (``phase_open``
  fails while a phase is active; a deliverable belongs to an item in the active
  phase) and assumed by ``backlog.rs`` ``active_phase()``, by the
  ``palette-resume`` and ``palette-state`` skills, by ``templates/rubrics.rst``
  and by ``spec/palette-server.rst``. The backlog already records the status of
  every phase and every item's ``in-phase-<N>``, so the data model needs no
  change.
- The palette crate is 0.1.0 with 158 tests; ``~/.local/bin/palette`` reports
  0.1.0.
- Decisions D-33 to D-37 of this repository's state, 2026-09-30.

Goals and non-goals
-------------------

Goals:

- A saltyos record header is accepted by the lint without losing a relation it
  records.
- Each new value has one meaning the tools act on, and none of them is read as
  ``complete``.
- New records cannot start an amendment chain.
- Two pieces of work with different goals can run as two active phases.

Non-goals:

- Changing any saltyos document; a migration tool beyond what the lint reports.
- Storing the reverse of a relation: "superseded by" and "amended by" stay
  computed.
- Any change to the backlog status vocabulary or to phase closing.

Requirements and invariants
---------------------------

- A partial supersession changes nothing on the older record; only a whole
  supersession sets ``Superseded``.
- ``Implementation`` values other than ``complete`` may leave changeset edits
  pending; ``palette_changeset_promote`` and the status counts treat only
  ``complete`` as complete.
- ``Amends`` is never offered by a template or a tool; the lint admits it only
  on a record dated on or before the layout's ``amends-until``.
- ``Accepted`` names the accepting person; the date and time are expected and
  their absence is reported as a warning.
- An item belongs to at most one phase; every tool that took "the active phase"
  takes any active phase.
- Records accepted under the earlier header forms keep their meaning: the eight
  records of this repository are rewritten in the new ``Accepted`` form as a
  clarification.

Design
------

The contract is ``spec/palette-server.rst`` in this record's changeset
(Templates as schema, Records and relations, Lint rules, Read tools, Write
tools). In outline:

- ``Supersedes`` entry: ``RFC-<N> (<what is replaced>)`` or ``RFC-<N> (in part:
  <what is replaced>)``, both shown by the templates; the tools take ``partial:
  true`` on an entry. A record is ``Superseded`` exactly when a whole entry
  names it; a partial entry neither sets nor forbids the status. The computed
  "superseded by" carries the part.
- ``Implementation``: ``not-started | in-progress | partial | complete |
  abandoned | unassessed | not-applicable`` with the `` — <scope>`` suffix;
  ``in-progress`` is work under way, ``partial`` a settled part, ``unassessed``
  an inherited claim not re-verified.
- ``Amends``: an optional header field after ``Related``, ``RFC-<N> (<what
  changed>)`` or ``ADR-<N> (<what changed>)``, repeatable, older records only;
  an ADR amends only an ADR, since it decides within an RFC's contract.
  The layout template gains the required setting ``:amends-until: none |
  <YYYY-MM-DD>``, as ``checker`` is one today; it bounds the ``Date`` of a
  record that may carry ``Amends``, and ``none`` admits no record.
- ``Accepted``: ``none | <who> | <who> (<YYYY-MM-DDTHH:MMZ>)``; the middle
  form is what P016 warns about; ``<YYYY-MM-DDTHH:MMZ>`` becomes a checked
  placeholder, UTC with the ``Z`` suffix; ``palette_record_update`` writes the
  full form when a record becomes ``Accepted``.
- Phases: ``phase_open`` no longer checks for an active phase; the deliverable
  tools accept an item of any active phase; ``palette_status`` lists every
  active phase.

Impact and compatibility
------------------------

- A project on the earlier templates lints with P016 warnings on ``Accepted``
  (the earlier ``<date>, <who>`` value reads as a name without a time) until
  its records are rewritten; the change is mechanical and this repository's
  records are rewritten with the implementation.
- ``palette_record_create`` and ``palette_record_update`` gain an optional
  ``partial`` on supersedes entries; no existing input changes meaning.
- A layout without ``amends-until`` lints with P012 until the line is added;
  ``palette_init`` writes ``none`` and this repository's layout gains it with
  the implementation.
- The kits ship the templates, so the three kits re-render and bump their
  versions; the palette crate becomes 0.2.0.

Implementation and transition
-----------------------------

Templates and ``schema.rs`` change first, since the checks derive from them;
the parser, lint, index, status and tools follow; the spec is promoted with the
implementation; this repository's records are rewritten under the new template
in the same change, so the repository never lints red between commits.

Verification strategy
---------------------

- A synthetic fixture per rule: a partial supersession that leaves the older
  record Accepted and a whole one that requires Superseded; each
  ``Implementation`` value against P007 and promote; ``Amends`` on a record
  before, on and after ``amends-until``, and in a project without the setting;
  ``Accepted`` with and without the parenthetical; two active phases with an
  item in each, an item refused a second phase, and a deliverable written in
  the second phase.
- ``palette check`` on this repository and, once the saltyos session has
  converted its header values to the kit's tokens, on a saltyos checkout
  reports no error attributable to the four conventions.
- ``cargo test -p palette``, ``clippy -D warnings`` and ``fmt --check`` on
  Linux, macOS and Windows; ``validate.sh``; the three kits render.

Alternatives and costs
----------------------

- Leave ``Amends`` out and rewrite saltyos's 61 records as ``Related``: loses
  the direction of the relation and edits 61 documents outside this repository.
- Make partial supersession a separate field: two fields for one relation; the
  parenthetical already carries the part.
- Keep one active phase and open phase 3 after phase 2: serializes work that
  does not depend on phase 2.

Open questions
--------------

None.

References
----------

- `RFC-0003 <rfc-0003-palette-document-system.rst>`_
- `RFC-0004 <rfc-0004-palette-server.rst>`_
- ``shared/workflows/palette/templates/{rfc,adr,layout}.rst``
- ``shared/mcp-servers/palette/src/{records.rs,schema.rs,layout.rs}``
- ``shared/mcp-servers/palette/src/{status.rs,generated.rs}``,
  ``lint/relations.rs``, ``ops/{records,backlog}.rs``
