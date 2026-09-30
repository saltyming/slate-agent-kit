RFC-0002: Installer and kit layout
==================================

:Status: Accepted
:Implementation: complete — the Rust installer, the kit payload layout, the
  prefs levels and their migration, native subagent configuration
:Verification: build — 2026-09-30; cargo test on macOS and
  scratch-HOME installs, reinstalls and uninstalls of the three kits, Linux and
  Windows through slate CI on next (run 36659672290, green)
:Areas: installer; kit layout; prefs; MCP registration
:Authors: Claude Opus 5.5
:Reviewers: none yet
:Implementers: Claude Opus 5.5 (2026-09-29 to 2026-09-30)
:Accepted: 2026-09-29, Hamin Sung (decisions made in conversation)
:Date: 2026-09-29
:Revised: none
:Depends: RFC-0001 (the action levels the prefs record)
:Supersedes: none
:Related: none
:Changes: spec/installer.rst (created); spec/prefs.rst (created)
:Description: One Rust installer, fetched and run by thin entry points, installs
  every kit, registers the servers, writes prefs and native configuration, and
  installs from a payload folder separate from each kit's own instructions.

Summary
-------

Each kit installs through a single Rust program in the slate workspace. The
kits' ``install.sh``, ``install.ps1`` and ``Makefile`` only obtain and run it.
The installable files of a kit move into a ``dist/`` folder so that the kit's
root carries the instructions for maintaining the kit, not the payload.

Problem and context
-------------------

The kits carry nine installer implementations (``install.sh``, ``Makefile`` and
``install.ps1`` in each of three kits), maintained by hand. They have drifted:
the order of steps, the handling of custom rules, the uninstall prompts and the
supported overrides differ by kit and by operating system. Configuration edits
are spread over awk (Codex TOML), node (the Kimi plugin), python3 or jq (Claude
``settings.json``) and PowerShell, and the PowerShell side never received the
Codex edits. Prompts accept any text, ask about backends the user did not
choose, and end without a summary.

A kit's root ``CLAUDE.md`` or ``AGENTS.md`` is the file installed into the
user's harness home. A session working inside the kit's repository loads it as
that repository's own instructions, so a Codex or Kimi manual governs work on
the kit itself.

Evidence
--------

- ``kits/codex-agent-kit/install.ps1`` registers aside with ``codex mcp add``
  only; ``tooling/install-mcp.sh`` (``configure_codex``) also sets
  ``tool_timeout_sec``, ``default_tools_approval_mode`` and the code-mode
  namespace lists. Without them an aside call on Codex fails.
- ``kits/codex-agent-kit/Makefile``: ``install`` rewrites ``AGENTS.md`` without
  the user's custom rules; ``configure`` appends them again on every run.
- ``kits/claude-agent-kit/install.sh`` documents ``CLAUDE_DIR`` and ``BIN_DIR``
  overrides in its help text and assigns both unconditionally.
- ``tooling/kit-scripts/configure-prefs.sh`` prompts eleven aside questions in a
  row with no validation, including models for backends the user did not pick.
- Binaries go to ``~/.local/bin`` on Unix and to ``%USERPROFILE%\.local\bin``
  (Claude) or ``%CODEX_HOME%\slate-agent-kit\bin`` (Codex) on Windows.
- Harness-native subagent defaults: Claude Code ``CLAUDE_CODE_SUBAGENT_MODEL``
  (https://code.claude.com/docs/en/sub-agents.md); Codex ``[agents]``
  ``default_subagent_model`` and ``default_subagent_reasoning_effort``
  (https://learn.chatgpt.com/docs/config-file/config-reference, and present in
  the Codex 0.157.0 binary); Kimi Code ``[secondary_model]`` with
  ``default_model`` naming an alias of ``[models]``, where an unknown alias makes
  the configuration invalid and session startup fails
  (https://www.kimi.com/code/docs/en/kimi-code-cli/configuration/config-files.html).

Goals and non-goals
-------------------

Goals:

- One implementation of every installer step for all three harnesses and all
  supported operating systems.
- Prompts that are the same across harnesses, validated, conditional on earlier
  answers, and followed by a summary to confirm.
- Prefs that record the action levels, migrated from the old values in every
  harness.
- An optional subagent default model, written to each harness's native
  configuration after validation.
- A kit repository whose root instructions describe maintaining the kit.

Non-goals:

- The rule text that reads the prefs.
- The palette server's own behavior; the installer only installs and registers
  it.

Requirements and invariants
---------------------------

- Files the user owns (``-custom:`` signature, prefs, custom rules) are never
  overwritten or removed without the user's confirmation.
- A configuration file the installer edits keeps every key, comment and
  formatting it did not change.
- Every change is recorded in the kit's manifest so uninstall can reverse
  exactly what install did.
- Install and uninstall behave the same on Linux, macOS and Windows.

Design
------

The installer is a Rust crate in the slate workspace, shipped prebuilt for the
same platforms as aside and dispatch. It reads a per-kit descriptor rendered by
slate, installs the kit's payload, obtains and registers the MCP servers, runs
the prefs wizard and writes native configuration. The kit entry points download
the installer for their platform (or build it from a slate checkout) and hand it
the payload. The exact contract is ``spec/installer.rst`` and ``spec/prefs.rst``
in this record's changeset.

Impact and compatibility
------------------------

- The one-line install commands keep their URLs; what they fetch changes.
- The payload paths inside each kit repository change (``dist/``); scripts
  cached from an earlier release stop working, which the major version bump
  announces.
- node is no longer required for Kimi; python3 and jq are no longer used.
- The dispatch prefs lose the granularity setting: under ``suggest`` the agent
  groups its proposals itself.

Implementation and transition
-----------------------------

The installer, the payload layout, the rendered entry points and the rule text
that names the new prefs ship in one release. An upgrade from 12.x (claude) or
0.7.x (codex, kimi) migrates the prefs after the user confirms, and removes
binaries and registrations the earlier installers left in other locations.

Verification strategy
---------------------

- ``cargo test`` on Linux, macOS and Windows covers every configuration edit
  against fixture files, including comment preservation and invalid input.
- Install, reinstall over the previous release, configure and uninstall run
  for all three harnesses against an isolated ``HOME`` with stand-in harness
  CLIs.
- The same answers produce the same prefs files in all three harnesses.

Alternatives and costs
----------------------

- One engine in POSIX sh and one in PowerShell: the two implementations drift,
  as the current nine did; prompts and validation have to be written twice.
- sh and PowerShell engines with a Rust helper for configuration edits only:
  fixes the edits but keeps two prompt implementations.
- Cost of the chosen design: the first installer step downloads a binary; a
  platform without a prebuilt binary builds it with cargo.

Open questions
--------------

None.

References
----------

- ``docs/glossary.rst`` (level, subagent)
- ``tooling/install-mcp.sh``, ``tooling/kit-scripts/`` (current behavior)
