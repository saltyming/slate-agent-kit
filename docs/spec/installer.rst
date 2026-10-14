Installer
=========

:Status: Contract
:Date: 2026-09-30

Scope and authority
-------------------
The installer installs one kit into one harness home: the kit's instruction
files, rules and skills; the dispatch and palette MCP servers; the prefs
files; and the harness's native configuration. It also reconfigures and
uninstalls. It is one Rust program, ``slate-setup``, built from the slate
workspace crate ``shared/setup`` and shipped prebuilt for the same eight targets
as dispatch and palette. Every harness and every operating system runs the
same code; no step uses node, python, jq, awk or PowerShell logic of its own.

This document is the contract; ``shared/setup`` implements it, and each kit's
``dist/kit.toml`` descriptor is authoritative for what that kit installs.

Definitions and model
---------------------
Components
~~~~~~~~~~
``slate-setup``
  The installer. It reads a kit payload and its descriptor and does every step
  below.

Kit payload
  The ``dist/`` folder of a kit repository, rendered by slate's
  ``tooling/render-kit.sh``.

Descriptor
  ``dist/kit.toml``, rendered by slate, naming the kit, its harness, its files
  and the slate release its binaries come from.

Entry points
  ``install.sh``, ``install.ps1`` and ``Makefile`` at a kit's root, rendered by
  slate from one template each. They obtain ``slate-setup`` and the payload and
  run the installer; they implement no installer step.

Release assets
  ``slate-setup-<target>``, ``dispatch-<target>`` and ``palette-<target>``
  archives, and ``agent-guard-<target>`` for the Linux and macOS targets,
  listed in the release's ``checksums.txt``.

Kit payload
~~~~~~~~~~~
::

   dist/
     kit.toml
     CLAUDE.md | AGENTS.md        the kit's primary instruction file
     rules/<kit>--<name>.md       rule files
     skills/<name>/SKILL.md       one folder per skill, with any files it ships
     prefs/<name>-prefs.md        prefs templates (spec/prefs.rst)

A kit's root keeps ``AGENTS.md`` with the instructions for maintaining that
repository; it is never installed.

Descriptor
~~~~~~~~~~
``dist/kit.toml``::

   kit = "claude-agent-kit"          # file prefix and manifest name
   harness = "claude"                # claude | codex | kimi | agy
   version = "26.10.1"               # kit version, YY.MM.N
   contract = 1                      # compatibility contract (Compatibility)
   slate_version = "26.10.1"         # slate release that provides the binaries
   primary = "CLAUDE.md"             # file in dist/
   load = "rules-dir"                # rules-dir | concat
   rules = ["claude-agent-kit--task-execution.md", "..."]   # concat order
   skills = ["palette-init", "..."]
   prefs = ["consult", "dispatch", "subagent", "git", "comment"]
   servers = ["dispatch", "palette"]
   legacy = ["workslate"]            # optional legacy cleanups, claude only

``load = "rules-dir"`` means the harness loads every file in its rules folder
(Claude Code; Antigravity, whose rules folder loads a file only when it opens
with the YAML frontmatter block the render emits in front of the signature).
``load = "concat"`` means the harness loads only its primary file, so the
installer writes the primary file followed by every rule file in ``rules``
order and then every custom rule file, each preceded by a line ``---`` with
blank lines around it (Codex, Kimi Code).

