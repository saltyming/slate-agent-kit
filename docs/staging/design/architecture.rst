slate-agent-kit design
======================

:Status: Maintained
:Date: 2026-09-29

Purpose and scope
-----------------

slate-agent-kit is the single source of the agent kits for Claude Code, Codex
and Kimi Code: their rules, skills, document templates and prefs templates;
the shared MCP servers (aside, dispatch, palette); and the installer. Each
harness kit is a git submodule under ``kits/`` whose installable content is
rendered from this repository. The kits do not vendor slate; synchronization
is driven from here.

Architecture
------------

Sources
~~~~~~~

``shared/rules/core/``
  The kernel (the articles, in six parts) and the execution, delegation and
  git rule files, which hold only what the articles do not imply.

``shared/rules/mcp/``
  When to consult (aside) and when to dispatch.

``shared/workflows/palette/``
  The palette rule, the palette skills, and the document templates, which are
  also the schema the palette server checks against.

``shared/workflows/memory/``
  The memory-triage skill.

``shared/prefs/``
  The templates of the user-owned prefs files.

``adapters/<harness>/``
  ``tokens.sed`` (render-time values, including the kit version),
  ``inserts/*.md`` (harness-specific text for each insert marker) and, for
  Codex and Kimi, ``surface.md`` (how that harness loads the rules and reaches
  the servers).

``shared/crates/``
  The Rust libraries the servers share: agent-exec (one headless backend
  process invocation, from binary lookup to a run record with normalized
  token usage) and harness-log (session location and usage parsing for each
  backend's own output).

``shared/mcp-servers/``
  The Rust MCP servers: aside (consultation, on codex and claude), dispatch
  (external execution, on codex, claude and opencode), palette (document
  reads, checks and writes) and bench (model × effort × harness grids that
  record usage and list-price cost). aside, dispatch and bench run codex and
  claude through agent-exec and differ in the parameters they pass;
  dispatch's opencode runner is its own and reports through agent-exec's
  event types.

``shared/setup/``
  The Rust installer, ``slate-setup``.

``tooling/``
  ``render-kit.sh`` (renders a kit), ``validate.sh`` (checks sources and
  renders), ``install-mcp.sh`` (builds the servers and registers them through
  the installer), ``slate-version`` (the slate release a kit's binaries come
  from), and ``kit-scripts/`` (the entry-point and maintainer templates).

Render
~~~~~~

``tooling/render-kit.sh <harness>`` expands each insert marker from the
harness's inserts, substitutes the tokens, and writes the kit's ``dist/``: the
manual, the rule files in load order, the skills with the palette templates,
the prefs templates, and the ``kit.toml`` descriptor. It also renders the kit's
``install.sh``, ``install.ps1``, ``Makefile`` and the maintainer ``AGENTS.md``
at the kit root, and removes paths of the earlier layout. A missing insert file
is a hard error.

Install
~~~~~~~

A kit's entry point obtains ``slate-setup`` (prebuilt for the platform from the
slate release the descriptor names, or built from a slate checkout) and runs it
on the kit's ``dist/``. ``slate-setup`` asks its questions, shows a summary, and
then installs the files, the binaries and the server registrations, writes the
prefs and the harness's native subagent configuration, and records everything
in a manifest. Claude Code loads its rules folder file by file; Codex and Kimi
load one ``AGENTS.md``, which the installer writes as the manual followed by the
rule files and the user's custom rules.

Ownership and state
-------------------

- Rendered files (each kit's ``dist/`` and entry points) are never edited by
  hand; their sources are in this repository.
- A kit repository owns only its ``README.md``, ``CHANGELOG.md`` and
  ``LICENSE.md``.
- In a harness home, files whose first line carries the ``-custom:`` signature
  (prefs, custom rules) belong to the user; install and uninstall keep them
  unless the user chooses otherwise.
- A project's ``_palette/`` holds the user's internal palette documents; the
  palette server writes only there and at the project paths its layout names.

Execution and concurrency
-------------------------

- Each harness session starts its own stdio MCP server processes.
- dispatch allows one active run per working directory unless told otherwise.
- The palette server serializes writes to one project with a lock and applies
  each write to every affected file or to none.
- The installer replaces a binary by renaming a new file over it, so a server
  that is running keeps its old file.

Failure and recovery
--------------------

- Render stops on a missing insert or an unreadable source instead of writing a
  truncated file.
- ``validate.sh`` checks required sources, render completeness, insert
  integrity, harness leaks, retired terms, article identifiers, standing-corpus
  byte budgets, the entry points and descriptors, and this repository's own
  palette documents.
- Uninstall reverses what the manifest records and restores each configuration
  value only while it is still the one the installer wrote.

Interfaces and dependencies
---------------------------

- The installer's contract is ``spec/installer.rst`` and ``spec/prefs.rst``; the
  palette server's is ``spec/palette-server.rst``; the backend execution
  layer's is ``spec/agent-exec.rst``; the bench server's is ``spec/bench.rst``
  and the benchmark suite that feeds it is ``design/benchmark-suite.rst``;
  what each harness supports is
  `spec/support-matrix.rst <../spec/support-matrix.rst>`_.
- The Rust crates use ``rmcp`` for MCP over stdio and cross-build for every
  Linux target with cargo-zigbuild; the shared crates carry no C dependency,
  while dispatch alone carries the bundled SQLite and the TLS stack of its
  opencode client.

References
----------

- `glossary.rst <../glossary.rst>`_
- ``CLAUDE.md`` and ``AGENTS.md`` at the repository root (working on this
  repository, the release train)
