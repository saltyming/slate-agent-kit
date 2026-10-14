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

Each capability lists the four harnesses as ``Claude``, ``Codex``, ``Kimi``
and ``Agy`` (the Antigravity CLI, ``agy``).

Contract
--------

Rules
  Claude, Codex, Kimi, Agy: the constitution and every rule file, including
  the formal register rule and the memory invariant.

Rule delivery
  Claude: ``CLAUDE.md`` plus the rules folder, each file loaded. Codex and Kimi:
  one ``AGENTS.md`` the installer writes as the manual followed by every rule
  file and the user's custom rules. Agy: ``~/.gemini/config/AGENTS.md`` plus
  ``~/.gemini/config/rules/``, each file loaded when it opens with the YAML
  frontmatter block Antigravity requires (``trigger: always_on``, a quoted
  ``description``), which the render emits in front of the signature of every
  rule and prefs file and the installer adds to a custom rule; the manual
  carries none. ``~/.gemini/config`` is shared with the Antigravity
  application.

Harness surface rule
  Claude: harness text lives in the inserts of the shared files. Codex:
  ``codex-surface``. Kimi: ``kimi-surface``. Agy: ``agy-surface``.

palette
  Claude, Codex, Kimi, Agy: the palette rule, the palette skills and templates,
  and the palette MCP server.

Consultation
  Claude, Codex, Agy: the dispatch server's ``dispatch_consult``, on codex and
  claude, reading the harness's own transcripts (for Agy, the workspace's
  latest Antigravity conversation). Kimi: the same through the local plugin.
  agy is not a consultation backend: ``dispatch_consult`` refuses
  ``backend: agy``.

Dispatch
  Claude, Codex, Agy: shared MCP server; execution backends codex, opencode,
  claude and agy. The agy backend runs the Antigravity CLI headless with
  permissions skipped under the agent guard (``--sandbox`` at the
  workspace-write level; the read-only level is refused), the prompt on stdin
  as a stream-json event, the outcome read from the ``result`` event and a
  denied tool from its ``denied_actions``. Kimi: the same through the local
  plugin, which needs a workspace root.

Waiting on a run
  Claude: ``Bash`` with ``run_in_background``, which re-invokes the session
  when ``dispatch wait`` exits. Codex: ``exec_command`` yields after at most
  30 s with ``dispatch wait`` still running, and the process is then polled
  with empty ``write_stdin`` calls of up to ``background_terminal_max_timeout``
  (300 s by default), each returning when the process exits. Kimi:
  ``Shell`` with ``run_in_background`` and ``disable_timeout``, which notifies
  the session when the command completes. Agy: ``run_command`` returns within
  10 s with ``dispatch wait`` still running as a background task, and the
  harness notifies the session when it exits.

Prefs
  Claude, Codex, Kimi, Agy: consult, dispatch, subagent, git and comment prefs
  files, written by the installer and owned by the user. Claude and Agy load
  them with the rules; Codex and Kimi read them when first needed.

Subagent default model
  Claude: ``CLAUDE_CODE_SUBAGENT_MODEL`` in ``settings.json``, no effort
  setting. Codex: ``[agents] default_subagent_model`` and
  ``default_subagent_reasoning_effort``. Kimi: ``[secondary_model]``
  ``default_model`` (an alias of ``[models]``) and ``default_effort``. Agy:
  none; the subagent prefs model is a value the agent passes to
  ``define_subagent``.

Model guidance
  Claude, Codex, Kimi, Agy: the ``models`` rule file, loaded after the
  delegation rule, which says which model and effort a subagent, a dispatch
  step or a consultation runs on where the user, the prefs or the call has not
  fixed them. Its table lists the harness vendor's own models only. Claude:
  Fable 5.1, Opus 5.5, Sonnet 5.5, Haiku 5.5. Codex: ``gpt-6-astra``,
  ``gpt-6.1-sol``, ``gpt-6-luna``. Kimi: ``k3``, ``k3-256k``,
  ``kimi-for-coding``, ``kimi-for-coding-highspeed``. Agy: the models
  ``agy models`` lists (``claude-opus-5-5``, ``claude-sonnet-5-5``,
  ``gemini-3.8-flash``, ``gpt-oss-120b-medium``, each Claude and Gemini slug
  also with a ``-low``, ``-medium`` or ``-high`` suffix).

Read-only subagents
  Claude: ``Explore``, ``Plan``, ``claude-code-guide``. Codex: none; a read-only
  opinion goes through ``dispatch_consult``. Kimi: ``Agent`` with ``explore``
  or ``plan``. Agy: none; a tools allowlist narrows a subagent but is not a
  read-only kind, so a read-only opinion goes through ``dispatch_consult``.

Write-capable subagents
  Claude: every other ``Agent`` type, and ``Workflow``. Codex: ``spawn_agent``
  of any type. Kimi: ``Agent`` with ``coder``, and ``AgentSwarm``. Agy:
  ``define_subagent`` and ``invoke_subagent``, with ``manage_subagents``.

Native memory
  Claude: written by the agent, governed by the memory invariant. Codex:
  generated in the background from past sessions. Kimi: none. Agy: knowledge
  items, governed by the memory invariant.

Hooks
  Claude: Claude hooks. Codex: command hooks. Kimi: no default support. Agy:
  ``hooks.json``, not used by the kit.

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