Harness homes
~~~~~~~~~~~~~
Each harness has a layout (``layout::Layout``: the home, the manual folder,
the rules folder, the skills folder, the manifest folder, the backup folder,
the servers' state home and the native configuration file) that every shared
step consults; ``layout::manifest_dirs`` lists every harness's manifest folder
for the shared-binary ownership checks, and ``layout::check_home`` refuses a
``--home`` the harness cannot use.

``claude``
  ``--home``, else ``$CLAUDE_CONFIG_DIR``, else ``~/.claude``. Primary file
  ``CLAUDE.md``; rules in ``rules/``; skills in ``skills/``; settings in
  ``settings.json``; the manifest at the home root.

``codex``
  ``--home``, else ``$CODEX_HOME``, else ``~/.codex``. Primary file
  ``AGENTS.md``; rules copied to ``rules/`` for reference; skills in ``skills/``;
  configuration in ``config.toml``; the manifest at the home root.

``kimi``
  ``--home``, else ``$KIMI_CODE_HOME``, else ``~/.kimi-code``. Primary file
  ``AGENTS.md``; rules copied to ``rules/`` for reference; skills in
  ``skills/``; configuration in ``config.toml``; plugins in ``plugins/``; the
  manifest at the home root.

``agy``
  ``~/.gemini`` under the process home (``HOME``; ``USERPROFILE`` on Windows).
  Antigravity has no home override, so ``--home`` is accepted only when it
  names that folder and is refused otherwise with a message saying to set the
  process home instead. Primary file ``config/AGENTS.md``; rules and prefs in
  ``config/rules/``, each with the frontmatter block; skills in
  ``config/skills/``; the servers in ``config/mcp_config.json``, written by
  ``agy mcp``; approvals in ``antigravity-cli/settings.json``; the manifest,
  the backups (``backups/``) and the servers' state home under
  ``slate-agent-kit/``. ``config/`` is shared with the Antigravity
  application.

The binary folder is ``--bin-dir``, else ``~/.local/bin`` on every operating
system (``%USERPROFILE%\.local\bin`` on Windows).

Contract
--------
Commands
~~~~~~~~

``slate-setup install``
  Full install or reinstall.

``slate-setup configure``
  Prefs, custom rules, native configuration and server registration only.

``slate-setup uninstall``
  Reverse what install recorded.

``slate-setup mcp``
  Binaries and server registration only, for one or more harnesses; used by
  slate's ``tooling/install-mcp.sh``.

Common options: ``--payload <dir>`` (default: the ``dist/`` beside the running
entry point), ``--home <dir>`` (for agy only its own folder, Harness homes),
``--bin-dir <dir>``, ``--binaries prebuilt|build|skip`` (default
``prebuilt``), ``--slate-dir <dir>`` (the checkout to build from), ``--roots
<paths>`` (an OS path list), ``--set <key>=<value>`` (repeatable; keys in
spec/prefs.rst), ``--custom-rules <dir|none>``, ``--yes``, ``--dry-run``.
``mcp`` also takes ``--harness <name>`` (repeatable or comma-separated),
``--uninstall`` and ``--slate-version``; it records its registrations, edits
and installed binaries in ``.slate-agent-kit-mcp-manifest.toml`` in the
harness's manifest folder (the home; ``~/.gemini/slate-agent-kit`` for agy).
Before registering it retires the servers that manifest recorded and the run
no longer registers, as install does (Legacy cleanup, Retired servers), with
the plan in its summary; it removes a binary only through that retirement.
The environment variable ``CUSTOM_RULES_DIR`` seeds ``--custom-rules`` and is
never required.

Terminal experience
~~~~~~~~~~~~~~~~~~~
- Every run has the same shape in every harness: detect, ask, summarize and
  confirm, apply, report. Nothing is changed before the confirmation.
- The ask phase shows numbered steps (``[2/6] MCP servers``). Each question
  shows its allowed values as a menu or a list, and the current value as the
  default. An invalid answer is rejected with the allowed values and asked
  again. A question that does not apply to earlier answers is not asked: a
  backend's model and effort only for the chosen backend; effort only where the
  harness supports it.
- An existing prefs file is kept unless the user chooses to reconfigure it.
- The summary lists every file to write or back up, every registration, and
  every configuration key with its old and new value.
- Prompts read from the terminal even when standard input is a pipe
  (``/dev/tty`` on Unix, ``CONIN$`` on Windows). A run has a terminal only when
  standard output or standard error is one; a console that is merely attached
  (a CI step, a process started with piped output) does not count. Without a
  terminal, or with ``--yes``, every question takes its current or default value
  and the run never waits for input.
- Color and symbols are used only on a terminal and when ``NO_COLOR`` is unset.
- A failure names what failed, what state it left, and the command that fixes
  or retries it. The final report lists installed paths and says which harness
  to restart.
- ``--dry-run`` prints the summary and exits.

Install steps
~~~~~~~~~~~~~
1. Detect the harness home, the installed kit version from the manifest, the
   prefs files (renaming a prefs file that carries an earlier release's name
   first, spec/prefs.rst *Migration*), the harness CLI (``claude``,
   ``codex``), legacy leftovers, and the servers the previous manifest
   recorded that the descriptor no longer lists.
2. Ask: binaries mode; workspace roots; each prefs file (spec/prefs.rst); the
   custom rules folder.
3. Summarize and confirm.
4. Legacy cleanup for every entry in ``legacy`` (below), for binaries and
   registrations that earlier installers left elsewhere, and for the retired
   servers.
5. Binaries: dispatch and palette into the binary folder and, on Linux and
   macOS, ``agent-guard`` beside them.
6. Payload files: primary file, rules and skills; for ``concat`` the combined
   primary file.
7. Prefs files and custom rules.
8. Server registration and native configuration.
9. Report.

Binaries
~~~~~~~~
- ``prebuilt`` downloads ``<name>-<target>.tar.gz`` (``.zip`` on Windows) from
  the slate release ``v<slate_version>``; when that release does not exist it
  uses the latest release and says so. Every archive is checked against the
  release's ``checksums.txt``; a mismatch aborts; a missing checksum file warns.
- ``build`` runs ``cargo build --release -p dispatch -p palette`` in
  ``--slate-dir``, and on Linux and macOS then ``cargo build --release -p
  agent-exec --bin agent-guard``.
- ``agent-guard`` is the executable dispatch starts its backends through on
  Linux and macOS. It is installed, replaced and recorded like a server binary
  and is never registered with a harness. ``prebuilt`` downloads
  ``agent-guard-<target>.tar.gz`` and checks it like the others; a release
  that does not carry it installs the servers and adds one warning that
  dispatch refuses to start execution backends and runs consultations
  unguarded until a release that ships it is installed; a checksum entry for
  an asset that is absent is an error. Nothing is installed for it on Windows.
- ``skip`` installs no binary and registers nothing.
- A binary is replaced by writing a temporary file and renaming it over the
  old one, so a running server keeps its old file; on macOS the new file is
  signed ad hoc.

Payload files and signatures
~~~~~~~~~~~~~~~~~~~~~~~~~~~~
- A kit-managed Markdown file starts with ``<!-- slate-agent-kit:common -->``
  or ``<!-- <kit> -->``. A user-owned file starts with ``<!-- <kit>-custom:``.
  Either signature may follow a closed YAML frontmatter block (a ``---`` line,
  lines, a ``---`` line) as the first line after it
  (``signature::frontmatter_end``, ``signature::signature_line``); the
  Antigravity kit's rule and prefs files are written that way, and the
  classification reads through the block for every harness. An unclosed block
  leaves the file unrecognised.
- Install overwrites kit-managed files and never overwrites a user-owned one.
  A kit-managed file the previous manifest lists and the new payload no longer
  has is removed, and the summary says so.
- A user-owned prefs file under the name an earlier release gave it is read
  under its current name and renamed after the confirmation
  (spec/prefs.rst, Migration).
- An existing primary file that is not kit-managed is copied to
  ``<file>.bak-<UTC timestamp>`` before it is replaced; the backup is recorded
  (for agy under the kit's ``backups/`` folder, ``util::backup_file_in``).
- A skill folder is replaced as a whole when its ``SKILL.md`` is kit-managed.
- Custom rules: every ``*.md`` in the chosen folder is copied into the rules
  folder as ``<kit>--<name>.md`` with the user-owned signature added when
  missing. For agy the always-on frontmatter block
  (``signature::always_on_block``, description
  ``Harness::rule_frontmatter_title``) is added first when the file has none,
  and the signature follows the block
  (``signature::ensure_custom_signature_with_frontmatter``), so Antigravity
  loads the rule. A file that would replace a kit-managed one is refused. For
  ``concat`` harnesses the combined primary file is regenerated from scratch
  on every install and configure, so a custom rule appears exactly once.

Server registration
~~~~~~~~~~~~~~~~~~~

Environment per server:

- dispatch: ``SLATE_AGENT_STATE_HOME`` (Claude: the Claude home; Codex, Kimi
  and Agy: the layout's state home, ``<home>/slate-agent-kit``);
  ``SLATE_HARNESS=<harness>``, the harness whose session log a consultation
  forwards; ``DISPATCH_EXTRA_ROOTS`` when roots were given; for Kimi with a
  non-default home also ``KIMI_CODE_HOME``.
- palette: ``PALETTE_EXTRA_ROOTS`` when roots were given.

The roots question is asked for every harness. For Kimi, whose plugin runtime
starts servers outside any project, an empty answer is allowed only after a
warning that dispatch and palette will reject every project.

The palette server's read-only tools are the names the installed binary prints
for ``palette --read-only-tools``, probed once per run for every harness; a
probe that fails writes no palette approvals for any harness and the summary
says so.

``claude``
  ``claude mcp remove <server> -s user`` then
  ``claude mcp add <server> -s user --transport stdio -e <K=V>... -- <binary>``.
  The palette server's read-only tools and dispatch's consultation and read
  tools (``dispatch_consult``, ``dispatch_status``, ``dispatch_list``,
  ``dispatch_logs``, ``dispatch_backends``) are added to ``permissions.allow``
  in ``settings.json`` as ``mcp__<server>__<tool>``, in one edit. When the home
  is not the default, the CLI runs with ``CLAUDE_CONFIG_DIR`` set to it if the
  installed Claude Code honors that variable; otherwise registration is
  skipped and the commands are printed.

``codex``
  ``codex mcp remove`` then ``codex mcp add`` with ``CODEX_HOME`` set. Then in
  ``config.toml`` each of dispatch's consultation and read tools
  (``dispatch_consult``, ``dispatch_status``, ``dispatch_list``,
  ``dispatch_logs``, ``dispatch_backends``) and each read-only tool of the
  palette server is pre-approved as ``[mcp_servers.<server>.tools.<tool>]``
  with ``approval_mode = "approve"``, only when ``codex mcp add`` wrote the
  server's table; the server-level approval stays at the default. No tool
  timeout and no code-mode namespace is written.

``kimi``
  The plugin folder ``<home>/plugins/managed/slate-agent-kit-mcp/`` holds
  ``kimi.plugin.json`` and ``SKILL.md``. The manifest has ``name``
  (``slate-agent-kit-mcp``), ``version`` (the kit version), ``description``,
  ``keywords``, ``mcpServers`` with one entry per server (``command`` the
  binary, ``args`` empty, ``cwd`` the plugin folder, ``env`` as above), and
  ``interface`` (``displayName``, ``shortDescription``, ``developerName``).
  ``SKILL.md`` names the expected tool prefixes, among them
  ``dispatch_consult``. ``<home>/plugins/installed.json`` gets or updates the
  plugin's entry and keeps every other entry; an unreadable registry is backed
  up before it is rebuilt.

``agy``
  ``agy mcp remove <server>`` then ``agy mcp add --env <K=V>... <server>
  <binary>`` (the flags before the name), run with the process home, so agy
  writes ``~/.gemini/config/mcp_config.json`` itself. Then dispatch's
  consultation and read tools (``dispatch_consult``, ``dispatch_status``,
  ``dispatch_list``, ``dispatch_logs``, ``dispatch_backends``, a fixed list)
  and the palette server's read-only tools as probed are added to
  ``permissions.allow`` in ``~/.gemini/antigravity-cli/settings.json`` as
  ``mcp(<server>/<tool>)``, in one edit that creates the file and the
  ``permissions`` object when absent, adds no entry already present, and keeps
  every other key and the user's own entries; the edit is recorded in the
  manifest and reversed at uninstall and retirement. A missing ``agy`` turns
  the registration into a report of the commands, as for the other harnesses.

Native configuration
~~~~~~~~~~~~~~~~~~~~
The subagent default model and effort from ``subagent-prefs.md`` are written
only when set, and only after validation:

``claude``
  ``settings.json`` ``env.CLAUDE_CODE_SUBAGENT_MODEL``. Valid: ``sonnet``,
  ``opus``, ``haiku``, ``fable``, ``inherit``, or an identifier starting with
  ``claude-``. Effort is not configurable and not asked.

``codex``
  ``config.toml`` ``[agents]`` ``default_subagent_model`` and
  ``default_subagent_reasoning_effort`` (``low``, ``medium``, ``high``,
  ``xhigh``, ``max``, ``ultra``).

``kimi``
  ``config.toml`` ``[secondary_model]`` ``default_model``, chosen from the keys
  of ``[models]`` (offered as a menu; when ``[models]`` has none the step is
  skipped with an explanation), and ``default_effort`` (``low``, ``medium``,
  ``high``, ``xhigh``, ``max``). ``force`` is never set and no pool entry is
  named ``primary``.

``agy``
  None: Antigravity has no subagent default-model setting. The values are
  validated and written to the prefs file only, where the agent reads them as
  what to pass to ``define_subagent``; any model name is accepted.

Every configuration edit keeps all keys, comments and formatting it does not
change, and records the key's previous value (or its absence) in the manifest.

Manifest
~~~~~~~~
``.<kit>-manifest.toml`` in the harness's manifest folder (the home for
claude, codex and kimi; ``~/.gemini/slate-agent-kit`` for agy) records: kit
and version; every installed file and folder; every backup; every
configuration key edited with its previous value; every registration. The
checks that ask whether another kit still lists a binary read every harness's
manifest folder (``layout::manifest_dirs``). An older line-format manifest
(``<home>/.<kit>-manifest``) is read once for compatibility and replaced.

Uninstall
~~~~~~~~~

- Removes the kit-managed files and folders the manifest lists after checking
  their signatures; lists the user-owned ones and removes them only if the
  user chooses to (default: keep).
- Unregisters the servers, removes the Kimi plugin entry and the Claude and
  Antigravity permission entries it added (for Antigravity the
  ``mcp(<server>/<tool>)`` strings the manifest recorded, and a
  ``settings.json`` the installer created when nothing else was written into
  it).
- Restores each edited configuration key to its recorded previous value, or
  removes it if it did not exist, but only when its current value is still the
  one the installer wrote; otherwise it reports the key and leaves it.
- Removes a binary only when no other kit's manifest in any harness's
  manifest folder lists it; ``agent-guard`` also stays while another kit's
  manifest lists dispatch.
- Runs the legacy cleanups. The kit's state folder stays while it holds
  backups, which are never removed.

Legacy cleanup
~~~~~~~~~~~~~~

``workslate`` (claude)
  Removes the hook entries in ``settings.json`` whose command contains
  ``workslate`` and ``--hook=``, or ``[workslate-task-verify]``, after backing
  up the file; removes ``<bin-dir>/workslate``; runs
  ``claude mcp remove workslate -s user`` when the home is the default; removes
  ``workslate.db``, ``workslate.db-wal`` and ``workslate.db-shm`` under
  ``<home>/projects/*/workslate/`` and the emptied folder.

Earlier installer locations
  Binaries in ``<CODEX_HOME>\slate-agent-kit\bin`` from the earlier Windows
  Codex installer are removed after the new registration succeeds.

Retired servers
  A server the previous manifest recorded and the descriptor no longer lists
  (``aside`` from releases before 26.10.1) is retired on install and
  configure, also with ``--binaries skip``; ``mcp`` retires, the same way and
  in the same order, a server its own manifest recorded that the run no longer
  registers. For Claude Code, Codex and Antigravity the recorded servers are
  the manifest's registrations; for Kimi, whose manifest records the plugin and
  not its servers, they are the ``mcpServers`` of the installer's plugin; a
  server whose binary the manifest lists counts too. In order: the
  configuration keys recorded for that server (those under
  ``mcp_servers.<server>`` and the list entries ``mcp__<server>`` and
  ``mcp__<server>__<tool>``) are restored, newest first, each only while it
  holds what the installer wrote, and from a shared list only those entries;
  the server is unregistered (``claude mcp remove <server> -s user``, ``codex
  mcp remove <server>`` with ``CODEX_HOME``, or its entry and keyword removed
  from ``kimi.plugin.json`` with ``SKILL.md`` rewritten); its binary, when the
  retiring manifest lists it, is removed unless another manifest in any
  harness's manifest folder, a kit's or an ``mcp`` one, lists it, and a kept
  binary is reported and no longer recorded by the retiring manifest; a binary
  the retiring manifest does not list is left in place. An ``mcp`` run over
  several harnesses does not count its own manifests that retire the same
  server as keeping the binary: it unregisters the server in every harness
  first and then removes the binary once, and while one of those harnesses
  still has the server registered the binary stays recorded in each of them
  and the next run retries; a binary already gone when its removal comes counts
  as removed. The keys come before the unregistration because ``codex mcp
  remove`` deletes the server's whole table. A removal the CLI runs and refuses
  (it also refuses a server that is not registered) is reported with its
  message and the command to run by hand, and the registration is no longer
  recorded; a CLI that cannot run keeps the registration and the binary
  recorded, prints the commands and retries on the next run; a failed key
  restore or binary removal is a problem. The manifest changes only for the
  parts that succeeded, and the summary lists each server, its binary and each
  key, with the decision on the binary, before anything changes.

Entry points
~~~~~~~~~~~~
``install.sh``
  POSIX ``sh`` with ``set -eu``. Uses the payload beside it when
  ``dist/kit.toml`` exists there; otherwise downloads the kit repository archive
  for ``--ref`` (default ``main``) and extracts it to a temporary folder. Uses
  ``slate-setup`` from ``--slate-dir`` (building it with cargo) when
  ``--binaries build`` is given; otherwise downloads and verifies the prebuilt
  binary for the platform from the release ``v<slate_version>``. Runs it with
  every argument passed through and with standard input from ``/dev/tty`` when
  available. Removes its temporary files on exit.

``install.ps1``
  The same for Windows with a zip archive.

``Makefile``
  ``install``, ``configure``, ``uninstall`` and ``help`` targets that run
  ``install.sh`` against the local payload; installer options go in ``ARGS``
  (``make install ARGS="--yes"``).

Both scripts keep the earlier installers' options as aliases: ``--uninstall``
(``-Uninstall``) selects the ``uninstall`` command, ``--skip-mcp``
(``-SkipMcp``) becomes ``--binaries skip``, and ``DISPATCH_ROOTS``
(``-DispatchRoots``) becomes ``--roots`` when no ``--roots`` is given.

Errors and edge cases
---------------------
- A failure names what failed, the state it left and the command that fixes or
  retries it; the exit code is non-zero.
- A file whose signature is neither the kit's nor ``-custom:`` is listed as
  foreign and left in place; a user-owned file is never replaced without the
  user's choice.
- A configuration key whose current value is no longer the one the installer
  wrote is reported and left alone on uninstall.
- A missing harness CLI turns the registration step into a report of the
  commands the user runs by hand; the file steps still complete.
- An unreadable manifest is reported with the path and the run stops before
  changing anything.

Ownership and ordering
----------------------
The installer owns the kit-signed files, the entries it registers and the
configuration keys it records in the manifest; the user owns everything with a
``-custom:`` signature and every key the manifest does not list. Steps run in
the order legacy cleanup, stale files, retired servers, binaries, payload,
prefs renames, prefs, native configuration, server registration, manifest; a
step that fails leaves the earlier steps' results in place and the manifest
describing them.

Compatibility
-------------
The manifest format, the descriptor fields and the command names are stable
within one contract number, the descriptor's ``contract``; a release that
changes one of them incompatibly raises the number, and the release number
(``YY.MM.N``) says nothing about compatibility. A descriptor without
``contract`` is contract 1. An older line-format manifest and 12.x or 0.7.x
prefs files are read once and replaced. A new descriptor field is compatible;
a removed or renamed one is not.

Conformance
-----------

``cargo test -p slate-setup`` covers the descriptor, the manifest, the prefs
parser, migration and rename, the configuration edits for each harness, the
retirement of a recorded server on each harness, and the install, reinstall,
upgrade-from-previous-release and uninstall flows in a scratch home with
stand-in harness CLIs; for Antigravity also the exact filesystem diff of an
install against the recorded set, the byte-identical reinstall, the
uninstall that keeps the user's permission entries and unknown keys, the
ownership classification of files with a frontmatter block (BOM, CRLF, an
unclosed block), the ``--home`` refusal, shared binaries kept in both
directions, ``mcp`` and retirement, and a layout regression that keeps the
three earlier harnesses' install, uninstall and ``mcp`` outputs
byte-identical. ``mcp`` retirement is covered on each harness over a manifest
recording a server as a registration alone and with its binary, with a second
installation sharing the binary, with ``--dry-run``, repeated, with a refusing
CLI and with a failed binary removal, and the installed binaries recorded; the
agy approvals follow the stand-in palette's tool list, an added tool included,
with the probe failure note, the user's entries kept and no duplicate on
reinstall. The tests pass on Linux, macOS and Windows.

References
----------
- ``RFC-0001`` for the levels the prefs files hold.
- ``spec/prefs.rst`` for the prefs files this installer writes.
- ``shared/setup`` and ``tooling/kit-scripts/`` for the implementation and the
  kit entry points.
