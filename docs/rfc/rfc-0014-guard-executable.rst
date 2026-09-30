RFC-0014: The parent-death guard is its own executable
======================================================

:Status: Accepted
:Implementation: complete — the agent-guard executable in the agent-exec
  crate, its lookup and modes, the installer and release entries for it
:Verification: runtime — 2026-09-30; guard tests on Linux and macOS and the
  Job Object on Windows through slate CI run 36726523883, aside and dispatch
  run on macOS with and without the executable; the release workflow's guard
  steps ran locally for two targets only
:Areas: MCP servers; aside; dispatch; installer
:Authors: Claude Fable 5.1
:Reviewers: Hamin Sung
:Implementers: Claude Fable 5.1 with Claude Sonnet and Claude Opus subagents
  (2026-09-30)
:Accepted: Hamin Sung (2026-09-30T10:58Z)
:Date: 2026-09-30
:Revised: none
:Depends: RFC-0008 (direct use of the agent-exec crate and its guard)
:Supersedes: RFC-0008 (in part: the guard as a re-invocation of the server
  binary that every ``main`` intercepts)
:Related: none
:Changes: spec/agent-exec.rst (Guard; Functions; Ownership and ordering)
:Description: The guard that kills a backend's process tree when its server
  dies runs as a dedicated executable, so no program that links the crate can
  re-execute itself.

Summary
-------

RFC-0008 moved dispatch's parent-death guard into the shared crate as it was:
the library re-invokes the running executable with a hidden first argument,
and the executable's ``main`` must intercept that argument before anything
else. The guard becomes ``agent-guard``, a small executable built from the
crate and installed beside the servers; the crate starts backends through it
and never through the running executable. A marker in the environment stops a
process that was started as a guard and is not one from starting backends.
The guard has three modes per run: required, preferred, off.

Problem and context
-------------------

The re-invocation is safe only while every executable that links the crate
keeps the promise about its ``main``. Inside dispatch there was one caller.
In a shared crate the promise has to hold for aside, dispatch, bench, every
test executable and any later tool, and nothing enforces it. An executable
that breaks it does not fail: the re-invoked copy does its ordinary work
again, which starts another guard, which re-invokes another copy.

Evidence
--------

- 2026-09-30, while the crate was being written: its integration-test
  executable (whose ``main`` is the Rust test harness) was re-invoked as a
  guard, ran the tests again and re-invoked itself; about 7,300 processes of
  ``target/debug/deps/run-<hash> __pdeath_guard ... -- codex ...`` accumulated
  against a per-user limit of 8,000 before they were killed. They could not
  be killed by name while the executable stayed runnable, because each one
  spawned the next.
- dispatch's guard (``pdeath_guard.rs``, ``main.rs`` "argv-sniffs for the
  hidden ``__pdeath_guard`` re-invocation") has one caller and its tests
  re-invoke the real ``dispatch`` binary, which is why the pattern held there.
- The same pattern in other projects carries the same rule and the same
  failure: a Go program using ``reexec`` must call its init hook first in
  ``main`` and in every test's ``TestMain``. Programs shared by several
  executables use a dedicated helper instead (Bazel's ``process-wrapper``,
  tini). macOS has no kernel facility that kills a process tree with its
  parent, so a watching process is needed there; Windows has the Job Object.

Goals and non-goals
-------------------

Goals:

- No executable can re-execute itself through the crate, whatever its
  ``main`` does.
- The tests run the guard that ships.
- A missing guard has a defined result per server.

Non-goals:

- A guard on Windows other than the Job Object.
- Kernel containment on Linux (cgroups, namespaces).

Requirements and invariants
---------------------------

- The crate starts a guarded backend only through ``agent-guard``, never
  through the running executable, and ``agent-guard`` never starts a guard.
- ``agent-guard`` leads a new process group, kills it when its parent dies and
  otherwise mirrors the program's exit status.
- A process that carries the guard marker in its environment cannot start a
  backend through the crate.
- ``Required`` without the executable is an error before any backend starts;
  ``Preferred`` runs unguarded and says so; ``Off`` never looks for it.
- No server's ``main`` contains guard code.

Design
------

The contract is the Guard section of ``spec/agent-exec.rst``. ``agent-guard``
is a second binary target of the agent-exec crate, single-threaded, with the
Linux and macOS watchers dispatch has today. The crate looks for it beside the
running executable, then on ``PATH``. dispatch and bench require it; aside
prefers it.

Impact and compatibility
------------------------

- The installer places ``agent-guard`` beside the servers on Linux and macOS,
  and the release carries it for those targets.
- A dispatch installed without it refuses to start backends and says how to
  install it; an aside without it answers as before, unguarded.
- dispatch's ``main`` loses its guard branch; the hidden ``__pdeath_guard``
  argument is gone.

Implementation and transition
-----------------------------

The executable is written with the crate, before aside and dispatch are
moved onto it; its tests use the built executable, run one at a time, and
count processes before and after.

Verification strategy
---------------------

- The guard kills a program and the program's own child when its parent is
  killed, and mirrors exit codes and signals otherwise, on Linux and macOS.
- A test executable that links the crate and runs a guarded backend leaves
  the process count where it was.
- A process started with the marker set refuses to start a backend.
- Each mode without the executable: error, unguarded with notice, no lookup.

Alternatives and costs
----------------------

- Keep the re-invocation and require the hook in every ``main`` and test
  harness: one more promise of the kind that failed.
- Become the guard by forking without ``exec``: no executable to ship, but the
  guard loop must use only async-signal-safe calls in a copy of a
  multi-threaded process.
- Intercept before ``main`` through a link-time initializer: relies on the
  standard library working before ``main``, which is not promised.

Open questions
--------------

None.

References
----------

- `RFC-0008 <rfc-0008-shared-backend-execution.rst>`_
- ``shared/mcp-servers/dispatch/src/{pdeath_guard.rs,winjob.rs,main.rs}``
