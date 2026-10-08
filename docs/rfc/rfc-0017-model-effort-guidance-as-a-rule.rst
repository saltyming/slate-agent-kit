RFC-0017: Model and effort guidance ships as a rule; the bench is dropped
=========================================================================

:Status: Accepted
:Implementation: complete — the models rule file with its insert in each
  harness's adapter, the render and validate changes, the support matrix and
  the architecture document
:Verification: build — 2026-10-08; the three kits rendered, validate.sh with
  the raised budgets, palette check with 0 errors, and an install of each kit
  into a scratch HOME; the rule's effect on model choice in a live session is
  not observed yet
:Areas: rules; adapters; render and validate; kit releases
:Authors: Claude Opus 5.5, Claude Sonnet 5.5
:Reviewers: Hamin Sung
:Implementers: Claude Sonnet 5.5 subagents directed by Claude Opus 5.5 (2026-10-08)
:Accepted: Hamin Sung (2026-10-08T14:18Z)
:Date: 2026-10-08
:Revised: none
:Depends: RFC-0001 (direct use of the action levels and the delegate binding
  that the new rule defers to); RFC-0002 (direct use of the kit payload and
  the render, which gain one rule file); RFC-0006 (direct use of the articles
  the rule cites and of the standing-corpus byte budgets)
