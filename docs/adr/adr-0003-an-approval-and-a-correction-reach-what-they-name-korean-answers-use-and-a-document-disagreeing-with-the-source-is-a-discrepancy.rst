ADR-0003: An approval and a correction reach what they name, Korean answers use 해요체, and a document disagreeing with the source is a discrepancy
================================================================================================================================================

:Status: Accepted
:Implementation: complete — the clauses in shared/rules/constitution.md,
  shared/workflows/palette/rules.md and
  shared/rules/implementing/mcp/consult.md, the register check in
  tooling/validate.sh, the four renders and CHANGELOGs
:Verification: static — 2026-10-17; sh tooling/validate.sh passes with the
  clauses in the four renders, each kit installs into a scratch HOME with the
  new § 18 and consult sentence, cargo fmt, clippy and the workspace tests pass
  on rustc 1.99.0, no runtime check of the rules
:Within: RFC-0018 (where a norm lives: an article of the constitution, cited by
  number)
:Authors: Claude Fable 5.1
:Reviewers: Hamin Sung
:Implementers: Claude Fable 5.1
:Accepted: Hamin Sung (2026-10-17T03:38Z)
:Date: 2026-10-17
:Revised: none
:Depends: RFC-0018 (the constitution's article structure and the
  standing-corpus ceiling as a release decision)
:Supersedes: none
:Related: none
:Changes: shared/rules/constitution.md (§ 4 and § 7 gain clause (4); § 18 names
  the polite informal register and its Test line the matching failure);
  shared/workflows/palette/rules.md (a hand edit is linted before it is
  reported; a contract disagreement between a document and the source is a
  discrepancy, a source-location citation is corrected in place);
  shared/rules/implementing/mcp/consult.md (the agent puts no decision to the
  user that a running consultation informs until its reply is read)
:Description: The constitution gains § 4 (4) and § 7 (4), which bound an
  approval and a correction to what the user named, § 18 moves the Korean
  register to 해요체, the palette rule lints a hand edit before it is reported and
  sends a contract disagreement between a document and the source to the state,
  and the consult rule holds a decision a running consultation informs until
  its reply is read.

Context
-------

A session in another palette project on 2026-10-17 showed an agent, after two
corrections pointing opposite ways ("why ask about document order" and then
"why decide the design alone"), reading the first correction as licence over
design decisions, reading the user's acceptance of one ADR as acceptance of two
sibling ADRs, rewriting two records so that they matched the code, running a
line-wrapping script over a record header and reporting without a lint, and
asking the user to accept an RFC while the consultation submitted to review it
was still running. The articles cover each failure in part: § 4 (3) requires
approval but does not say how far one reaches, § 7 (3) separates how from what
but does not say how far a correction reaches, the palette rule says every
document is written through the tools but asks nothing of a hand edit before it
is reported, the palette-state skill records a disagreement between a document
and the code as a discrepancy while the palette rule's "a maintained document
states only what the source implements" reads as code being the truth, and the
consult rule says a consultation is worth its cost when it can change an open
decision without saying that the decision waits for the reply.

The user also observed that the same model under the same corrections flails
less in the Claude.ai app, whose prompt asks for prose and the 해요 register. The
constitution's § 18 required 합니다; a register set in an output style that
differs from the article is one more point the agent is pulled between.

Evidence
--------

- The session transcript of 2026-10-17 in the other project, pasted by the
  user: the failures listed above, a revert made without reading the design
  document, a miscounted changeset total, and the agent's own account that it
  moved against the last correction rather than on facts.
- A reproduction on 2026-10-17: in a copy of this repository, the header field
  lines of ADR-0002 were joined into one paragraph and rewrapped at 80 columns,
  and palette check on the copy reported 16 P002 errors (eleven missing
  required fields and a Status value outside the allowed values); a copy whose
  continuation lines were joined onto the field lines reported nothing, which
  is correct since the values are intact. The reflow command leaves field lists
  byte for byte (shared/mcp-servers/palette/src/reflow.rs). The lint and the
  safe tool existed; the scripted edit went unlinted.
