RFC-0004: palette MCP server
============================

:Status: Accepted
:Implementation: not-started — the palette server's read and write tools and its
  command-line check
:Verification: documentation — 2026-09-29; decision record only
:Areas: palette; MCP servers
:Authors: Claude Opus 5.5
:Reviewers: none yet
:Implementers: none yet
:Accepted: 2026-09-29, Hamin Sung (decisions made in conversation)
:Date: 2026-09-29
:Revised: none
:Depends: RFC-0003 (the families, placement and relation rules it enforces)
:Supersedes: none
:Related: none
:Changes: spec/palette-server.rst (created)
:Description: A Rust MCP server reads, checks and writes palette documents, so
  structure and cross-file consistency are kept by code rather than by prose.

Summary
-------

palette documents are read, checked and written through a Rust MCP server
shipped like aside and dispatch. The server owns structure (identifiers,
status, links, sections, budgets, relations, changesets, staging, indexes); the
agent supplies prose through typed fields. A command-line mode runs the same
checks for CI.

Problem and context
-------------------

The failures described in RFC-0003 happened in a project whose written policy
already forbade them. The policy was correct and was not followed: a multi-file
update done by hand (a status, a decision moved to a record, a link) leaves the
files disagreeing when any step is skipped or the session ends early. Checking
RST with docutils depends on how Python was installed on each machine.

Evidence
--------

- The project's policy stated that the next-safe-action section holds one
  bounded action; the section was 212 lines (measured 2026-09-29).
- The same project runs a Python checker that needs docutils; its own
  instructions name a machine-specific interpreter path.
- dispatch already confines its working directory to the project root and an
  allowlist (``shared/mcp-servers/dispatch``), the pattern the palette server's
  writes reuse.
- Among 122 records in that project, 60 ``Depends`` links point to a newer
  record and 69 records form one dependency cycle, so dependency order and
  closure cannot be computed from its headers.

Goals and non-goals
-------------------

Goals:

- Every structural rule of RFC-0003 is checked by code.
- Every multi-file update is one tool call that completes in every file or
  changes none.
- Read-only tools are safe to pre-approve; write tools stay under each
  harness's approval.
- No dependency beyond the binary.

Non-goals:

- Writing prose for the agent; judging whether a decision is right.

Requirements and invariants
---------------------------

- Writes stay inside the project's ``_palette/`` and the paths its layout names,
  and inside the allowed roots.
- A file the server cannot parse is reported, never rewritten.
- Only the part of a file an operation concerns is changed; everything else,
  including the user's own edits, is kept byte for byte.
- A time-varying header field is written only by the server.

Design
------

The contract is ``spec/palette-server.rst`` in this record's changeset: project
resolution, the document model derived from the templates, the lint rules, the
read tools, the write tools and their transactions, and the command-line check.

Impact and compatibility
------------------------

- The installer registers a third server in every harness.
- A session without the server can still edit by hand; the next resume's lint
  reports what drifted.

Implementation and transition
-----------------------------

The server embeds the templates from ``shared/workflows/palette/templates/`` at
build time, so a template change and the checks derived from it ship together.

Verification strategy
---------------------

- Synthetic fixtures reproduce each failure listed in RFC-0003's evidence and
  each is reported by lint.
- Every write tool leaves every file unchanged when it fails partway.
- ``cargo test`` passes on Linux, macOS and Windows.

Alternatives and costs
----------------------

- Prose rules and a lint only: detects drift after the fact and leaves the
  multi-file update to the agent.
- A Python checker: depends on each machine's Python and docutils
  installation.

Open questions
--------------

None.

References
----------

- ``shared/workflows/palette/templates/``
- ``shared/mcp-servers/dispatch`` (project root and allowlist handling)
