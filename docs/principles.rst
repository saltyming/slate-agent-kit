Principles — slate-agent-kit
============================

:Status: Maintained
:Date: 2026-09-29

One source; kits are rendered
-----------------------------

Every file a kit installs is rendered from this repository's sources. A
rendered file edited by hand is overwritten by the next render and diverges
from the other kits until then, so a change is made in ``shared/``,
``adapters/`` or ``tooling/`` and re-rendered.

Every harness and every platform
--------------------------------

A change holds for Claude Code, Codex and Kimi Code, and on Linux, macOS and
Windows, because the kits and the servers claim all of them. A difference
between harnesses is stated in that harness's adapter, never by leaving a
harness behind.

The standing corpus is a budget
-------------------------------

The rules every session loads are kept within hard byte budgets. They state
purpose, decision rights and judgment; procedures live in skills and tool
descriptions, which load only when used. A rule that grows the corpus has to
earn its bytes.

Code enforces what code can
---------------------------

A structural rule that code can check (document shape, identifiers, links,
budgets, render completeness) is checked by ``validate.sh`` or the palette
server rather than restated in prose, because prose rules on shape are not
followed reliably.

main stays installable
----------------------

The kits' installers fetch slate's ``main`` branch, so ``main`` is always
consumable. A breaking change is developed on a branch and released as a
whole, through the release train.
