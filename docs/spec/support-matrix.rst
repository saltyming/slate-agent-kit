Harness support
===============

:Status: Contract
:Date: 2026-09-29

Scope and authority
-------------------

What each harness kit supports. An unsupported capability is stated
explicitly; a missing entry is a defect because it hides a scope decision. The
rendered kits under ``kits/`` and the adapters under ``adapters/`` are
authoritative where this list and they disagree, and the disagreement is a
defect to fix.

Definitions and model
---------------------

Each capability lists the three harnesses as ``Claude``, ``Codex`` and
``Kimi``.

Contract
--------

Rules
  Claude, Codex, Kimi: the kernel and every rule file, including the formal
  register rule and the memory invariant.

Rule delivery
  Claude: ``CLAUDE.md`` plus the rules folder, each file loaded. Codex and Kimi:
  one ``AGENTS.md`` the installer writes as the manual followed by every rule
  file and the user's custom rules.

Harness surface rule
  Claude: harness text lives in the inserts of the shared files. Codex:
  ``codex-surface``. Kimi: ``kimi-surface``.

palette
  Claude, Codex, Kimi: the palette rule, the palette skills and templates, and
  the palette MCP server.

Consultation (aside)
  Claude, Codex: shared MCP server reading the harness's own transcripts. Kimi:
  the same server through the local plugin.

Dispatch
  Claude, Codex: shared MCP server; backends codex, opencode, claude. Kimi: the
  same through the local plugin, which needs a workspace root.

Prefs
  Claude, Codex, Kimi: aside, dispatch, subagent, git and comment prefs files,
  written by the installer and owned by the user. Claude loads them with the
  rules; Codex and Kimi read them when first needed.

Subagent default model
  Claude: ``CLAUDE_CODE_SUBAGENT_MODEL`` in ``settings.json``, no effort
  setting. Codex: ``[agents] default_subagent_model`` and
  ``default_subagent_reasoning_effort``. Kimi: ``[secondary_model]``
  ``default_model`` (an alias of ``[models]``) and ``default_effort``.

Read-only subagents
  Claude: ``Explore``, ``Plan``, ``claude-code-guide``. Codex: none; a read-only
  opinion goes through aside. Kimi: ``Agent`` with ``explore`` or ``plan``.

Write-capable subagents
  Claude: every other ``Agent`` type, and ``Workflow``. Codex: ``spawn_agent``
  of any type. Kimi: ``Agent`` with ``coder``, and ``AgentSwarm``.

Native memory
  Claude: written by the agent, governed by the memory invariant. Codex:
  generated in the background from past sessions. Kimi: none.

Hooks
  Claude: Claude hooks. Codex: command hooks. Kimi: no default support.

Errors and edge cases
---------------------

None.

Ownership and ordering
----------------------

This list changes in the same change as the adapter, rule or installer that
changes what a harness supports.

Compatibility
-------------

None.

Conformance
-----------

``tooling/validate.sh`` checks that every harness renders every rule file,
skill and prefs template this list names.

References
----------

- `design/architecture.rst <../design/architecture.rst>`_
