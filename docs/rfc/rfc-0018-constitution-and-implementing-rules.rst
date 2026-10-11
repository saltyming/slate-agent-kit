RFC-0018: The manual is a constitution with a preamble; the rules implement it
===============================================================================

:Status: Accepted
:Implementation: complete — the constitution and the implementing rules, the
  claude surface rule, the contract field, the three kits at 26.10.0 and the
  slate release of the same number
:Verification: build — 2026-10-11; the three kits rendered, validate.sh with
  the raised budgets, cargo test and clippy on stable, palette check, and an
  upgrade of each kit from its previous release into a scratch HOME; the
  effect of the preamble and of § 19 (4) on replies in a live session is not
  observed yet (Verification strategy)
:Areas: rules; adapters; render and validate; installer; kit releases; specs
:Authors: Claude Fable 5.1
:Reviewers: Hamin Sung; GPT-6 Astra (high)
:Implementers: Claude Fable 5.1
:Accepted: Hamin Sung (2026-10-11T02:25Z)
:Date: 2026-10-11
:Revised: none
:Depends: RFC-0001 (the direction, autonomy and levels the articles restate);
  RFC-0002 (the kit payload, the render and the descriptor); RFC-0006 (the
  articles, their numbering and the one-definition rule, which this record
  keeps)
:Supersedes: RFC-0006 (in part: the requirement that reasons and examples
  leave the standing corpus, and the budgets; the preamble is a reason, and
  § 19 (4) carries one example)
:Related: RFC-0005 (memory discipline: the content of § 21, whose clauses
  (2)–(4) move to the execution rule); RFC-0017 (the models rule, whose
  budgets this record raises again)
:Changes: spec/installer.rst (Descriptor; Compatibility);
  spec/prefs.rst (Compatibility); spec/agent-exec.rst (Compatibility);
  spec/palette-server.rst (Compatibility); spec/support-matrix.rst (Contract);
  design/architecture.rst (Sources); contributing.rst (Branches);
  glossary.rst (Direction and authority; Retired terms)
:Description: The kit's manual opens with a preamble stating the intent the
  articles derive from, § 19 gains a clause on how a reply leaves the decision
  with the user, the mechanism-heavy articles are trimmed to norm and test
  with their procedure in the implementing rules, every harness gets a surface
  rule so the constitution is one text, and the three kits and the slate
  release share one calendar version with a separate contract number.

Summary
-------

The manual a kit installs is a constitution: a preamble, then the articles
``§ 1`` to ``§ 21`` as RFC-0006 defined them, each a norm with its test. The
preamble states what the articles derive from and ends with its own limit: it
guides interpretation where the rules leave a choice, overrides no article,
grants no authorization and changes nothing the harness reserves to itself.
``§ 19`` gains clause (4): an answer is not approval of the agent's next
action; a proposal carries its grounds, uncertainty and trade-off, in a
deliberative reply before the course of action; no command, label or emphasis
presents an unapproved proposal as settled. The articles whose text was mostly
procedure (``§ 8``, ``§ 11``, ``§ 13``, ``§ 21``) keep their obligations and
tests; the procedure moves to the implementing rules, where most of it already
stood. The sources move to ``shared/rules/constitution.md`` and
``shared/rules/implementing/``; every harness, Claude included, has a
``surface.md`` carrying its bindings, so the constitution renders to the same
text in every kit. The three kits and the slate release share one number,
``26.10.0``, and the descriptor gains ``contract = 1``, which alone speaks for
compatibility. The standing-corpus budgets rise to 31500, 31500 and 30500
bytes.

Problem and context
-------------------

In sessions under the 13.3.x rules the user observed two things: the agent
bolded its own proposed action and stated a verdict before its grounds, which
read as pressure, and its replies missed the question asked in favour of what
it meant to do next. ``§ 1`` says direction belongs to the user, but the agent
applied it to scope and files and never to the form of a reply. In the
2026-10-11 session the agent also deferred an approved stage of this very
change to "a separate decision", a reduction ``§ 1`` and ``§ 3`` forbid in
words; the explicit article did not hold on its own either.

