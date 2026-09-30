Glossary — slate-agent-kit
==========================

:Status: Maintained
:Date: 2026-09-29

The terms the kit's rules, skills, tools and palette documents use. Each term
has one meaning. In a project's own documents, the project's glossary wins; a
term here never overrides a project's domain vocabulary.

Direction and authority
-----------------------

user
  The person who sets direction, boundaries and done criteria, and whose
  approval is the only thing that authorizes execution.

direction
  What to do and why: scope, priority, trade-offs and done criteria. The user
  sets it. An action that would change it (a different scope or design, work
  assigned to someone else than the documents name, spending beyond the
  configured level) goes to the user.

autonomy
  What the agent decides under the user's direction: method, order, tools, and
  whether to delegate or consult within the configured level.

prohibition
  An action that is never taken without an explicit user request, such as a
  destructive git operation. Prohibitions are a short fixed list.

article
  One numbered rule of the kit (``§ N``): a norm in numbered clauses and the
  test that shows it was broken. Defined once in the manual, cited by number
  everywhere else; numbers never move.

approval
  The user's explicit agreement that turns a proposal into authorized work.

level
  How freely the agent may use a surface that spends the user's resources
  (consultation, dispatch, subagents). ``on-request``: only when the user asks.
  ``suggest``: the agent proposes it in one line and waits. ``auto``: the agent
  judges whether it adds value now, uses it, and states it in one line.

Work structure
--------------

backlog item
  One unit of intent. It exists once, in the backlog, which records its status
  (``proposed``, ``approved``, ``in-phase-N``, ``done``, ``dropped``).

phase
  One increment the user approved: its goal, reason, assumptions and exit
  criteria. At most one phase is active.

deliverable
  One approved unit inside a phase, defined by its ``Done when`` contract: the
  outcomes a person can check when it is finished.

task
  One user request, handled by one pass of the execution loop (understand,
  plan, execute).

plan
  The ordered steps for carrying out a task or a deliverable. A plan is a
  session artifact, not a document.

step
  One entry of a plan. A step may be delegated.

run
  One step executed by an external backend through dispatch.

batch
  A group of delegates or runs approved and started together.

scope
  The files and work a single delegate owns. A file has one writer.

Actors
------

agent
  Any model-driven actor.

leader
  The agent in conversation with the user. It integrates and verifies what its
  delegates return.

delegate
  An agent the leader hands work to.

subagent
  A delegate created by the harness's own mechanism: the ``Agent`` tool in
  Claude Code, ``spawn_agent`` in Codex, the ``Agent`` tool in Kimi Code.

team
  A leader together with delegates that run at the same time and can message
  each other: agent teams in Claude Code, ``AgentSwarm`` in Kimi Code. Codex has
  no equivalent.

consultation
  A read-only request for an opinion (aside, or the harness's own advisor). A
  consultation is not delegation.

Documents
---------

layout
  The file that records where each document family lives in a project:
  ``internal`` (``_palette/``, personal and not committed) or a project path
  (committed and shared). Every family is used.

backlog
  The only list of work items and the only place their status is recorded; the
  index of phases and deliverables.

state
  The document holding decisions not yet written into an RFC or ADR, the
  questions blocking the active phase, and discrepancies. It records no
  progress and gives no instructions to a later session.

contributing
  The maintained document that tells a person or an agent how to contribute to
  the project: branches, commit format, pull request format, and the
  verification a change runs. The kit's git rule follows it where the prefs
  say ``repository``.

RFC
  A record of a consequential decision (module boundaries, public contracts,
  project-wide policy) together with the evidence it relies on.

ADR
  A record of a durable local choice within boundaries an RFC or the existing
  design already set, together with the evidence it relies on.

design
  The maintained description of how the system is built.

spec
  The maintained statement of contracts: interfaces, formats, obligations.

principles
  The project's lasting principles.

changeset
  The edits one RFC or ADR makes to maintained documents that the source does
  not yet implement, written against the documents' current text. A record has
  at most one changeset; an edit leaves it when its implementation lands.

staging
  A generated copy of a maintained design or spec document with every accepted
  changeset against it applied: the document as it will read once the accepted
  decisions are implemented. Never written by hand.

evidence
  What a decision relies on: the current code state when the code exists, the
  RFC dependency basis when it does not, and research findings. Evidence is
  written inside the RFC or ADR that relies on it.

Status vocabulary
-----------------

Decision
  Something the user accepted, with its source.

Proposal
  A suggestion by an agent or an earlier session, dated and not binding.

Verified fact
  A statement checked against a named source, with the kind and limit of the
  check.

Discrepancy
  Two sources that disagree, kept until the owning record resolves it.

Open question
  Something the user has not decided.

Rejected
  A proposal the user turned down, kept so it is not reintroduced.

Superseded
  A decision replaced by a later one, which it names.

accepted, implemented, verified
  Three separate facts. An accepted decision is not implemented until the code
  does it, and implemented code is not verified until a check shows it.

Session and lifecycle
---------------------

resume
  A new session picking up a project: it reads state within its budget,
  reports what it found, proposes, and waits for the user's direction.

graduate
  Move a decision out of state into its RFC or ADR, leaving a one-line pointer
  in state.

close (a phase)
  Move each outcome of a finished phase to where it is read next (backlog
  status and outcome pointers, proposed backlog items, proposed rule text), then
  delete the phase and deliverable files.

Retired terms
-------------

Do not use these; use the replacement.

- story: deliverable
- slice (as a planning unit): phase
- handoff: approval (the step that authorizes work) or resume (a new session
  picking up state)
- wave: batch
- lane: scope
- child, worker: delegate
- reviews (as a document): close
- records (as a document family): evidence inside the RFC or ADR
- dossier: no replacement; research that a decision relies on is evidence
