RFC-0001: Direction, autonomy and action levels
===============================================

:Status: Accepted
:Implementation: complete — the kernel's decision rights, judgment-based
  consultation, dispatch and delegation rules, and the three action levels
:Verification: static — 2026-09-30; render and ``validate.sh`` (retired-term and
  article checks); no session evidence yet
:Areas: rule kernel; delegation; consultation; dispatch
:Authors: Claude Opus 5.5
:Reviewers: Hamin Sung
:Implementers: Claude Opus 5.5 (2026-09-29), Claude Fable 5.1 (2026-09-30, articles)
:Accepted: Hamin Sung (2026-09-30T01:26Z)
:Date: 2026-09-29
:Revised: none
:Depends: none
:Supersedes: none
:Related: none
:Changes: none
:Description: The user sets direction, the agent acts autonomously under it,
  prohibitions stay a short fixed list, and consultation, dispatch and
  subagents share three action levels judged case by case.

Summary
-------

The kit's rules state which decisions belong to the user and which belong to
the agent, instead of listing conditions under which the agent must act.
Consultation, dispatch and native subagents share one scale of three levels
(``on-request``, ``suggest``, ``auto``), and at every level the agent judges
whether the action is worth its cost now.

Problem and context
-------------------

The rules tell the agent to act when a condition holds. Current models follow
such text literally, so the agent acts without asking whether the action helps
in the situation at hand. The aside rule makes a call mandatory "whether or not
the user asked" and forbids reconsidering it; the delegation rule calls
read-only delegates free and prefers them for any lookup beyond about three
queries, counting only the leader's context as a cost. No rule names the model a
delegate runs on, how many run at once, or who decides whether a turn is for
discussion or execution.

The prohibitions that protect the user (erasing work, overwriting the user's
changes, destructive git without a named command) are already a short list.
What is missing is a statement of which decisions belong to the user, so the
agent can recognize one in a situation no rule lists.

Evidence
--------

- ``shared/rules/mcp/aside.md``, proactive policy: "Call the preferred backend,
  whether or not the user asked, when any of these happens", and "Once you have
  decided to call the native advisor, do not reconsider whether to pair it with
  aside. 'This is not high-stakes after all' is the failure this rule exists to
  stop."
- ``shared/rules/core/loop-delegation.md``, read-only delegates: "Use them
  without asking", and "For a read-only lookup larger than about three search
  queries, prefer a delegate to searching inline".
- Observed 2026-09-29 in a project that uses palette: asked to read the state
  document and continue the discussion, the agent started five read-only
  delegates on the session's top model to verify a document the state assigned
  to the leader. Claude Code's Explore delegates inherit the session model, and
  no rule mentions model or count.
- Observed 2026-09-29 in this repository: while the user was discussing and
  waiting for the leader's own synthesis, the agent started an aside
  consultation because the proactive policy lists "an architecture decision" as
  a trigger. The user rejected the call.
- Claude Code resolves a subagent's model from the per-call ``model``
  parameter, then the agent definition's ``model``, then
  ``CLAUDE_CODE_SUBAGENT_MODEL``, then the parent's model
  (https://code.claude.com/docs/en/sub-agents.md).

Goals and non-goals
-------------------

Goals:

- State the decision rights of user and agent in the kernel.
- Replace every trigger list with a purpose and a value test.
- One scale of action levels for consultation, dispatch and subagents.
- Make the turn's mode the user's decision.

Non-goals:

- Rules for native memory and for palette documents.
- The prefs file format and the migration of existing prefs.

Requirements and invariants
---------------------------

- Every existing prohibition keeps its effect: no erasing or hiding of work the
  agent created, no overwriting of changes the user made, no destructive git
  without the named command.
- Only the user's approval authorizes execution.
- One writer per file, and every delegate inherits every invariant.
- No rule makes an action mandatory on a condition alone, and no rule forbids
  reconsidering an action.

Design
------

Decision rights
~~~~~~~~~~~~~~~

The user sets direction: what to do and why, scope, priority, trade-offs and
done criteria. The agent decides the rest under that direction: method, order,
tools, and whether to delegate or consult within the configured level. An action
that would change direction goes to the user before it happens: a different
scope, design or order than approved; giving work to someone other than the plan
or a document names; using a surface beyond its level. Prohibitions are a short
fixed list that does not grow per incident.

Judgment
~~~~~~~~

A rule states an action's purpose and how to judge its value. The value test for
consulting, dispatching and delegating asks whether the action could change a
decision not yet made, whether the user is already performing that role, and
whether its cost in models, count, quota and time is proportionate to what it
can change.

Action levels
~~~~~~~~~~~~~

Each surface has one level. ``on-request``: used only when the user asks.
``suggest``: the agent proposes it in one line and waits. ``auto``: the agent
applies the value test, uses it, and states it in one line (count, model,
purpose). The level replaces the aside auto-call policy, the dispatch execution
policy and the dispatch approval mode. Write-capable delegation is a use of a
subagent or of dispatch at its level; the separate delegation gate goes away.

Turn mode
~~~~~~~~~

The user decides whether a turn is for discussion or execution. In a discussion
the agent reads, reports, proposes and waits. A next action written in a
document is a proposal, never a trigger.

Impact and compatibility
------------------------

- Rule text shortens: trigger lists and their exceptions become a purpose and a
  value test, which lowers the standing corpora's size.
- Behavior changes for existing users: a surface configured as proactive now
  judges each use. Existing prefs values have to be migrated to the levels in
  all three harnesses.
- Harness bindings keep their meaning: Codex's native subagents are
  write-capable, Kimi's ``explore`` and ``plan`` subagents are read-only; both
  are subagents at the configured level.

Implementation and transition
-----------------------------

The decision is implemented in the toolkit's rule sources:
``shared/rules/core/kernel.md``, ``shared/rules/core/loop-execution.md``,
``shared/rules/core/loop-delegation.md``, ``shared/rules/mcp/aside.md``,
``shared/rules/mcp/dispatch.md`` and each harness's ``adapters/<harness>/inserts/``.
The rule text, the prefs templates and their migration ship in one release
across the three kits, because a kit whose rules name the new levels cannot read
prefs that still hold the old values.

Verification strategy
---------------------

- ``tooling/validate.sh`` rejects a render that contains a trigger phrase this
  RFC removes ("whether or not the user asked", "do not reconsider", "Use them
  without asking").
- Each render states the decision rights, the value test and the three levels.
- A scripted scenario in each harness: a discussion request in a palette project
  ends with a report and a proposal, and starts no delegate or consultation.

Alternatives and costs
----------------------

- Keep the trigger lists and add exceptions: each exception is another trigger,
  and the list grows per incident.
- Autonomy as "everything except prohibitions": the actions that went wrong
  were allowed ones that needed the user first; expressing them as prohibitions
  makes the list grow.
- A finer scale (off, conservative, preference-only, proactive, approval
  modes): more levels than a user needs, each needing its own rule text.

Open questions
--------------

None.

References
----------

- ``docs/glossary.rst`` (direction, autonomy, prohibition, level, delegate,
  subagent, consultation)