RFC-0006 removed every reason from the standing corpus on the ground that a
model does not need reasons to comply, and recorded "no session evidence yet".
The user wants the kit to work the way a constitution does: the intent stated
once, the articles as its tests, and the procedure in a layer below. The user
also wanted the differences between the Claude kit and the other two removed
in the same release: a different version line, a rule file named
``parallel-work`` where the others say ``delegation``, and harness bindings
spread through the Claude manual as inserts while Codex and Kimi keep theirs
in a surface file.

Evidence
--------

- ``tooling/validate.sh`` on 2026-10-11 before this change: standing corpus
  claude 28352 B of 28500, codex 29057 B of 29500, kimi 28234 B of 28500.
  After: claude 31049 B, codex 31179 B, kimi 30381 B. The Claude increase by
  file: ``CLAUDE.md`` +36 (preamble about +1050, § 19 (4) about +600, the
  inserts moved out −1145, the trimmed articles and the old format note
  −470); the new surface rule 1929 (1145 moved in, the rest its frame); git
  +531 (the destructive-git list moved in); delegation +167; execution +28.
- ``shared/rules/core/kernel.md`` before the change: 21 articles with norms
  and tests and no stated reason; ``loop-execution.md`` already carried the
  undo steps and the destructive-git list that ``§ 11`` and ``§ 13`` also
  carried.
