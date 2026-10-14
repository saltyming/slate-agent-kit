RFC-0024: Release hardening: layout moves, refresh verification, 79-column prose, mcp retirement, wait liveness, agy approvals
==============================================================================================================================

:Status: Accepted
:Implementation: complete — shared/mcp-servers/palette,
  shared/mcp-servers/dispatch, shared/setup, docs
:Verification: build — 2026-10-14; workspace tests (938 passed, 0 failed),
  clippy -D warnings and fmt on rustc 1.99.0 on macOS, validate OK, palette
  check clean after reflowing 16 records (whitespace only), the Linux and
  Windows start-time readers are type-checked, not run, until the three-OS CI,
  no live install
:Areas: palette; dispatch; installer; specs
:Authors: Hamin Sung; Claude (Fable 5.1)
:Reviewers: codex gpt-6-astra (aside, 2026-10-14)
:Implementers: Claude (Opus 5.5) with two Sonnet 5.5 delegates
:Accepted: Hamin Sung (2026-10-14T06:23Z)
:Date: 2026-10-14
:Revised: 2026-10-14 — Changes names Legacy cleanup in spec/installer.rst, whose
  Retired servers entry states which commands retire a server (Design, mcp
  retirement); 2026-10-14 — Changes names Uninstall in spec/installer.rst,
  which counts the agy approval strings that now come from the probe (Design,
  agy approvals)
:Depends: RFC-0023 (direct use of the agy installer module whose approvals
  change and of the per-harness layout); RFC-0022 (direct use of the wait
  subcommand and the reconciliation whose liveness check changes); RFC-0021
  (direct use of palette_record_refresh, which gains the verification
  arguments); RFC-0019 (direct use of the document root and the non-family
  changeset targets the layout move must keep inside it)
:Supersedes: none
:Related: none
:Changes: spec/palette-server.rst (Write tools; Command line; Ownership and
  ordering; Conformance); spec/dispatch-server.rst (The wait subcommand; Errors
  and edge cases; Conformance); spec/installer.rst (Commands; Server
  registration; Uninstall; Legacy cleanup; Conformance)
:Description: Seven recorded defects and gaps are closed before 26.10.1: palette
  refuses a layout move that strands a changeset target, records Verification on
  refresh and wraps the prose it writes; dispatch tells a reused pid from a live
  process and names the prefs as they are; the installer retires servers in its
  mcp command and reads the palette read tools for agy.

Summary
-------

The backlog items recorded at the three phase closes, and the user's request
for wrapped prose, are implemented together before the release. palette:
``palette_layout_set`` refuses a staging move whose new document root would
exclude a target one changeset edits (B-8); ``palette_record_refresh`` takes
``verification`` and ``verification_note`` and writes the Verification field
(B-9); the write tools wrap the paragraph and list text they write at 79
columns and a ``palette reflow`` command rewraps existing documents once
(B-13). dispatch: the ``dispatch_submit`` description names the dispatch prefs
as they are (B-11); ``dispatch wait`` and the server's reconciliation treat a
pid whose process started after the row was written as dead (B-12). Installer:
``slate-setup mcp`` retires a server its manifest recorded and the kit no
longer lists (B-10); the agy module reads the palette read-only tools from
``palette --read-only-tools`` like the claude and codex modules (B-14).

Problem and context
-------------------

B-8 (Phase 1): moving the staging placement changes the document root, and a
changeset whose non-family target then falls outside it is not refused; the
next lint shows only P007 and generation deletes its mirror. B-9 (Phase 1):
only promotion writes Verification, so ADR-0001, implemented without a
changeset, stays at ``none`` after ``palette_record_refresh``. B-13 (user,
2026-10-14): the write tools store a section's text as the caller supplied it,
so a paragraph given on one line stays one line, as RFC-0022 and RFC-0023 show,
while hand-written documents and the backlog bodies are wrapped at 79 columns.
B-10 (Phase 2): the retirement step runs only in the kit install path, so
``slate-setup mcp`` (``tooling/install-mcp.sh``) leaves an ``aside``
registration and binary in place. B-11 (Phase 2): the ``dispatch_submit``
description still tells the agent to follow the ``conservative`` /
``preference-only`` / ``proactive`` policies, which the dispatch prefs replaced
with levels in RFC-0001. B-12 (Phase 2): liveness is ``kill(pid, 0)`` or
``GetExitCodeProcess``, so a pid the operating system reused keeps a dead run
``running`` and a wait on it never exits. B-14 (Phase 3): the agy module writes
nine approval strings from a constant while the claude and codex modules read
the palette read-only tools from the binary, so a new palette read tool would
miss agy.

