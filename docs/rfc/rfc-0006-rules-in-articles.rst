RFC-0006: Rules in articles
===========================

:Status: Accepted
:Implementation: complete — the kernel, the rule files, the prefs templates
  and the validator of slate v0.7.0
:Verification: static — 2026-09-30; render, ``validate.sh`` and the rendered
  byte counts; no session evidence yet
:Areas: rule kernel; rule files; prefs templates; validator
:Authors: Claude Fable 5.1
:Reviewers: Hamin Sung
:Implementers: Claude Fable 5.1
:Accepted: 2026-09-30, Hamin Sung (decision made in conversation)
:Date: 2026-09-30
:Revised: none
:Depends: RFC-0001 (direction, autonomy and levels: the content the articles
  restate)
:Supersedes: none
:Related: RFC-0005 (memory discipline: the content of § 21)
:Changes: none
:Description: Every rule is one numbered article with a norm and a test;
  reasons, procedures and examples leave the standing corpus.

Summary
-------

The kit's rules are written as numbered articles (``§ 1`` to ``§ 21``) in six
parts. Each article states one norm in numbered clauses and ends with a test:
the observation that shows the article was broken. Reasons, worked examples,
trigger-phrase lists and step-by-step procedures are removed from the standing
corpus; what a procedure adds beyond its article is kept as one line in the
rule file. The identifier scheme ``INV-*`` / ``GATE-*`` is retired; an article
is cited by number, and numbers never move. The standing corpus falls from
about 45 KB to about 25 KB per harness.

Problem and context
-------------------

