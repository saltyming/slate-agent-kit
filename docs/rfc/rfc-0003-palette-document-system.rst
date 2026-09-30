RFC-0003: palette document system
=================================

:Status: Accepted
:Implementation: complete — the palette rule, the templates, the layout and
  the skills in the toolkit
:Verification: static — 2026-09-30; ``palette check`` on this repository's own
  documents, 0 findings; render and ``validate.sh``
:Areas: palette; project documentation
:Authors: Claude Opus 5.5
:Reviewers: Hamin Sung
:Implementers: Claude Opus 5.5 (2026-09-29 to 2026-09-30)
:Accepted: Hamin Sung (2026-09-30T01:26Z)
:Date: 2026-09-29
:Revised: 2026-09-30 — a thirteenth family (contributing) and a closed phase
  keeps its files
:Depends: RFC-0001 (turn mode; a document's next action is a proposal)
:Supersedes: none
:Related: none
:Changes: none
:Description: palette becomes a layered document system in which each family
  holds one kind of fact, placement is chosen per project, decisions and their
  evidence move into records, and no document records development stages.

Summary
-------

palette grows from a planning folder into a documentation system: backlog,
phase, deliverable and state for work; RFC and ADR for decisions with their
evidence; changesets and staging for accepted but unimplemented contracts;
design, spec, principles and glossary for maintained truth, and contributing
for how people and agents contribute (branches, commits, pull requests,
verification). Each family holds
one kind of fact, each project chooses where each family lives, and no family
records development stages.

Problem and context
-------------------

In a project that uses palette with a mature record discipline, the palette
documents themselves failed in four ways: the state document grew into a log
of decisions and results; the same fact (an item's status, a decision) was
written in several files and they disagreed; the state document prescribed
the next session's actions, including models and concurrency; and updates
were left to the end of a session, so an interrupted session left them stale.
A family defined as the destination for material moved out of decision records
became a store with no exit.

Evidence
--------

Measured 2026-09-29 in that project:

- The state document was 4,462 lines. Its accepted-decisions section was 3,255
  lines and carried 13 decision markers; its next-safe-action section was 212
  lines of plans and results.
- A deliverable (then called a story) was ``done`` in the state document and
  ``pending`` in the phase index.
- The records folder held 93 files, 52 of them per-record implementation
  histories or evidence, the largest 12,061 lines; one file was a list of the
  day's decisions, a role the state document and the records already had.
- The project's own policy defined the records folder as the place for
  "investigations, verification evidence, and historical implementation
  records moved out of decision documents".
- The kit's templates duplicate: the phase brief's scope list and the phase
  index list the same items; the backlog template's product principles section
  and a principles document hold the same principles.

Goals and non-goals
-------------------

Goals:

- One kind of fact per family; one place per fact.
- Evidence inside the record that relies on it.
- Placement of each family chosen per project; authority independent of
  placement.
- A new session resumes from state without being told what to do.

Non-goals:

- The tools that check and write the documents.

Requirements and invariants
---------------------------

- Only the user's approval authorizes execution, wherever a document lives.
- A document under a project path never links into ``_palette/``.
- Item status is recorded only in the backlog.
- No document records development stages or progress narrative; history lives
  in version control and session transcripts.

Design
------

- Families: backlog (every item and its status; the index of phases and
  deliverables), phase (goal, reason, assumptions, exit criteria), deliverable
  (one approved unit and its ``Done when``), state (decisions not yet written
  into a record, questions blocking the active phase, discrepancies; one line
  each), RFC and ADR (decisions with their evidence and the full header set),
  changeset and staging (accepted edits to maintained documents that the
  source does not implement yet), design, spec, principles, glossary.
- Every family is used. ``layout.rst`` places each family internal
  (``_palette/``, personal, not committed) or at a project path (committed,
  shared). Every document is RST in the house-style subset.
- Relations between records point only to older records, ``Depends`` lists
  direct uses only, and reverse links are computed.
- No handoff document: state is updated in place when a decision is made or a
  fact is verified; a new session reads state within a budget, reports,
  proposes and waits.
- No reviews document: closing a phase moves each outcome to where it is read
  next; the phase and deliverable files stay as the record of what was
  approved, and the backlog alone carries status.
- File names are lowercase kebab-case; records are ``rfc-0001-<slug>.rst`` and
  ``adr-0001-<slug>.rst``; a changeset carries its record's name; staging
  mirrors ``design/`` and ``spec/``.

Impact and compatibility
------------------------

- An existing palette project has stories, a separate index, phase briefs and
  reviews; it has to be moved to the new families. The kit changelogs describe
  the move; the move itself is done per project.
- Terms change (story becomes deliverable, handoff becomes approval or resume);
  the glossary lists the replacements.

Implementation and transition
-----------------------------

The palette rule, the templates and the skills change together with the
server that checks and writes the documents, and ship in the same release.

Verification strategy
---------------------

- The palette server's lint passes on this repository's own palette and
  ``docs/``.
- In a scratch project, init creates the chosen layout, and resume reports
  state within budget and starts nothing.

Alternatives and costs
----------------------

- Keep the records family with a retention policy: its content still leaves the
  records that rely on it, and the policy is prose nobody enforces.
- Keep a reviews document per phase: it is read once, and what matters in it
  already has a place where it will be read again (backlog, rules, records).
- A fixed set of profiles instead of per-family placement: projects already have
  documentation folders of their own.

Open questions
--------------

None.

References
----------

- ``docs/glossary.rst``
- ``shared/workflows/palette/templates/``
