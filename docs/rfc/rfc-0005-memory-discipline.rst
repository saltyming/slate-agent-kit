RFC-0005: Memory discipline
===========================

:Status: Accepted
:Implementation: complete — the memory invariant, its harness bindings and
  the memory-triage skill
:Verification: static — 2026-09-30; render and ``validate.sh``; this repository's
  memory triaged to zero files on 2026-09-30
:Areas: rule kernel; native memory
:Authors: Claude Opus 5.5
:Reviewers: Hamin Sung
:Implementers: Claude Opus 5.5 (2026-09-29), Claude Fable 5.1 (2026-09-30, triage)
:Accepted: 2026-09-29, Hamin Sung (decisions made in conversation)
:Date: 2026-09-29
:Revised: none
:Depends: RFC-0001 (a correction that is a rule goes to the user as rule text)
:Supersedes: none
:Related: none
:Changes: none
:Description: Native memory holds only facts no other place can hold; a
  correction is applied, or proposed as rule text, instead of being stored; a
  skill proposes cleanup and the user decides.

Summary
-------

The kit states where a fact belongs before it may go to native memory, routes
corrections that are rules to the rule files as proposals, and ships a skill
that proposes memory cleanup without deleting anything on its own.

Problem and context
-------------------

Every small correction became a native-memory file. Memory held rules that
compensated for kit defaults project by project, so cleaning memory up removed
the compensation and the corrected mistake came back; the agent then proposed
another memory for it. The harness's own memory instructions invite saving
"corrections and confirmed approaches" and give no route from a correction to a
rule file.

Evidence
--------

- Measured 2026-09-29 in a project using palette: 85 memory files, 18 of them
  about agents and models. After a cleanup removed memories that restricted
  delegate models, the agent started five delegates on the session's top model,
  and in the same session proposed a new memory to prevent it.
- That project's own memory policy already listed what not to store (code
  descriptions, progress, temporary conditions, anything already in a rule
  file), and stated that project-wide working rules belong in its instruction
  file and the kit's rules, but gave no step for a correction that is such a
  rule.
- Codex generates its Memories in the background from past sessions; Kimi Code
  has no native memory (checked 2026-09-29 against their documentation and
  installed builds).

Goals and non-goals
-------------------

Goals:

- A placement order that puts memory last.
- A route for corrections: applied, proposed as project rule text, or proposed
  as kit rule text.
- A cleanup that proposes, with reasons, and leaves the choice to the user.

Non-goals:

- Cleaning up any particular project's memory; that is done with the skill in
  that project.

Requirements and invariants
---------------------------

- Nothing in memory is deleted without the user's choice.
- A memory is marked covered only when the covering rule can be quoted.

Design
------

The kernel carries one invariant: the placement order (code, maintained
documents, rule files, palette, memory last); corrections that concern the
current task are applied, rule-shaped ones are proposed as rule text; the list
of what memory never holds; one rule or fact per memory, revised in place,
merged by trigger. Each harness's binding narrows or explains its own memory
surface. The ``memory-triage`` skill classifies memories as keep, promote,
covered, revise, merge or delete and applies only what the user chooses.

Impact and compatibility
------------------------

- Fewer memories are written; corrections that are rules reach the rule files
  after the user accepts the text.
- Existing memories are untouched until the user runs the skill.

Implementation and transition
-----------------------------

The invariant, the bindings and the skill ship in the same release as the
action levels, because the rules that replace delegate-model memories are part
of that release.

Verification strategy
---------------------

- Run on a copy of a memory folder, the skill produces a classified list with a
  reason for each memory and changes nothing.
- Each render carries the invariant and its harness binding.

Alternatives and costs
----------------------

- Keep memory rules per project: each project reinvents them, and the harness's
  invitation to save corrections is left unchanged.
- Automatic cleanup: deletes rules that were compensating for missing kit rules,
  which is how the regression happened.

Open questions
--------------

None.

References
----------

- ``shared/rules/core/kernel.md`` (the memory invariant)
- ``shared/workflows/memory/skills/memory-triage/SKILL.md``