After RFC-0001 the standing corpus was still 45 KB per harness (62 KB in the
installed 12.x kit). The corpus loads into every session, and compliance with
any one rule falls as the corpus grows: the 2026-09-30 session that closed
phase 1 had every rule loaded and still reported unfinished deliverables in
the register of a completion report. The rules carried three kinds of text
that a model does not need in order to comply: the reason for a rule (useful
to a human editor, not to the reader who must apply it), procedures that the
rule already implies (a five-step git pre-flight, an undo checklist with six
items), and lists of example phrases that trigger a rule ("코드 확인 후
정한다", "figure out what needs changing").

The user asked for statute form: numbered articles, each stating the norm
without depending on the reader sharing the author's context, with names and
numbers to match.

Evidence
--------

- ``tooling/validate.sh`` on 2026-09-30, before this change: standing corpus
  claude 45294 B, codex 46061 B, kimi 44866 B. After: claude 24452 B, codex
  25433 B, kimi 24276 B.
- ``~/.claude`` on 2026-09-30 (kit 12.0.1): ``CLAUDE.md`` 15190 B plus rule
  and prefs files, 62008 B in all, of which the prefs files were 9439 B of
  mostly explanatory prose.
- ``shared/setup/src/prefs/doc.rs`` (2026-09-30): the prefs parser keys on
  ``## <heading>`` lines and the ``**value**`` line under each; the prose
  between them is not read.
- The 2026-09-30 phase-1 completion report, discussed with the user in the
  same session: the report placed three unfinished deliverables in a table and
  summarized the phase as finished.

Goals and non-goals
-------------------

Goals:

- One norm per article, one test per article, one definition per number.
- Nothing in the standing corpus that a model does not need to comply.
- Every reference to a rule, in the kits and in this repository, uses the
  article number.
- The prefs templates carry values and allowed values only.

Non-goals:

- Changing what the rules require. The articles restate RFC-0001 and
  RFC-0005; the one addition is § 3 (3), the planning-time disclosure of
  completion criteria only the user can authorize.
- A commentary document that restates each article's reason. The reasons
  live in RFC-0001, RFC-0005 and this record.

Requirements and invariants
---------------------------

- An article is ``**§ N Title.**`` followed by numbered clauses ``(1)``,
  ``(2)`` and a final ``Test:`` sentence. ``N`` is an integer, optionally with
  a lowercase letter suffix for an article inserted later.
- An article is defined once, in the kernel. Every other file cites it by
  number.
- Numbers never move. A new article takes the next free number in its part,
  or a letter suffix (``§ 6a``) when it must sit beside an existing one.
- No rendered file contains ``INV-`` or ``GATE-`` identifiers; the validator
  rejects them as retired terms.
- The standing corpus stays under the validator's byte budget for each
  harness; a budget is lowered when the corpus shrinks and never raised as a
  side effect of a change.

Design
------

Parts and articles:

- Part I, Direction: § 1 direction belongs to the user; § 2 the turn's mode;
  § 3 full delivery; § 4 documents advise, approval authorizes; § 5 deferred
  scope; § 6 deviation.
- Part II, Autonomy: § 7 judgment, not triggers; § 8 levels; § 9 operating
  envelope.
- Part III, State: § 10 no rollback by the agent; § 11 undo is a file edit;
  § 12 user-owned changes; § 13 destructive git only as named.
- Part IV, Delegation: § 14 one writer per file; § 15 delegates are bound.
- Part V, Verification and reporting: § 16 verify before claiming completion;
  § 17 faithful reporting.
- Part VI, Conduct: § 18 register; § 19 plain wording; § 20 context is not a
  stopping condition; § 21 memory holds only what has no other home.

Mapping from the retired identifiers:

- INV-DIR-1 → § 1; INV-DIR-2 → § 2; INV-SCOPE-1 → § 3; INV-AUTH-1 → § 4;
  GATE-SCOPE-CONFIRM → § 5; GATE-DEVIATION → § 6.
- INV-AUTO-1 → § 7; INV-AUTO-2 → § 8; INV-QUALITY-1 → § 9.
- INV-STATE-1 → § 10; INV-STATE-2 → § 11; INV-STATE-3 → § 12; GATE-GIT → § 13.
- INV-DELEG-1 → § 14; INV-DELEG-2 → § 15.
- INV-VERIFY-1 → § 16; INV-VERIFY-2 → § 17.
- INV-COMM-1 → § 18; INV-COMM-2 → § 19; INV-CTX-1 → § 20; INV-MEM-1 → § 21.

Additions and changes of substance, all discussed with the user on
2026-09-30:

- § 3 (3): a completion criterion that only the user can authorize (a push, a
  merge, an external run) is raised before work begins.
- § 17 (3): a completion report opens with what remains and what it waits on.
- § 19 (2): no praise of the agent's own work; the Claude binding adds that a
  style's insight blocks describe code, not the agent.
- The framework-conventions rule file (React, Rust, Python naming) is
  removed: it stated what the models already know and what each repository
  decides for itself.
- § 4 (3), added after the 2026-09-30 review: the compression had dropped the
  general "plan, get approval, implement" checkpoint of the earlier kernel's
  quick reference; it is restored as a clause of § 4.

The rule files keep only what the articles do not imply: the reading order,
the planning outline, the refactoring and comment conventions, the undo
reconstruction steps, the list of destructive git commands, the delegation
prompt contents, the aside and dispatch settings, and the palette families and
loop. The prefs templates keep the signature comment, the ``## <heading>``
lines, the ``**value**`` lines, and one line of allowed values per heading.

Impact and compatibility
------------------------

- Every kit README, CHANGELOG and skill that cited an ``INV-*`` or ``GATE-*``
  identifier cites the article instead. This record holds the mapping; the
  kit CHANGELOG entries repeat it.
- The installed prefs files are user-owned; the installer's upgrade keeps
  their values and rewrites only the template's prose around them.
- The removed conventions rule file is a kit-signed file; the installer's
  stale-file step removes it on upgrade.
- ``tooling/validate.sh`` checks article integrity (every cited ``§ N`` has
  exactly one definition) and treats ``INV-`` and ``GATE-`` as retired terms.
  Its byte budgets are 26000 (claude), 27000 (codex) and 26000 (kimi).

Implementation and transition
-----------------------------

The kernel, rule files, prefs templates, adapter inserts, validator and render
script change together in slate; the kits are re-rendered and their READMEs
and CHANGELOGs updated in the same release. There is no runtime migration.

Verification strategy
---------------------

- ``sh tooling/validate.sh`` reports ``validate: OK`` with the article
  integrity check and the retired-term check in force.
- The rendered standing corpus of each harness is under its budget.
- The installer's tests pass with the trimmed templates (they read the value
  line under each heading).
- A later session working under the articles reports unfinished work first
  and raises user-gated completion criteria at planning time; a session that
  does not is a finding against § 17 or § 3, not against this record.

Alternatives and costs
----------------------

- Keep the ``INV-*`` / ``GATE-*`` identifiers and only shorten the text. The
  identifiers encoded a category (scope, state, gate) that the parts now
  carry, and two kinds of rule (invariant, gate) that the articles no longer
  distinguish. Renaming once is cheaper than carrying a second vocabulary.
- Move the reasons into a loaded commentary file. Rejected: a loaded file
  costs what it saves. The reasons stay in the records.
- Cut to 20 KB. Reaching it would remove clauses that state what the rule
  covers (the envelope evidence, the delegation prompt contents); the user
  can ask for that cut once the articles have been used in sessions.

Open questions
--------------

None.

References
----------

- `RFC-0001 <rfc-0001-direction-autonomy-levels.rst>`_
- `RFC-0005 <rfc-0005-memory-discipline.rst>`_
- ``shared/rules/core/kernel.md``, ``tooling/validate.sh``