- palette_status reports Pending changeset edits, so the miscounted edit total
  in the transcript had a tool-reported figure available.
- The user's observation about the Claude.ai app is an observation, not a
  measurement; this record relies on it only for the register choice.
- A codex consultation on 2026-10-17 (dispatch d-2) read the first wording and
  found five readings wider or narrower than intended: an approval taken per
  file inside an approved plan, a correction's category inflated by the agent,
  the discrepancy sentence forbidding an authorised fix, "hand edit" excluding
  a script, and a consultation gate that expired when the run ended rather than
  when its reply was read. The Decision carries the reworded sentences.
- Standing-corpus sizes after the change, measured by tooling/validate.sh on
  2026-10-17: claude 32964, codex 33611, kimi 32549 and agy 33032 bytes against
  ceilings of 34000, 34000, 33000 and 34000.

Decision
--------

§ 4 gains clause (4): an approval reaches what the user saw and named, with the
work that scope needs (§ 3); an item the user did not see, a sibling record or
a later one of the same kind, needs its own. § 7 gains clause (4): a correction
reaches the decision it names and decisions of the same kind; it does not move
the how/what line in (3), and a correction about how is no licence over what. §
18 reads: Korean uses the polite informal register (해요, 예요); formal (합니다) or
casual (banmal) endings only when the user asks in the conversation; its Test
line names a 합니다 or banmal ending the user did not ask for.

The palette rule gains two sentences: an edit made outside the palette tools,
by hand or by script, is linted (palette_lint, or palette check) before it is
reported as done; a contract a document states and the source disagreeing is a
discrepancy for the state, resolved only on the user's decision, while a
drifted citation of a source location is a clarification, corrected in place.
The consult rule gains one: the agent puts no decision to the user that a
consultation was submitted to inform until its reply is read, or its failure or
cancellation reported.

tooling/validate.sh checks for the phrase "polite informal register" in the
constitution in place of "polite formal". The ceilings do not move.

Consequences
------------

The clauses are anchors for an agent asking what the rule is, not enforcement:
the same sessions showed that rule text yields to the pressure of a correction,
and validate.sh's own comment says per-rule compliance degrades as the corpus
grows. The kimi corpus has 451 bytes left under its ceiling, so the next clause
will likely require a ceiling decision.

The register change applies to every harness the kits cover, and a user whose
output style asks for 합니다 now has the article and the style disagreeing; the
article says the user's request in the conversation decides.

The discrepancy sentence narrows what an agent may edit on its own: a record
whose line-number citation drifted is corrected in place as a clarification,
and a record whose stated contract differs from the source becomes a state
entry that the user resolves. A project memory that tells the agent to align a
record with the implementation when the two differ now conflicts with the rule
and is the user's to revise.

Alternatives
------------

- A harness hook running palette check after every edit under a document path:
  declined by the user, who found hooks of little effect in practice; it would
  also be a slate-setup feature with no equivalent in every harness.
- A clause requiring a word diff of the fields a scripted edit touched: not
  added, since § 16 already asks for the output to be checked and the lint
  catches the case the clause would cover.
- Keeping 합니다 in § 18 and setting 해요 only in the Claude Code output style:
  rejected because the two would name different registers and the agent would
  be pulled between them.
- A single clause on the scope of the user's words covering both approval and
  correction: rejected in favour of one clause in each article, so that each
  reads on its own.

Verification
------------

sh tooling/validate.sh passes with the register check pointed at the new
wording; the four renders carry the three clauses and the two rule sentences;
an install into a scratch HOME of each kit writes the new § 18 and consult
sentence. The conformance test is the articles' own: a 합니다 ending the user did
not ask for, an acceptance of a sibling record on the approval of one, a design
decision taken on a correction about bookkeeping, a hand edit reported without
a lint, or a decision put to the user while its consultation runs, is a
failure.

References
----------

- RFC-0018 (the constitution and the implementing rules).
- ADR-0001 (the precedent for a clause added to an article in a release).
- shared/mcp-servers/palette/src/lint/structure.rs (P002 required header
  fields) and shared/mcp-servers/palette/src/reflow.rs (field lists kept byte
  for byte).