:Supersedes: RFC-0011 (the bench server and the benchmark suite, wholly);
  RFC-0015 (committing the benchmark suite, wholly); RFC-0016 (running bench
  cells in the user's harness home, wholly)
:Related: none
:Changes: spec/support-matrix.rst (Contract); design/architecture.rst
  (Sources); spec/agent-exec.rst (Guard)
:Description: Each harness kit ships a standing rule that says which model and
  effort a subagent, a dispatch step or a consultation runs on, with a table of
  that harness vendor's own models, and the bench that was to measure this is
  dropped.

Summary
-------

The bench server, its benchmark suite and the runs in the user's harness home
(RFC-0011, RFC-0015, RFC-0016) are dropped; none of them was implemented in
this repository. Their purpose, deciding which model and effort fit which work
(backlog item B-8, task-class routing), is served by a standing rule instead.
Each kit renders a ``models`` rule file from one shared source plus a
per-harness table of that harness vendor's own models: the Claude kit lists
Anthropic models only, the Codex kit OpenAI models only, the Kimi kit Moonshot
models only. The rule states how to choose by the shape of the work and gives
each model a tier, what it is for and a starting effort. It applies where the
user, the prefs or the call has not fixed the choice, and it ships in a patch
release of each kit.

Problem and context
-------------------

RFC-0011 planned to answer B-8 by measurement: a kit-registered bench server
running a model by effort grid, a committed benchmark suite on saltyos
excerpts (RFC-0015) and runs in the user's own harness home (RFC-0016). All
three are Accepted with Implementation not-started. Public benchmarks do not
replace that measurement for the kit: vendors report different benchmarks on
different harnesses and effort settings, so their figures do not compare
across vendors, and they age with each model release.

The user abandoned the bench after running a prototype of it (Evidence, The
user's measurement). The agent still has to choose, and it has no standing
knowledge of the tiers a harness offers. A delegate that names no model runs
on the default subagent model the prefs set, else on the session's model, so
it inherits the top model and bills its reading at the top rate unless the
leader names another. When a lower tier is used, the vendor statements below
show where it goes wrong at low effort: it skips a search, stops early or
skips a check. A lower tier needs the same scope and verification as a higher
one, at an effort the vendor names as its start.

Evidence
--------

The user's measurement
~~~~~~~~~~~~~~~~~~~~~~

Run by the user from 2026-09-30 to 2026-10-01 through the abandoned bench
prototype, in Claude Code session 7aa66484. No result file survives; the
session transcript is the only copy. Models: ``claude-opus-5-5`` at low to
max and ``gpt-6.1-sol`` at low to xhigh.

- Small scoped tasks (a review against seeded bugs, an implied-change task),
  2 repeats per cell: effort did not change the result. ``claude-opus-5-5``
  found 13 to 14 of 16 seeded defects at every level, ``gpt-6.1-sol`` 16 of
  16. Time and cost grew 3 to 5 times from the lowest to the highest effort.
- One kernel implementation task (boot and reach init on x86_64 and aarch64;
  every run reached init), scored by judged design fidelity from 0 to 1,
  higher better. Fidelity rose from medium to high; xhigh was not
  distinguishable from high.
- Cost: ``gpt-6.1-sol`` cost 10 to 30 times less per task than
  ``claude-opus-5-5``. About 30 % of the Opus cost on long tasks came from
  subagents and the advisor that the kit spawned. The total recorded cost was
  about 330 USD at list prices.
- Limits: one kernel task, 1 to 2 runs per cell, crossed judges, and two
  rubric items found wrong afterwards. The cross-model comparisons were
  retracted in that session; the within-model effort trends were kept, and the
  columns below are not comparable across models.

.. list-table:: Judged design fidelity, kernel task, one value per repeat
   :header-rows: 1
   :widths: 2 4 4

   * - Effort
     - claude-opus-5-5
     - gpt-6.1-sol
   * - low
     - 0.45
     - 0.65
   * - medium
     - 0.53, 0.47
     - 0.63, 0.48
   * - high
     - 0.67
     - 0.77
   * - xhigh
     - 0.70
     - 0.75
   * - max
     - 0.80
     - not run

Vendor statements
~~~~~~~~~~~~~~~~~

Read 2026-10-08.

- Anthropic effort guidance
  (https://platform.claude.com/docs/en/build-with-claude/effort): for Haiku
  5.5, "Start with ``medium`` for most work, including agentic coding"; at
  ``low`` in long agent prompts it is more likely to skip a search, stop early
  or skip a check. Sonnet 5.5: ``medium`` for well-specified agentic coding,
  ``high`` for harder or longer work.
- Haiku 5.5 (https://www.anthropic.com/claude-haiku-5-5): "It pairs well with
  Opus 5.5 and Sonnet 5.5 as a subagent on coding work"; Terminal-Bench 4.0:
  Haiku 39.2 %, Sonnet 70.6 %.
- Opus 5.5 (https://www.anthropic.com/claude-opus-5-5): Terminal-Bench 4.0:
  Opus 66.4 % at xhigh, Fable 5.1 55.8 %.
- Claude Code model configuration
  (https://code.claude.com/docs/en/model-config): the default model is Opus
  5.5, and Opus, Sonnet and Haiku 5.5 default to ``medium`` effort.
- Codex: the catalog printed by ``codex debug models`` (codex-cli 0.161.0,
  2026-10-08) and https://learn.chatgpt.com/docs/models.md: "Use GPT-6.1 Sol
  when available ... and Luna for clear, repeatable tasks"; "Start with High
  for Luna or Light for Astra"; "Most tasks do not need Max or Ultra".
  https://learn.chatgpt.com/docs/agent-configuration/subagents.md: Luna for
  "clear, repeatable, or high-volume work", 6.1 Sol for work that "needs
  planning, tool use, validation, and follow-through". OpenAI's own benchmark
  pages returned HTTP 403 on 2026-10-08 and the third-party relays found could
  not be traced to them, so the Codex table carries no benchmark figure.
- Kimi: https://www.kimi.com/code/docs/en/kimi-code/models and
  https://www.kimi.com/code/docs/en/kimi-code-cli/configuration/config-files.html
  (the ``[secondary_model]`` pool, whose aliases such as ``kimi-code/k3`` are
  what a subagent can name, and its hints; the highspeed model is 6 times
  faster at 3 times the quota), and
  https://www.kimi.com/code/docs/en/kimi-code/whats-new.html (with thinking
  off, K3 and K2.8 Preview requests are served by K2.8 Preview without
  thinking; switching models invalidates the context cache). The K3 and K2.7
  Code benchmark figures sit on separate model cards and K2.8 Preview has
  none, so the Kimi table carries no benchmark figure.

Delegation economics
~~~~~~~~~~~~~~~~~~~~

- Anthropic, "Optimizing for cost and intelligence"
  (https://platform.claude.com/docs/en/about-claude/models/optimizing-for-cost-and-intelligence):
  a Fable 5.1 lead with 25 Sonnet 5 workers took about 2.3 hours against 15 to
  20, cost 47 to 55 % less and scored 10 to 12 points lower in accuracy;
  delegation saved money only on routine pieces or on work larger than one
  context; "When the work is one dependent chain, or fits in a single context
  ... the coordinator's model alone at lower effort came out ahead".
- Systima, "The subagent tax" (https://systima.ai/blog/subagent-tax), a
  third-party report of single runs: Opus sequential 4 min 15 s and 295k input
  tokens, against 8 min and 762k with two subagents on the inherited model;
  pinning those subagents to Haiku cut the tokens by 37 % and the time to 3
  min 45 s.

Goals and non-goals
-------------------

Goals:

- The agent can choose a model and an effort for a delegate, a dispatch step
  or a consultation from the kit's own text, in every session, without loading
  anything.
- Scoped work goes to the cheapest suitable tier; design-bound work keeps its
  effort; a lower tier is never given less than the scope and verification a
  higher one would get.

Non-goals:

- Measuring the models, in the kit or alongside it; the bench server, its
  suite, its prefs file and its release artifact are not built.
- Ranking vendors against each other, or listing a harness's competitors.
- A new article, a new prefs file or a new MCP tool.

Requirements and invariants
---------------------------

- The rule applies only where the user's current-turn instruction, the prefs
  (§ 8) or the call has not fixed the model and effort; each of those
  outranks the table.
- A kit's table names only that harness vendor's models. A figure in a table
  compares tiers on one vendor's page and ranks no vendor against another.
- Each table carries its as-of date and says where a quoted figure comes from;
  a figure that cannot be traced to the vendor is left out.
- A dispatch step or a consultation on another vendor's backend is outside the
  table: it runs on the model its prefs name, else on that backend's default.
- The text cites articles by number (§ 8, § 15) and defines none.
- A lower tier gets the same scope, the same self-contained prompt and the
  same verification as the top model would, and never runs below its listed
  starting effort; the leader confirms that a check the delegate reports was
  run.

Design
------

One shared source, ``shared/rules/core/loop-models.md``, holds the choice
rules; a marker ``{{@INSERT models-table}}`` pulls in the per-harness table
from ``adapters/<harness>/inserts/models-table.md``. The render writes the
result as the kit's ``models`` rule file, loaded after the delegation rule.
The kernel's file map names the file, and the delegation rule points to it for
the model and effort of a delegate.

The choice rules:

- Scoped work, whose shape is given (a named fix, a search, a mechanical edit
  across any number of files, a review against a checklist), runs on the
  cheapest tier the table lists for that kind of work at the tier's starting
  effort. Work that needs design reasoning (implementing an RFC, a change
  whose shape the delegate must work out) runs at high; xhigh and max only
  where a check shows a gain.
- A lower tier is neither a lower effort nor a smaller task.
- A delegate on scoped work names its model through the harness's per-call or
  default setting, where the harness has one, instead of inheriting the
  session's.
- Parallel workers on a lower tier cut wall time when the work splits into
  many independent pieces; they cut cost only on routine pieces or on work too
  large for one context. One dependent chain stays with one model.

Each table gives model, tier, use and starting effort:

.. list-table:: Models by harness
   :header-rows: 1
   :widths: 2 8

   * - Harness
     - Models in the table
   * - Claude
     - Fable 5.1 (frontier), Opus 5.5 (default), Sonnet 5.5 (workhorse), Haiku
       5.5 (fast)
   * - Codex
     - ``gpt-6-astra`` (frontier), ``gpt-6.1-sol`` (default), ``gpt-6-luna``
       (fast)
   * - Kimi
     - ``k3`` and ``k3-256k`` (frontier), ``kimi-for-coding`` (workhorse),
       ``kimi-for-coding-highspeed`` (fast)

Each table also states the facts from Evidence that bear on its models: the
Haiku behavior at low effort, the Terminal-Bench figures and the vendor's
placement of Haiku as a coding subagent on the Claude side;
previous-generation models, the ``ultra`` effort that runs subagents of its
own, and the vendor's placement of Luna and 6.1 Sol on the Codex side; the
quota cost of highspeed (the workhorse takes routine work, highspeed only
where latency matters), the pool aliases a subagent names, and the cost of
switching model or effort mid-session on the Kimi side.

The starting efforts follow the vendor's own guidance where it names one
(Anthropic: ``medium`` for agentic coding on Haiku 5.5 and Sonnet 5.5; OpenAI:
High for Luna, Light for Astra; the Kimi rows follow the model hints on the
Kimi pages) and the user's measurement for the split between scoped and
design-bound work: on scoped work effort bought time and cost, not results; on
design-bound work fidelity rose from medium to high and not beyond.

Impact and compatibility
------------------------

- The standing corpus of every kit grows by one rule file; the byte budgets in
  ``validate.sh`` rise by its size. The rule loads in every session, so it
  costs input tokens in each.
- The table goes out of date as vendors release and retire models. Revising a
  table is a source edit, the support matrix changes in the same change, and
  it ships in a kit patch release; the as-of date shows how old a table is.
- The level of each surface (§ 8) still decides whether the surface is used at
  all; the rule decides only which model and effort it runs on.
- No MCP server, prefs file, installer behavior or release artifact changes.
  The changesets of the bench leave the repository; the three records stay as
  Superseded.

Implementation and transition
-----------------------------

The rule file, the three inserts, the render change and the raised byte
budgets land in one change, so that ``validate.sh`` never runs a larger corpus
against the old budgets. The changesets of RFC-0011, RFC-0015 and RFC-0016 are
deleted in the same change as this record, since their edits will not land;
the generated indexes and staging documents are regenerated. The change ships
in claude-agent-kit 13.3.3, codex-agent-kit 0.11.3, kimi-agent-kit 0.11.3 and
slate v0.10.3 through the release train in ``CLAUDE.md``.

Verification strategy
---------------------

- The rendered Claude, Codex and Kimi kits each contain the rule once, after
  the delegation rule, and its table names only models of that harness's
  vendor.
- ``sh tooling/validate.sh`` prints ``validate: OK`` with the raised budgets.
- ``palette check .`` reports 0 errors on this repository; RFC-0011,
  RFC-0015 and RFC-0016 read ``Superseded`` and ``abandoned``, and no staging
  document remains for the bench.
- A session in each harness after ``make install`` shows the rule among the
  loaded files; whether the agent then picks the listed tier for scoped work
  is observed in use, not established here.

Alternatives and costs
----------------------

- A user-owned prefs file: the user would write the tiers themselves. Other
  users get no default, the knowledge of which models a harness offers is
  per-harness, and the kit is where that knowledge reaches every session. The
  prefs still outrank the rule.
- An on-demand skill: the agent has to decide to load it, which is the gap the
  rule closes; a delegate chosen without the table is the case at issue.
- One table across vendors: the public figures do not compare across vendors,
  and each kit runs inside one vendor's harness. aside and dispatch reach
  other vendors' models through their own parameters and prefs.
- Keeping the bench: abandoned by the user. Its prototype cost about 330 USD,
  and its results (effort mattered on design-bound work between medium and
  high, and not on scoped work) are the basis of the starting efforts here.

Open questions
--------------

None.

References
----------

- `RFC-0001 <rfc-0001-direction-autonomy-levels.rst>`_
- `RFC-0002 <rfc-0002-installer-and-kit-layout.rst>`_
- `RFC-0006 <rfc-0006-rules-in-articles.rst>`_
- `RFC-0011 <rfc-0011-measurement.rst>`_
- `RFC-0015 <rfc-0015-benchmark-suite-committed.rst>`_
- `RFC-0016 <rfc-0016-bench-runs-in-user-home.rst>`_
- `spec/support-matrix.rst <../spec/support-matrix.rst>`_
- `design/architecture.rst <../design/architecture.rst>`_
- The URLs in Evidence, read 2026-10-08
