RFC-0012: Tables and code blocks in maintained documents and records
====================================================================

:Status: Accepted
:Implementation: complete — the house-style template, the palette server's
  scanner and lint, its tests
:Verification: build — 2026-09-30; cargo test on macOS (500 tests), palette
  check on this repository with and without _palette/ and on the saltyos
  checkout, Linux and Windows through slate CI on the next push
:Areas: palette; templates; MCP servers
:Authors: Claude Opus 5.5
:Reviewers: Hamin Sung
:Implementers: Claude Opus 5.5 (2026-09-30)
:Accepted: Hamin Sung (2026-09-30T06:50Z)
:Date: 2026-09-30
:Revised: none
:Depends: RFC-0004 (direct use of the lint rules and of changeset staging and
  promote, which must carry the new constructs unchanged)
:Supersedes: none
:Related: RFC-0003 (the families and the house-style subset every palette
  document follows)
:Changes: spec/palette-server.rst (Lint rules)
:Description: The house style admits the list-table and code-block directives in
  records, changesets, staging and maintained documents, and a new lint rule
  checks their structure so a malformed one is an error instead of silent
  breakage.

Summary
-------

The house style forbids every table and directive. In the families other
people read (rfc, adr, changeset, staging, design, spec, principles, glossary,
contributing) it now admits two directives: ``.. list-table::`` with the
options ``:header-rows:`` and ``:widths:``, and ``.. code-block::`` with a
language argument. A new lint rule, ``P017``, checks their structure against
what docutils accepts; a malformed one is an error. Table cells are checked
like prose. The work families (backlog, phase, deliverable, state) and the
layout keep the full ban, and every other directive, simple and grid tables,
substitutions and footnotes stay forbidden everywhere.

Problem and context
-------------------

The saltyos documents are moving to the kit's palette families. Their
maintained documents and records use tables and code blocks throughout, and
palette 0.2.1 rejects each one with ``P001``. The ban came from the older
internal house style (saltyos ``_palette/templates/house-style.rst``), whose
reason was that palette documents are structured text agents re-read and that
models break complex RST silently. That reason is about the internal work
documents. When the families grew to cover records and maintained documents,
the internal restriction spread to documents whose content is tabular by
nature (key tables, request tables, option matrices), and no record decided
that.

The user decided on 2026-09-30 to admit ``list-table`` and ``code-block`` in
those families, with a structure check that removes the silent-breakage risk.

Evidence
--------

- ``shared/workflows/palette/templates/house-style.rst`` and
  ``lint/syntax.rs`` (palette 0.2.1): every table and every directive other
  than a comment is a ``P001`` error, in every family. No RFC records the
  ban; RFC-0003 says only that every document follows the house-style
  subset.
- saltyos ``docs/`` (2026-09-30, staging copies excluded): 167
  ``list-table`` directives (spec 71, changeset 49, RFC 40, design 6, ADR 1,
  and ``principles.rst``), options ``:header-rows:`` (167) and ``:widths:``
  (22 by this record's scan: 15 numeric, 7 ``auto``; the saltyos request
  counted 9); about 50 carry a title argument
  (``.. list-table:: Key requests``); 104 links sit inside table cells; all
  rows use ``* -`` and all further cells ``-``; 28 ``code-block`` directives
  with the languages ``text``, ``rust``, ``ini``, ``c`` and ``toml``, one of
  them indented inside a definition-list body.
- docutils 0.23 (``rst2html``), run on 2026-09-30: ``:header-rows:`` equal to
  the number of rows is an error (no body rows remain), and larger is an
  error; rows with different cell counts, a ``:widths:`` count that differs
  from the columns, and ``:widths: grid`` are errors; ``:widths: auto``, an
  empty cell and a title argument are accepted. Without a blank line after
  ``.. list-table::`` the rows are read as the directive's arguments and the
  table is empty; without one after ``.. code-block:: rust`` the code is read
  as extra arguments. A ``code-block`` without a language is accepted. Two
  spaces between ``* -`` and the cell text move the cell's indent past its
  continuation lines and break the two-level list; rows indented deeper than
  the options become a block quote and are rejected; a tab in the
  indentation counts as the next multiple of eight columns.
- The scanner (``rst.rs``) classifies every indented line after explicit
  markup as ``ExplicitBody``, and ``Doc::links`` and the substitution and
  footnote checks read prose lines only: admitting ``list-table`` without
  reclassifying its cells would leave links in cells unchecked by ``P004``.
- ``changeset.rs`` (``converted_body``) copies every line of an edit body
  verbatim except headings, which start at column 0; an indented directive
  body passes through staging and promote unchanged.

Goals and non-goals
-------------------

Goals:

- ``list-table`` and ``code-block`` pass the lint in the rfc, adr, changeset,
  staging, design, spec, principles, glossary and contributing families.
- A structurally wrong one is an error with its own rule id, distinct from
  ``P001`` ("not allowed here").
- Cell text gets every check prose gets: links, substitution and footnote
  references, tables.
- A changeset edit carrying either directive survives staging generation and
  promote byte for byte.

Non-goals:

- Other directives (``note``, ``code``, ``csv-table``, ``table``, ``image``
  and the rest), simple and grid tables, substitutions, footnotes and
  citations: still forbidden.
- Tables or directives in the work families and the layout.
- Converting any project's documents; saltyos converts its own.
- Checking the code inside a code block, or the language name against a
  highlighter's list.

Requirements and invariants
---------------------------

- A document the lint passes renders under docutils without a table or
  code-block error.