- The review by GPT-6 Astra (high) on 2026-10-11, through aside: the first
  preamble draft contradicted ``§ 7`` ("the person holds every decision"),
  ``§ 3`` ("changes nothing not approved") and ``§ 11`` (removal without the
  user's undo), and its closing clause "where the articles are silent, the
  intent decides" would let intent override an explicit restriction; the first
  § 19 (4) draft forbade every imperative and demanded alternatives in every
  reply; a purpose does not derive a policy uniquely, so a derivation table
  cannot decide what leaves the constitution. All four points are adopted
  here.
- Anthropic's description of its constitution and of training on reasons
  (alignment.anthropic.com, "Teaching Claude why", 2026) concerns training;
  it does not show that a runtime preamble improves compliance. The preamble
  is therefore a hypothesis this record states and the Verification strategy
  tests; it is not claimed as established.
- ``docs/spec/{prefs,installer,agent-exec,palette-server}.rst`` on 2026-10-11:
  four Compatibility sections tied stability to "a major version of a kit".
- ``git`` on 2026-10-11: a ref name cannot contain ``:``, so an epoch prefix
  (``1:1.0.0``) cannot be a tag; ``release.yml`` publishes on ``v*`` tags;
  ``slate-setup`` compares no version strings.

Goals and non-goals
-------------------

Goals:

- A preamble that states the intent and its own limit, in short declarative
  sentences a Codex or Kimi model reads the same way as a Claude model.
- ``§ 19 (4)``: how a reply leaves the decision with the user, with one
  example pair and matching test clauses.
- Every article still defined once, in the constitution, as norm and test;
  the procedure in the implementing rules; no obligation lost in the move.
- One constitution text for the three harnesses; each harness's bindings in
  its surface rule.
- One version for the three kits and the slate release; compatibility named
  by a contract number.

Non-goals:

- Changing what any article requires, beyond ``§ 19 (4)``.
- A behavioural comparison of the variants before release (Verification
  strategy).
- Reducing the standing corpus; the budgets rise, and the reason is recorded.

Requirements and invariants
---------------------------

- The preamble guides interpretation where the articles and implementing
  rules leave a choice. It overrides no article, grants no authorization, and
  changes nothing the harness reserves to itself; the text says so.
- Every ``§ N`` keeps one bold definition in the constitution with a
  ``Test:`` sentence; numbers do not move (RFC-0006).
- An obligation that leaves an article's text is stated in an implementing
  rule, which loads with the constitution in every harness; none moves to a
  skill, which loads only when used.
- The constitution carries no ``{{@INSERT}}`` marker; harness text lives in
  ``adapters/<harness>/surface.md``.
- The descriptor's ``contract`` is an integer; a descriptor without it is
  contract 1; the installer records it in the manifest.
- The release number is ``YY.MM.N`` and the same for the three kits, the
  slate release and the workspace crates; the contract number rises only on an
  incompatible change of the descriptor, manifest, prefs or server interfaces.

Design
------

Constitution. ``shared/rules/constitution.md``: title and version, the
Preamble, one line on the article form, the File Map, the six Parts, Working
together. It renders to the primary file of each kit.

Preamble, as rendered::

  This manual binds an agent working under a person's direction. The person
  sets what is done, in what order, what counts as done and which choices stay
  theirs; within that, the agent chooses method, tools and the order of its own
  work, and does what the approved scope needs without asking again. The agent
  delivers the approved scope whole, verifies before it calls anything done,
  and reports the state as the files would show it. It changes nothing outside
  what was approved, and removes nothing it made except at the person's word or
  by putting the approved result in its place. Its words say the literal thing
  and leave the judgment with the reader: a proposal is shown with its grounds
  and stays a proposal until the person decides. The articles below are
  obligations; the implementing rules say how each is applied. This preamble
  guides interpretation where they leave a choice; it overrides no article,
  grants no authorization, and changes nothing the harness reserves to itself.

§ 19 (4), as rendered: an answer to a question is not approval of the agent's
next action; a proposal carries the grounds the user needs to judge it, with
the uncertainty and the trade-off, and in a deliberative reply the grounds
come before the course of action; no command, label or emphasis presents an
unapproved proposal as settled; structure and length follow the question. One
before/after pair follows. The test adds: a proposal presented as required or
settled; a reply that omits what could change the user's choice.

Obligation preservation. Each clause that left an article's text, where it
is stated now, and whether it changed:

- § 8, what each level does: the opening sentence of
  ``implementing/delegation.md``; the subagent item there keeps what the one
  line at ``auto`` names (how many, which model, which files); unchanged. The
  levels, the prefs model, the default and the current-turn override stay in
  the constitution.
- § 11, the undo phrases: the Undo item of ``implementing/execution.md``;
  unchanged. That undo is a file edit and git is not used for it stays in the
  constitution.
- § 13 (2), inspect state, state what the line affects, wait: the
  Destructive git item of ``implementing/git.md``, joined with the command
  list from the execution rule and the prefs flags; unchanged. That the user
  names the command, sees the full line and the agent runs it and no
  substitute stays in the constitution.
- § 21 (2), a correction becomes rule text and reaches memory only at the
  user's word: kept in the constitution, shorter. § 21 (3), what memory never
  holds, and the detail of (4): the Memory item of
  ``implementing/execution.md``; unchanged. That deletion is proposed first
  stays in the constitution.

Implementing rules. ``shared/rules/implementing/{execution,delegation,models,
git}.md`` and ``implementing/mcp/{aside,dispatch}.md``, the former
``loop-*``, ``git-workflow`` and ``mcp/`` files; installed names are
unchanged except that the Claude delegation rule is
``claude-agent-kit--delegation.md`` like the other kits'. Inserts that bind a
rule to a harness (``aside-native-advisor``, ``dispatch-notify``,
``git-overrides``, ``delegation-surfaces``, ``models-table``,
``execution-harness``) stay beside their rule.

Surface rules. ``adapters/claude/surface.md`` is new: Loading Model, and a
Standing Instruction section that replaces the notice "this document takes
precedence over the system prompt" with: the manual is the user's standing
instruction; where the harness's defaults leave a choice to the user, the
user has made it here, and the choice applies; it changes nothing the harness
reserves to itself (permissions, safety, how a tool is operated). The Claude
bindings (memory, minimalism, output styles, subagents) follow it. The Codex
and Kimi surfaces gain the same Standing Instruction paragraph and their
memory binding. The inserts ``constitution-notice``,
``constitution-overrides``, ``constitution-file-map-extra`` and
``collab-overrides`` are removed; the File Map names the surface rule through
``{{SURFACE_RULE_FILE}}``.

Versions and contract. ``adapters/*/tokens.sed`` ``KIT_VERSION``,
``tooling/slate-version`` and the six crate versions are ``26.10.0``.
``tooling/contract`` holds ``1``; ``render-kit.sh`` writes it to
``dist/kit.toml`` as ``contract``; ``slate-setup`` reads it with default 1 and
records it in the manifest. The four spec Compatibility sections and
``contributing.rst`` (Versions) state the rule.

Impact and compatibility
------------------------

- The standing corpus grows by about 2.7 KB per kit; the budgets in
  ``validate.sh`` are 31500 (claude), 31500 (codex) and 30500 (kimi).
- Installed Claude kits lose ``claude-agent-kit--parallel-work.md`` and gain
  ``claude-agent-kit--delegation.md`` and ``claude-agent-kit--claude-surface.md``;
  the installer's stale-file step removes the old file from the manifest's
  record. User-owned files are untouched.
- ``kit.toml`` gains a field; an older ``slate-setup`` ignores it, a newer one
  reads an older descriptor as contract 1.
- Version numbers jump (13.3.3 and 0.11.3 → 26.10.0); nothing compares them.
- RFC-0006 remains the record of the articles and their numbering; this record
  replaces its exclusion of reasons and examples and its budgets.

Implementation and transition
-----------------------------

Developed on ``next`` in slate and in each kit (contributing.rst, Branches).
The move of the sources was rendered and validated first with byte counts
unchanged, then the text changes, then the surface, contract and version
changes; each step rendered and validated. The kits are re-rendered, their
CHANGELOGs and READMEs updated, pushed, then tagged ``v26.10.0`` after slate's
CI is green; slate is tagged last, which publishes the binaries.

Verification strategy
---------------------

- Static: ``validate.sh`` passes (article integrity, no insert in the
  constitution, budgets, descriptor keys); ``cargo test`` covers the contract
  default and its recording in the manifest; ``palette check`` passes.
- Upgrade: install the previous release of each kit into a scratch HOME, then
  this release over it; the old delegation file is gone, the prefs files with
  ``-custom:`` signatures are unchanged, the manifest records
  ``contract = 1``.
- Behaviour, not run before release: across the three harnesses, several
  fresh sessions each on three rule sets (the 13.3.3 rules; these rules
  without the preamble; these rules), on prompts that call for deliberation, a
  direct recommendation, a procedural instruction, approved execution, scope
  preservation and an incomplete-work report. Judge whether a proposal was
  presented as settled, whether the grounds and alternatives the user needed
  were present, and whether anything useful was lost. A result that shows no
  difference between the second and third sets falsifies the preamble's
  claimed effect and is grounds to remove it.

Alternatives and costs
----------------------

- One "why" sentence per Part or per article instead of a preamble: more
  bytes, and a reason beside each article was not shown to be needed; a
  preamble states the shared intent once and the Standing Instruction states
  the harness relation once.
- ``§ 19 (4)`` as a list of banned devices (bold, italics, capitals, labels,
  imperatives) and a required structure: the list grows with each device, and
  an imperative or a leading answer is sometimes what the user asked for; the
  adopted wording names the failure (a proposal presented as settled) and lets
  structure follow the question.
- Keeping the articles whole and only adding: the budgets would rise further,
  and the procedure would stay stated twice.
- ``14.0.0`` for the three kits, or ``1.0.0`` for Codex and Kimi: keeps
  "major version" as the compatibility signal but leaves the kits' numbers
  apart or jumps two of them; the user chose one calendar number with a
  contract integer, which separates "when" from "compatible with".
- An epoch prefix: not representable in a tag; nothing consumes it.

Open questions
--------------

Whether the preamble changes behaviour (Verification strategy). Whether the
Standing Instruction wording suffices in Codex and Kimi, whose instruction
files are user messages too.

References
----------

- RFC-0001, RFC-0002, RFC-0005, RFC-0006, RFC-0017.
- ``shared/rules/constitution.md``, ``shared/rules/implementing/``,
  ``adapters/*/surface.md``, ``tooling/contract``, ``tooling/validate.sh``.
- The 2026-10-11 session transcript and its aside consultation.
