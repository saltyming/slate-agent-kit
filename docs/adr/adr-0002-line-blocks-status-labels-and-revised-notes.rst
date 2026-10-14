ADR-0002: P001 reports line blocks, wrapping keeps status labels off line starts, and Revised notes lose their closing period
=============================================================================================================================

:Status: Accepted
:Implementation: complete — shared/mcp-servers/palette, docs
:Verification: build — 2026-10-14; workspace tests (941 passed, 0 failed),
  clippy -D warnings and fmt on rustc 1.99.0 on macOS, palette check clean on
  this repository
:Within: spec/palette-server.rst (Contract)
:Authors: Hamin Sung; Claude (Opus 5.5)
:Reviewers: none yet
:Implementers: Claude (Opus 5.5)
:Accepted: Hamin Sung (2026-10-14T07:24Z)
:Date: 2026-10-14
:Revised: none
:Depends: RFC-0024 (direct use of the wrapping the write tools and palette
  reflow apply)
:Supersedes: none
:Related: none
:Changes: spec/palette-server.rst (Lint rules; Write tools; Command line)
:Description: Three defects found while closing Phase 4 are fixed in the
  palette server: P001 reports a line block, wrapping never starts a line with
  a done or pending label, and a Revised note's closing period is dropped.

Context
-------

Implementing RFC-0024 showed three defects in the palette server. A line that
opens with ``|`` is an RST line block, which the house style does not allow,
and it is also how a pipe-table row reads when its separator line is missing;
P001 reported tables only by their borders and separators, so such a line
passed. The wrapping that RFC-0024 added could start a line in a work document
with ``done:`` or ``pending —``, which P009 reports as a status marker, so a
reflow or a write tool would refuse its own result. ``palette_record_update``
joins Revised entries with ``; ``, so a note ending in a period wrote ``.;``,
as RFC-0024's Revised field showed.

Evidence
--------

- ``shared/mcp-servers/palette/src/lint/syntax.rs``: P001 matched grid borders,
  simple-table borders and pipe separators only.
- ``src/lint/patterns.rs`` ``STATUS_MARKER``: a line start of ``done`` or
  ``pending`` followed by a colon or a dash is a P009 error in phase,
  deliverable and state documents; ``src/rst.rs`` ``may_break_between`` refused
  breaks before list, field, table and comment markers and rules, not before
  these labels.
- ``src/ops/records.rs``: the clarification entry was ``<date> — <note>`` with
  semicolons replaced and appended after ``; ``; RFC-0024 carried
  ``retirement).; 2026-10-14``.
- No document of this repository has a line opening with ``|``.

Decision
--------

P001 reports a line opening with ``|`` followed by a space or nothing as a line
block or a pipe-table row, and keeps reporting a table when the next line is a
pipe separator; inline literals and literal blocks are not read.
``may_break_between`` refuses a break before ``done`` or ``pending`` in any
case, bare or with a colon, so no wrapped line starts with one in any document.
``palette_record_update`` drops a note's closing periods and spaces before it
writes the Revised entry. RFC-0024's existing entry is corrected by hand once,
before its first commit.

Consequences
------------

A document with a line block now fails P001; none in this repository does.
Wrapping keeps ``done`` or ``pending`` with the word before it, which changes
some line breaks of documents wrapped earlier; the records RFC-0024 reflowed
were reflowed again. A note that ends with an abbreviation loses its period.

Alternatives
------------

- Allowing line blocks: the house style lists the constructs it allows, and a
  line block is not one; a pipe row without its separator would still pass.
- Refusing a break only before ``done:`` or ``pending`` followed by a dash: the
  dash is the next word, which the break check does not see; refusing the bare
  word is simpler and costs only a line break.
- Rewriting existing Revised entries on every update: the server keeps an
  accepted record's past entries as written.

Verification
------------

``cargo test -p palette`` with tests for line blocks, a pipe row without its
separator, a pipe table, a ``|`` inside prose and inside a literal block,
breaks before the labels, a wrapped paragraph that would otherwise start a line
with ``done:``, and clarification notes ending in a period; clippy and fmt;
``palette check .`` on this repository.

References
----------

- RFC-0024 (the wrapping and ``palette reflow``).
- ``shared/workflows/palette/templates/house-style.rst`` (the allowed
  constructs).
- Backlog items B-16, B-17 and B-18.