Evidence
--------

- ``shared/mcp-servers/palette/src/ops/init.rs`` ``layout_set``
  (``move_family``) builds the new layout and location before writing and
  ``layout.rs`` ``document_root()`` derives the root from the placement, so the
  root a move implies is known before any file moves; ``changeset.rs``
  ``Changeset::parse`` resolves targets through the snapshot's layout
  (``Snapshot::target``), and the collection keeps one parsed changeset per
  record and drops an unreadable one; target validity in ``layout.rs`` covers
  containment in the root, not lying inside the staging folder, and a mirror
  that does not collide with the reserved ``design`` and ``spec`` mirrors or a
  single-file mirror.
- ``ops/records.rs`` ``record_refresh`` and ``params.rs``
  ``RecordRefreshParams`` (``record``, ``implementation``,
  ``implementation_scope``, ``implementers``) write Implementation and
  Implementers only; promotion writes Verification as
  ``<kind> — <date>; <note>``, requires a non-empty note and replaces
  semicolons in it.
- ``util.rs`` ``wrap_words(text, first_prefix_len, indent, width)`` wraps with
  backtick-delimited spans kept whole and an over-long token on its own line;
  ``edit.rs`` (``WRAP_COLUMN = 80``) collapses and wraps the paragraph and
  bullet text the work-document writers produce; record sections and changeset
  edit bodies are written as given. ``changeset.rs`` ``converted_body`` copies
  an edit's lines into the target changing heading levels only, and
  ``generated.rs`` compares generated text after line-ending normalisation; the
  write transaction (``ops/mod.rs`` ``regenerate`` then lint) regenerates
  staging from the stored text. The RST scanner (``rst.rs``) separates
  definition terms from their bodies, treats ``..`` comments with indented
  bodies as one block, reclassifies list-table cells as prose, joins a link
  target split over lines by removing the newline and indentation, and finds a
  literal block by the ``::`` introducer and indentation; the glossary template
  is a definition list. The lint's P011 counts physical lines (state entries,
  backlog bodies, phase and deliverable files) and P009 and P010 inspect single
  lines.
- ``shared/mcp-servers/dispatch/src/main.rs`` line 108: the ``dispatch_submit``
  description's POLICY sentence names ``conservative`` / ``preference-only`` /
  ``proactive`` and its APPROVAL sentence an approval mode (``ask`` / ``auto``)
  and a per-step or batch granularity, none of which the dispatch prefs have
  had since RFC-0001; ``get_info`` points at the rules and prefs without naming
  them. ``liveness.rs``: ``process_alive(pid)`` is ``kill(pid, 0)`` on Unix
  (``EPERM`` counts as alive) and ``OpenProcess`` + ``GetExitCodeProcess`` on
  Windows; ``wait.rs``, the reconciliation in ``main.rs``
  (``store::active_owners`` returns ``(id, owner_pid)`` only) and
  ``augment_observability`` (``child_process_alive``) call it; the executor's
  watchdog checks the attempt-local pid from ``RunEvent::Started`` and returns
  ``Verdict::Proceed`` on a dead process, leaving completion to the runner, and
  writes no row state. ``created_at`` is SQLite ``datetime('now')`` at
  insertion and ``started_at`` the same at the Started event: UTC wall time to
  whole seconds. ``libc`` and ``windows-sys`` are dependencies.
- ``shared/setup/src/mcp.rs`` ``run`` registers the servers and records the
  registrations in the mcp manifest but not the installed binaries;
  ``retire.rs`` ``plan``/``apply`` take a ``Kit`` (payload name, servers,
  layout) and exclude the kit's own manifest when asking whether another
  installation keeps a binary; Kimi's recorded servers come from the plugin
  manifest. ``serve.rs``: ``register`` runs before the palette probe and
  ``apply_config`` later feeds ``agy::settings_edits`` /
  ``permission_entries(dispatch, palette)``, which emit the fixed
  ``PALETTE_PREAPPROVED_TOOLS``; ``binaries.rs``
  ``read_only_tools(env, palette)`` runs ``palette --read-only-tools``.
- Process start times: Linux ``/proc/<pid>/stat`` field 22 (``starttime`` in
  clock ticks since boot, with ``btime`` from ``/proc/stat``); macOS
  ``proc_pidinfo(pid, PROC_PIDTBSDINFO, ...)`` returning
  ``proc_bsdinfo.pbi_start_tvsec`` (``libc`` exposes both on Apple targets);
  Windows ``GetProcessTimes`` creation time (``windows-sys``). Each is readable
  for the caller's own processes; for another user's process macOS and Linux
  may refuse.
