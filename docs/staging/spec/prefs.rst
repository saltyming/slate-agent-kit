Prefs files
===========

:Status: Contract
:Date: 2026-09-30

Scope and authority
-------------------
Each installed kit has five prefs files in ``<home>/rules/``:
``<kit>--aside-prefs.md``, ``<kit>--dispatch-prefs.md``,
``<kit>--subagent-prefs.md``, ``<kit>--git-prefs.md`` and
``<kit>--comment-prefs.md``. Each starts with ``<!-- <kit>-custom:<name>-prefs -->``
and belongs to the user. Claude Code loads them with the other rule files; the
Codex and Kimi rules name them to be read when needed.

The templates in ``shared/prefs/`` are authoritative for the headings and
default values; the installer's schema is authoritative for the allowed values.

Definitions and model
---------------------
A setting is a heading ``## <Setting>`` followed by one value line
``**<value>**``. Every other line is free text that the installer never
changes. The installer edits a setting by replacing its value line; a file
written on Windows keeps its line endings.

Contract
--------
Settings
~~~~~~~~

Keys for ``--set`` are ``<file>.<key>``. Each key is stored under the heading
named after it below; a template that lacks a heading makes install fail with
a message naming the file and the heading. A blank value is written ``****``.

- aside: ``level`` Level; ``backend`` Backend; ``<backend>.model``
  "<Backend> model"; ``<backend>.effort`` "<Backend> reasoning effort";
  ``<backend>.fallback`` "<Backend> model fallback", for Codex and Claude.
- dispatch: ``level`` Level; ``backend`` Backend; ``model`` Model; ``effort``
  Reasoning effort; ``fallback`` Model fallback.
- subagent: ``level`` Level; ``model`` Default model; ``effort`` Reasoning
  effort.
- git: ``signing`` Commit signing; ``attribution`` Model attribution;
  ``commit-format`` Commit message format; ``pr-body`` PR body format;
  ``branch-naming`` Branch naming.
- comment: ``headers`` File headers; ``language`` Comment language;
  ``doc-comments`` Doc comments.

The allowed values:

aside
  ``level`` (``on-request``, ``suggest``, ``auto``; default ``suggest``);
  ``backend`` (``codex``, ``claude``); per backend ``model``, ``effort``
  (``low``, ``medium``, ``high``, ``xhigh``, ``max``, or blank) and
  ``fallback`` (a comma-separated model list, or blank), as
  ``aside.codex.model`` and so on. Only the chosen backend's values are asked.
  A file that still carries the settings of an earlier backend installs; the
  installer reports them in one warning line and treats them, and a
  ``backend`` value naming that backend, as blank.

dispatch
  ``level`` (default ``suggest``); ``backend`` (``codex``, ``opencode``,
  ``claude``); ``model``; ``effort`` (``low``, ``medium``, ``high``, ``xhigh``,
  or blank); ``fallback``.

subagent
  ``level`` (default ``suggest``); ``model`` (blank: the harness default;
  validated per spec/installer.rst *Native configuration*); ``effort`` (Codex
  and Kimi only).

git
  ``signing`` (``default``, ``no-gpg-sign``, ``unset``); ``attribution``
  (``on``, ``off``, ``unset``); ``commit-format`` (``conventional``,
  ``repository``, free text, ``unset``); ``pr-body`` (``summary-test-plan``,
  ``repository``, free text, ``unset``); ``branch-naming`` (``descriptive``,
  ``repository``, free text, ``unset``). ``unset`` means the agent asks the
  user when it first needs the value and records the answer.

comment
  ``headers`` (``repository``, ``structured``, free text); ``language``
  (``repository``, ``english``, ``korean``, free text); ``doc-comments``
  (``repository``, ``public-api``, free text).

Migration
~~~~~~~~~
Prefs written by claude-agent-kit 12.x or codex and kimi kits 0.7.x are
migrated when install or configure finds them, after the user confirms each
file (without a terminal: migrated). The old file is copied to
``<file>.bak-<UTC timestamp>``; the new file starts from the current template,
takes every value that has a new setting, and keeps the old file's ``Notes`` and
``Repository overrides`` sections verbatim, appending a section at the end when
the template has no heading for it.

- aside ``Auto-call policy``: ``conservative`` or ``preference-only`` becomes
  ``level`` ``on-request``; ``proactive`` becomes ``auto``. ``Preferred
  third-party advisor`` becomes ``backend`` (``none`` becomes ``codex`` with
  ``level`` ``on-request``).
- dispatch ``Execution policy`` and ``Approval mode``: ``conservative`` or
  ``preference-only`` becomes ``on-request``; ``proactive`` with ``ask`` becomes
  ``suggest``; ``proactive`` with ``auto`` becomes ``auto``. ``Default
  granularity`` is dropped.
- git and comment: every value carries over unchanged.
- subagent: new; created from the template.

Errors and edge cases
---------------------
- A value outside the allowed set is rejected at the prompt with the allowed
  values; a file edited by hand to such a value is reported by the agent when
  it reads the file and the default applies.
- A heading the template lacks is kept verbatim at the end of the migrated
  file.
- A file without a value line under a heading is reported by path and heading.

Ownership and ordering
----------------------
The user owns every prefs file; the installer writes only the value lines it
asked about and the migration writes a new file from the template with the old
values carried over. A current-turn instruction outranks a file's value.

Compatibility
-------------
Headings and values are stable within a major version of a kit; a renamed
setting ships with a migration, as the 12.x and 0.7.x policies did.

Conformance
-----------
``cargo test -p slate-setup`` checks that every template has a value line under
each heading, that the parser keeps every other line byte for byte, and that
each migration mapping produces the documented value.

References
----------
- ``spec/installer.rst`` for the configure step that writes these files.
- ``shared/prefs/`` for the templates.