- ``P001`` still reports ``list-table`` and ``code-block`` in backlog, phase,
  deliverable, state and layout documents.
- Every link in a table cell is checked by ``P004`` as if it were prose.
- Staging and promote leave the lines of both directives unchanged, with LF
  and CRLF files alike.

Design
------

Where the directives are admitted
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

By the file's family as the snapshot places it (with or without a layout):
RFC, ADR, changeset, staging, design, spec, principles, glossary and
contributing. Anywhere else, and for any other directive name, ``P001``
reports the directive as before. Either directive may be indented, inside a
list item or a definition body, and may appear inside a table cell.

list-table
~~~~~~~~~~

The admitted form::

  .. list-table:: <optional title>
     :header-rows: <n>
     :widths: <w> <w> ... | auto

     * - <cell>
       - <cell>
         <continuation of the cell>
     * - <cell>
       -

``P017`` (error) reports each of the following:

- an option other than ``:header-rows:`` and ``:widths:``, an option given
  twice, or an option line anywhere but directly after the directive line;
- ``:header-rows:`` that is not a non-negative integer; ``:widths:`` that is
  neither ``auto`` nor positive integers separated by spaces;
- no blank line between the directive line (or its last option) and the body,
  or no body; rows that do not start at the options' column;
- a body line that breaks the two-level list: at the body's indent every
  line starts a row with ``* -`` (the first cell on the same line); two
  columns deeper every line starts a further cell with ``-``; one space
  separates a marker from the cell text; cell content continues four columns
  deeper than the body's indent or more; blank lines may separate any of
  these;
- a row whose cell count differs from the first row's;
- ``:header-rows:`` greater than zero and not less than the number of rows;
- numeric ``:widths:`` whose count differs from the number of columns.

An empty cell (``-`` alone) is admitted, as docutils admits it.

The scanner classifies cell content by scanning each cell again as its own
text with its row or cell marker blanked out, so a cell's paragraphs are prose, a
literal block inside a cell is a literal block and a directive inside a cell
is a directive. Links in cells are therefore checked by ``P004``, references
by ``P001``, and a nested ``code-block`` or ``list-table`` by ``P017``.

code-block
~~~~~~~~~~

The admitted form::

  .. code-block:: <language>

     <code>

``P017`` (error) reports: a missing language or more than one argument (a
plain block uses ``::``); a language that is not one token of letters,
digits and ``_ + . # -``; any option; no blank line after the directive line;
no non-blank content indented deeper than the directive. The content is not
checked, like a literal block.

For both directives, ``P017`` reports a tab in the indentation of the
directive line, its options or its body: docutils expands a tab to the next
multiple of eight columns, which moves a line into or out of the directive
without showing it.

House style and contract
~~~~~~~~~~~~~~~~~~~~~~~~

The house-style template states the admitted families and both forms; the
spec's ``P001`` entry names the exception and a new ``P017`` entry states the
structure rules. The server derives nothing new from other templates.

Impact and compatibility
------------------------

- Relaxation only: every document that passed palette 0.2.1 still passes.
- saltyos's 167 tables and 28 code blocks lint as they are, provided their
  structure holds; the eight files with simple or grid tables still fail
  until converted.
- Cell text that was never checked is now checked; a project admitting tables
  may see new ``P004`` findings for broken links in cells, which docutils
  would also report.
- The change ships as a minor release of the palette server (0.3.0) and of
  the kits, since the house-style template is rendered into each kit.

Implementation and transition
-----------------------------

The scanner and the lint change together, with the house-style template and
the spec entry; ``P017`` joins the rule list in the server's tool description.
No data migrates. The spec edit is promoted when the server that implements it
is released.

Verification strategy
---------------------

- Unit and fixture tests: each ``P017`` case above, the admitted forms
  (title argument, ``:widths: auto``, empty cell, multi-line cell, a cell
  continuation line starting with ``*`` emphasis, an indented code block, a
  code block inside a cell), ``P001`` for both directives in a work family,
  and ``P004`` on a broken link inside a cell.
- A changeset test: ``Replace``, ``Insert after`` and ``Create`` edits whose
  bodies carry a ``list-table`` and a ``code-block``; the generated staging
  documents and the promoted maintained documents hold those lines
  unchanged, for LF and CRLF targets.
- ``palette check`` on this repository, and on a copy of saltyos's ``docs/``
  with its layout, reporting no ``P001`` or ``P017`` for the admitted
  constructs.

Alternatives and costs
----------------------

- Keep the ban and have projects write lists: the saltyos tables carry three
  to thirteen columns, and lists lose the comparison the table exists for.
- Admit the directives without a structure check: the silent-breakage reason
  for the ban returns, and links in cells stay unchecked.
- Admit simple and grid tables too: their structure is column alignment,
  which models break most often and which a line scanner checks poorly;
  ``list-table`` expresses the same content as a list.
- Report malformed directives under ``P001``: one id would mean both "not
  allowed here" and "allowed but malformed", and the fix differs.
- Admit a bare ``code-block``: ``::`` already writes an unlabeled block, and
  one form per purpose keeps documents uniform.

Open questions
--------------

None.

References
----------

- `RFC-0003 <rfc-0003-palette-document-system.rst>`_
- `RFC-0004 <rfc-0004-palette-server.rst>`_
- ``shared/workflows/palette/templates/house-style.rst``
- ``shared/mcp-servers/palette/src/rst.rs``, ``lint/syntax.rs``,
  ``changeset.rs``
- docutils, reStructuredText directives: ``list-table`` and ``code``
