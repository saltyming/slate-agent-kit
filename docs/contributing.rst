Contributing — slate-agent-kit
==============================

:Status: Maintained
:Date: 2026-09-30

Branches
--------

Ordinary changes go to ``main`` directly. A breaking release is developed on a
``next`` branch in this repository and in each kit submodule, and lands on
``main`` through one pull request per repository, fast-forwarded. Kit
branches are pushed before the slate branch that pins them, so a checkout with
``submodules: recursive`` always resolves the pins.

Commits
-------

Conventional Commits: ``<type>(<area>): <subject>``, a blank line, then a
body; types ``feat``, ``fix``, ``docs``, ``chore``, ``refactor``, ``test``,
``perf``; the area is optional. Commits are unsigned (``--no-gpg-sign``) and
carry no model attribution. One commit holds one source change together with
the render output it requires; the kit pins are bumped in the same commit as
the slate change that produced them.

Pull requests
-------------

The body states, in this order:

- what changed, by source area (rules, palette, servers, installer, docs,
  tooling), with the records (RFC or ADR) that decided it;
- the verification that ran: the commands, the CI run, and what was not run;
- whether kit submodule pins, kit versions or release artifacts are affected.

A reviewer checks that rendered outputs were not hand-edited, that
``validate.sh`` and ``palette check`` pass on the checkout, and that a change
to a public contract has its record.

Verification
------------

Before a change is proposed::

  cargo test --workspace
  cargo clippy --workspace --all-targets --all-features -- -D warnings
  cargo fmt --all -- --check
  sh tooling/render-kit.sh claude   # and codex, kimi, when shared/ or adapters/ changed
  sh tooling/validate.sh
  palette check .

CI repeats the Rust steps on Linux, macOS and Windows; a change is not
complete until that run is green.
