RFC-0020: An Acceptance setting decides whether Accepted must carry the time
============================================================================

:Status: Accepted
:Implementation: complete — the contributing template's optional Acceptance
  field, P016 in the palette server, palette-init
:Verification: build — 2026-10-13; 2026-10-13, cargo test on macOS (251 tests in
  the palette crate, with and without a layout, LF and CRLF), clippy and fmt on
  stable 1.99.0, saltyos and SaltyMail check output identical before and after,
  Linux and Windows through slate CI on the next push
:Areas: palette; templates; MCP servers
:Authors: Claude Fable 5.1
:Reviewers: Hamin Sung
:Implementers: Claude Opus 5.5 subagent (server, template and tests) and Claude
  Fable 5.1 (spec) (2026-10-13)
:Accepted: Hamin Sung (2026-10-13T13:02Z)
:Date: 2026-10-13
:Revised: 2026-10-13 — an Acceptance field outside the Records section is a P002
  error
:Depends: RFC-0010 (the contributing document's Records section as the home of a
  per-project record setting that is checked with and without a layout);
  RFC-0009 (direct use of the Accepted forms and of P016, whose severity this
  record makes a setting); RFC-0019 (the Lint rules section as its changeset
  rewrites it, which this record's changeset edits in turn)
:Supersedes: RFC-0009 (in part: the Accepted form without the time lints as a
  P016 warning in every project)
:Related: none
:Changes: spec/palette-server.rst (Records and relations; Lint rules);
  shared/workflows/palette/templates/contributing.rst (the optional Acceptance
  field of the Records section); contributing.rst (Records)
:Description: The contributing document's Records section may carry :Acceptance:
  who-and-time | who, which turns an Accepted value without the time into an
  error or into nothing; without the field P016 keeps warning.

Summary
-------

``Accepted`` takes ``none``, ``<who>`` or ``<who> (<YYYY-MM-DDTHH:MMZ>)``, and
P016 warns about the middle form in every project (RFC-0009). Whether the time
is required is a project's choice: a project that writes every acceptance
through the server always has it; a project that accepts records by hand, or
carries records from before the templates, may not want the warning or may want
an error. The contributing document's ``Records`` section gains an optional
field ``:Acceptance: who-and-time | who``. With ``who-and-time``, an
``Accepted`` value without the time is a P016 error; with ``who``, it is not
reported; without the field, P016 warns as it does today. The field lives
beside ``Amends`` because, like the cutoff of RFC-0010, it must be read by a
checkout without ``_palette/``.

Problem and context
-------------------

P016 is a warning so that projects on the earlier templates lint without errors
(RFC-0009, *Impact and compatibility*). A warning is the same for the project
that wants the time on every record and the one that does not care; neither can
turn it into what it means for them. ``layout.rst`` cannot hold the choice: it
is internal and a CI checkout checks without it (RFC-0007). RFC-0010 solved the
same placement question for ``Amends`` by putting the cutoff in the
contributing document's ``Records`` section, which is committed with the
project.

Evidence
--------

- ``docs/spec/palette-server.rst`` *Records and relations*: "The form without
  the parenthetical is a warning"; *Lint rules* P016; read on 2026-10-13.
- RFC-0009 *Revised* (2026-09-30): the earlier Accepted form lints as a P016
  warning, not an error, for projects on the earlier templates.
- RFC-0010: the ``Amends`` cutoff is a field of the contributing document's
  ``Records`` section (``:Amends: none | until <YYYY-MM-DD>``), read by
  ``records::amends_cutoff`` from the snapshot's contributing document with and
  without a layout.
- ``shared/workflows/palette/templates/contributing.rst``: the ``Records``
  section holds the one field ``Amends``; the schema derivation treats every
  template field as required (``Templates as schema``), with ``Amends`` on
  records the one optional field.
- D-5 in the project state, 2026-10-13.

Goals and non-goals
-------------------

Goals:

- A project states once whether ``Accepted`` must carry the time, and the lint
  reports accordingly with and without a layout.
- Every project without the field lints exactly as today.

Non-goals:

- Changing what ``palette_record_update`` writes: it always writes
  ``<who> (<time>)``.
- A setting for any other field's form.
- Validating the time against the record's ``Date``.

Requirements and invariants
---------------------------

- The contributing template's ``Records`` section has, after ``Amends``, the
  optional field ``:Acceptance: who-and-time | who``; the schema treats it as
  optional in the contributing family, as ``Amends`` is on records, and P002
  reports any other value, a placement before ``Amends``, and the field found
  in the header or in another section.
- P016 reads the field from the contributing document the snapshot resolves
  (the layout's placement, or the inferred one): ``who-and-time`` makes an
  ``Accepted`` value without the time an error; ``who`` suppresses the finding;
  a missing field, a missing ``Records`` section or no contributing document
  keeps the warning.
- ``palette_record_update`` keeps writing ``<who> (<YYYY-MM-DDTHH:MMZ>)`` on
  acceptance whatever the setting.
- ``palette_init`` writes no contributing document and therefore no field; a
  project opts in by editing its contributing document.

Design
------

The contract is the *Records and relations* and *Lint rules* sections of
``spec/palette-server.rst`` as the changeset of this record writes them, and
the contributing template.

``records.rs`` gains ``acceptance_rule(snap) -> AcceptanceRule``
(``WhoAndTime``, ``Who``, ``Unset``) beside ``amends_cutoff``, reading the
``Acceptance`` field of the ``Records`` section the same way. The P016 check
takes the rule and emits an error, nothing, or the warning. ``schema.rs`` (or
where the template schema marks ``Amends`` optional) marks ``Acceptance``
optional for the contributing family. The template gains the field with its two
values; its explanatory placeholder says what each value does.

Impact and compatibility
------------------------

- Every existing contributing document lacks the field and lints as before;
  every existing P016 finding stays a warning until a project sets the field.
- A project that sets ``who-and-time`` turns its existing warnings into errors
  in the same check; that is the choice the field expresses.
- The field is read by palette 26.10.1 and later; an older server reports it as
  an unknown field (P002) in the contributing document.

Implementation and transition
-----------------------------

One change to the contributing template, the palette server and the spec; it
ships with slate v26.10.1. This repository's own contributing document gains
``:Acceptance: who-and-time`` with the change, since every record here is
accepted through the server.

Verification strategy
---------------------

- Synthetic fixtures, LF and CRLF, with and without a layout: a record accepted
  without the time under ``who-and-time`` (error), ``who`` (no finding), no
  field (warning), no ``Records`` section (warning), no contributing document
  (warning); a contributing document with a value outside the two (P002).
- ``palette check .`` on this repository with ``:Acceptance: who-and-time`` is
  clean; saltyos and SaltyMail, which lack the field, report the same findings
  as before.

Alternatives and costs
----------------------

- A ``layout.rst`` setting: not visible to a checkout without ``_palette/``, so
  CI and the session would report differently (RFC-0007).
- Make the time required everywhere: every project on the earlier templates
  gets errors it cannot clear without rewriting history.
- Remove P016: a project that wants the time loses the check.
- A command-line flag of ``palette check``: the choice would live in CI
  configuration rather than with the project's documents, and sessions would
  not see it.

Open questions
--------------

None.

References
----------

- `RFC-0009 <rfc-0009-record-header-rules.rst>`_
- `RFC-0010 <rfc-0010-amends-cutoff-in-contributing.rst>`_
- `RFC-0019 <rfc-0019-changesets-under-the-document-root.rst>`_
- ``shared/mcp-servers/palette/src/records.rs`` (``amends_cutoff``)