- The house style (``shared/workflows/palette/templates/house-style.rst``)
  wraps prose at 79 columns; the lint has no rule on line length.

Goals and non-goals
-------------------

Goals: the seven items closed as the Design states; every change stated in the
contracts (palette-server, dispatch-server, installer specs); tests for each;
the two long records reflowed once.

Non-goals: no lint rule on line length (hand-written documents stay the
author's business); no reflow of literal blocks, tables, field lists or the
templates; no change to how a reused pid is detected on a platform where a
start time is unreadable beyond the pid check; no new tool for retirement
outside the mcp command; no change to the dispatch rule text (the prefs levels
are already there).

Requirements and invariants
---------------------------

- ``palette_layout_set`` refuses, with ``invariant_violation`` naming each
  changeset and target, a staging move after which a changeset's non-family
  target would be invalid under the new placement (outside the document root,
  inside the staging folder, or with a mirror that collides with a reserved
  one), and a move while a changeset cannot be parsed; a move that strands
  nothing proceeds as today. The check runs before any file moves and leaves
  layout, sources and mirrors untouched on refusal.
- ``palette_record_refresh`` accepts ``verification`` (``none``,
  ``documentation``, ``static``, ``build``, ``runtime``) with a non-empty
  ``verification_note`` (semicolons replaced as promotion does), writes
  Verification in the template format promotion uses, keeps the field when both
  are absent, and refuses one without the other (``invalid_params``).
- Wrapping changes line breaks inside paragraphs, list items and definition
  bodies only, at 79 columns: no character other than whitespace between words
  changes; a title and its underline, a field list, a definition term, a
  literal block and its introducer's attachment, a directive with its whole
  extent, a table, a comment with its body and a blank line are unchanged; a
  backtick-delimited span is one token, a link target already split over lines
  is rejoined without a space, an escape is kept, and a token longer than the
  width stands on its own line. The scanned structure of a document (block
  kinds, titles, fields, links, directives) is identical before and after;
  findings of the lint rules that count physical lines (P011) or read one line
  (P009, P010) may differ and are reported as usual. The pass is idempotent.
- The write tools wrap the fragments they receive before parsing or splicing
  them, never the whole file; ``palette reflow`` is the only operation that
  rewraps an existing document, under the lock and the write transaction,
  regenerating staging from the reflowed text and linting the result;
  afterwards ``palette generate`` has nothing to generate.
- ``slate-setup mcp`` retires exactly what ``install`` and ``configure`` retire
  for the same recorded manifest, with the same order, summary and reporting,
  and from now on records the binaries it installs in its manifest.
- No tool description or ``get_info`` text of dispatch names ``conservative``,
  ``preference-only``, ``proactive``, an approval mode or an approval
  granularity; ``validate.sh`` greps the dispatch sources for those terms.
- For a row, a pid is alive only when the process runs and, when its start time
  is readable, started no later than the row's recorded time plus one second
  (owner: ``created_at``; child: ``started_at``); a missing or malformed
  timestamp, or an unreadable start time, leaves the pid check alone to decide.
  The check is a heuristic: a replacement process that starts within the
  allowance, or a clock corrected between the write and the check, is not told
  apart, and the spec says so.
- The agy module's palette approvals are the names
  ``palette --read-only-tools`` prints, obtained as the claude and codex
  modules obtain them; the dispatch approvals stay the fixed five.

Design
------

palette, layout move (B-8). ``layout_set`` builds the prospective layout before
moving anything, parses every changeset against the current snapshot (so each
target's identity is the one the parser uses today), and validates each
identity against the prospective placement with the same target rules the
parser applies (contained in the document root, not inside the staging folder,
mirror not colliding with the ``design`` and ``spec`` mirror folders or a
single-file mirror). A target that would be invalid, a changeset that does not
parse, and a Draft or Proposed record's changeset alike are refused with
``invariant_violation`` listing ``<record>: <target>`` (or
``<record>: unreadable``); nothing moves. The spec's Write tools entry for
``palette_layout_set`` states the check.

palette, refresh verification (B-9). ``RecordRefreshParams`` gains
``verification: Option<String>`` and ``verification_note: Option<String>``;
``record_refresh`` validates the kind against the template's values, requires a
non-empty note with it (semicolons replaced as promotion does), refuses one
without the other, writes ``:Verification: <kind> — <YYYY-MM-DD>; <note>``
through the writer promotion uses, and leaves the field untouched when both are
absent; Accepted, Revised and the body do not change. The spec's entry for
``palette_record_refresh`` and the Ownership section say that refresh writes
Verification too. ADR-0001 is refreshed with ``static`` and a note naming the
validate.sh check.

palette, wrapping (B-13). A ``reflow`` module wraps RST text at 79 columns by
block, using the scanner's own classification (``rst.rs``) so that it skips
what the scanner recognises: a title and its underline; a field list line and
its continuation; a definition term (the body is wrapped at its indent); a
literal block with its ``::`` introducer kept attached to the paragraph that
ends in it; a directive from its marker to the end of its indented extent,
list-table cells included; a grid or simple table; a comment with its indented
body; a blank line. Every other run of lines is refilled as a paragraph or a
list item (``- ``, ``* ``, ``#. ``, ``1. `` markers; a continuation indented to
the item's text; a nested item its own item; a paragraph after a blank line at
the item's indent part of the item), through ``util::wrap_words``, which keeps
a backtick-delimited span as one token and puts an over-long token on its own
line; a link target split over lines is rejoined without a space before
wrapping, escapes are preserved, and other inline markup may span lines as RST
allows. The write tools wrap the free text fragments they receive (record
sections, changeset edit bodies, deliverable and phase text, state entries,
backlog bodies) before parsing and splicing them, so stored text is wrapped and
cached line ranges stay valid; ``edit.rs`` adopts the same width (79) so no
later helper undoes it; the templates' constructs are never touched.
``palette reflow <project> [paths]`` runs the pass over every document the
layout places and the server does not generate, under the lock and the write
transaction (regenerate, lint, write only changed files), prints the files it
wrote, and regenerates staging from the reflowed text. The two long records of
this repository are reflowed once by that command, without a Revised entry
(D-2).

dispatch, description (B-11). The ``dispatch_submit`` description's POLICY and
APPROVAL sentences become one: whether and when to dispatch is decided by the
dispatch rule and the dispatch preferences the harness installed (level
``on-request`` | ``suggest`` | ``auto``, backend, model, effort, fallback);
this tool runs a step once that decision is made. ``validate.sh`` adds
``conservative``, ``preference-only``, ``proactive``, ``approval mode`` and
``approval granularity`` to the retired terms it greps in
``shared/mcp-servers/dispatch/src``.

dispatch, liveness (B-12). ``liveness`` gains
``process_start_time(pid) -> Option<SystemTime>`` (Linux ``/proc/<pid>/stat``
with ``btime``; macOS ``proc_pidinfo`` ``PROC_PIDTBSDINFO``; Windows
``GetProcessTimes``; ``None`` elsewhere or when unreadable) and
``alive_since(pid, recorded_at: Option<SystemTime>) -> bool``: the process runs
and, when both the start time and ``recorded_at`` are known, the start is no
later than ``recorded_at`` plus one second (SQLite writes whole seconds).
``store::active_owners`` returns ``created_at`` beside the pid; the boot and
read-time reconciliation and ``dispatch wait`` call
``alive_since(owner_pid, created_at)``; ``child_process_alive`` in
``dispatch_status`` calls ``alive_since(child_pid, started_at)``. The
executor's watchdog is unchanged: it watches an attempt-local pid it received
at the spawn and only decides whether to keep watching. A row whose owner pid
was reused is ``interrupted`` as a dead owner's row is today. The spec's wait
section replaces "liveness is by pid; a reused pid is a known limit" with the
check, its allowance and its limits.

installer, mcp retirement (B-10). ``retire::plan`` and ``apply`` take a
retirement context (the prior manifest, the servers kept, the harness and home,
the manifest folder and the identity of the caller's own manifest) instead of a
``Kit``; the kit path builds it from the kit, and ``mcp::run`` builds it from
the mcp manifest for each harness it registers, with the servers it registers
as the kept set. ``mcp::run`` prints the plan in its summary, applies it before
registering, records the result, and from now on records the installed binaries
in the mcp manifest, so an mcp-installed binary has an owner; a manifest that
recorded only a registration decides binary removal by the other manifests'
records, as the kit path does, and Kimi's recorded servers come from the plugin
manifest as today. ``--uninstall`` is unchanged; ``tooling/install-mcp.sh``
needs no change.

installer, agy approvals (B-14). ``serve.rs`` probes
``palette --read-only-tools`` for every harness and passes the names into
``apply_config``, where ``agy::settings_edits`` / ``permission_entries`` build
the ``mcp(palette/<tool>)`` strings from them and the ``mcp(dispatch/<tool>)``
strings from the fixed five; a failed probe writes no palette approvals and
notes it, as for claude and codex; a reinstall over existing approvals adds no
duplicate; ``PALETTE_PREAPPROVED_TOOLS`` is removed. The installer spec's agy
registration entry says the names come from the binary.

Impact and compatibility
------------------------

Every change is inside one contract number. Documents written by earlier
palette releases stay valid; ``palette reflow`` is the only way an existing
document's line breaks change, and it is run on this repository's two long
records once. A row written by an earlier dispatch release carries
``created_at`` and ``started_at`` already, so the liveness check applies to it;
a platform without a readable start time keeps today's behaviour. A home whose
mcp manifest records ``aside`` is cleaned by the next ``slate-setup mcp`` run
as it is by install. An agy home installed by the previous build carries the
same nine approval strings the probe yields today, so a reinstall changes
nothing there.

Implementation and transition
-----------------------------

Two deliverables run in parallel on Sonnet delegates, one writer per crate:
palette (B-8, B-9, B-13 with the reflow command) and servers-and-installer
(B-10, B-11, B-12, B-14), each with the tests the Verification strategy lists;
the leader writes the spec changes through this record's changeset, reflows the
two records with the new command, refreshes ADR-0001, updates the kit
CHANGELOGs and promotes. The release train follows.

Verification strategy
---------------------

- palette, layout: a move that strands one and several targets of one or
  several changesets is refused naming each, with Draft, Proposed and Accepted
  changesets, an internal staging, a target that would land inside the new
  staging folder, a target whose mirror would collide with a reserved one, and
  an unreadable changeset; the refusal leaves layout, sources and mirrors
  byte-identical; a move that strands nothing proceeds.
- palette, refresh: with both arguments; with neither (field kept); kind
  without note and note without kind (``invalid_params``); an empty note; an
  invalid kind; semicolons in the note; Accepted, Revised and the body
  unchanged.
- palette, reflow: on the house-style constructs (title, field list, definition
  list, literal block with its introducer inside and outside a list,
  ``code-block`` and ``list-table`` with nested cell constructs, grid table,
  comment with a body and a bare ``..``, nested lists with a parent
  continuation after nested content, a link that would split and one already
  split, an inline literal with spaces, escaped markup, a token longer than 79
  columns, CRLF): untouched bytes outside paragraphs and list bodies, the same
  scanned structure, idempotence; a write tool's one-line paragraph stored
  wrapped; reflow of an accepted changeset and of a maintained document
  with pending edits regenerates staging, after which ``palette generate``
  reports nothing and promotion still resolves every edit kind;
  ``palette reflow`` on this repository and ``palette check .`` clean.
- dispatch: ``alive_since`` on this process with a recorded time after its
  start (alive), with a recorded time more than one second before its start
  (dead), at the one-second boundary, with no recorded time and with a
  malformed timestamp (pid alone); the start-time reader on each CI operating
  system; ``wait``, the reconciliation and ``child_process_alive`` with a
  stand-in start-time provider for a reused pid, with timestamps written by
  SQLite; the description text; the retired terms absent (validate.sh).
- installer: ``slate-setup mcp`` over an mcp manifest recording ``aside`` as a
  registration only, as a registration with its binary, and on each harness
  with the stand-in CLIs (Kimi through the plugin manifest), with a second
  installation sharing the binary, with ``--dry-run``, repeated, with a
  refusing CLI and with a failed binary removal; the installed binaries
  recorded; the agy approvals following the stand-in palette's list including
  an added tool, the probe failure note, existing user entries kept, and no
  duplicate on reinstall.
- Workspace tests, clippy ``-D warnings`` and fmt on the latest stable on the
  three CI operating systems; ``validate: OK``; the records promoted.

Alternatives and costs
----------------------

- A lint rule on line length instead of wrapping in the server: it would report
  every hand-written document and change nothing the server writes. Rejected;
  the user asked for the server to wrap what it writes.
- Storing a process start time in the row at spawn and comparing it later: the
  owner's start time is not in the row today and a spawn-time read costs the
  same as a read at check time; comparing with the row's own timestamps needs
  no schema change. Rejected.
- A separate retirement command for ``mcp``: the kit path's plan and apply
  already do it; reusing them keeps one behaviour. Rejected.

Open questions
--------------

None.

References
----------

- Backlog items B-8, B-9, B-10, B-11, B-12, B-13, B-14 and the phase closes
  that recorded them; decisions D-1 and D-2 (state, 2026-10-14).
- RFC-0019 (document root), RFC-0021 (record_refresh), RFC-0022 (wait),
  RFC-0023 (agy approvals);
  ``shared/workflows/palette/templates/house-style.rst``.
- Linux ``proc(5)`` (``/proc/[pid]/stat`` field 22, ``/proc/stat`` ``btime``);
  Apple ``libproc.h`` (``proc_pidinfo``, ``PROC_PIDTBSDINFO``); Windows
  ``GetProcessTimes``.
