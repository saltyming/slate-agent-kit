Installer
=========

:Status: Contract
:Date: 2026-09-30

Scope and authority
-------------------
The installer installs one kit into one harness home: the kit's instruction
files, rules and skills; the aside, dispatch and palette MCP servers; the prefs
files; and the harness's native configuration. It also reconfigures and
uninstalls. It is one Rust program, ``slate-setup``, built from the slate
workspace crate ``shared/setup`` and shipped prebuilt for the same eight targets
as aside and dispatch. Every harness and every operating system runs the same
code; no step uses node, python, jq, awk or PowerShell logic of its own.

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
  ``slate-setup-<target>`` and ``palette-<target>`` archives beside the aside
  and dispatch archives, listed in the release's ``checksums.txt``.

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
   harness = "claude"                # claude | codex | kimi
   version = "13.0.0"                # kit version
   slate_version = "0.7.0"           # slate release that provides the binaries
   primary = "CLAUDE.md"             # file in dist/
   load = "rules-dir"                # rules-dir | concat
   rules = ["claude-agent-kit--task-execution.md", "..."]   # concat order
   skills = ["palette-init", "..."]
   prefs = ["aside", "dispatch", "subagent", "git", "comment"]
   servers = ["aside", "dispatch", "palette"]
   legacy = ["workslate"]            # optional legacy cleanups, claude only

``load = "rules-dir"`` means the harness loads every file in its rules folder
(Claude Code). ``load = "concat"`` means the harness loads only its primary
file, so the installer writes the primary file followed by every rule file in
``rules`` order and then every custom rule file, each preceded by a line
``---`` with blank lines around it (Codex, Kimi Code).

Harness homes
~~~~~~~~~~~~~
``claude``
  ``--home``, else ``$CLAUDE_CONFIG_DIR``, else ``~/.claude``. Primary file
  ``CLAUDE.md``; rules in ``rules/``; skills in ``skills/``; settings in
  ``settings.json``.

``codex``
  ``--home``, else ``$CODEX_HOME``, else ``~/.codex``. Primary file
  ``AGENTS.md``; rules copied to ``rules/`` for reference; skills in ``skills/``;
  configuration in ``config.toml``.

``kimi``
  ``--home``, else ``$KIMI_CODE_HOME``, else ``~/.kimi-code``. Primary file
  ``AGENTS.md``; rules copied to ``rules/`` for reference; skills in
  ``skills/``; configuration in ``config.toml``; plugins in ``plugins/``.

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
entry point), ``--home <dir>``, ``--bin-dir <dir>``,
``--binaries prebuilt|build|skip`` (default ``prebuilt``), ``--slate-dir <dir>``
(the checkout to build from), ``--roots <paths>`` (an OS path list),
``--set <key>=<value>`` (repeatable; keys in spec/prefs.rst),
``--custom-rules <dir|none>``, ``--yes``, ``--dry-run``. ``mcp`` also takes
``--harness <name>`` (repeatable or comma-separated), ``--uninstall`` and
``--slate-version``; it records its registrations and edits in
``<home>/.slate-agent-kit-mcp-manifest.toml`` and never removes binaries. The
environment variable ``CUSTOM_RULES_DIR`` seeds ``--custom-rules`` and is never
required.

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
   prefs files, the harness CLI (``claude``, ``codex``), and legacy leftovers.
2. Ask: binaries mode; workspace roots; each prefs file (spec/prefs.rst); the
   custom rules folder.
3. Summarize and confirm.
4. Legacy cleanup for every entry in ``legacy`` (below) and for binaries and
   registrations that earlier installers left elsewhere.
5. Binaries: aside, dispatch and palette into the binary folder.
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
- ``build`` runs ``cargo build --release -p aside -p dispatch -p palette`` in
  ``--slate-dir``.
- ``skip`` installs no binary and registers nothing.
- A binary is replaced by writing a temporary file and renaming it over the
  old one, so a running server keeps its old file; on macOS the new file is
  signed ad hoc.

Payload files and signatures
~~~~~~~~~~~~~~~~~~~~~~~~~~~~
- A kit-managed Markdown file starts with ``<!-- slate-agent-kit:common -->``
  or ``<!-- <kit> -->``. A user-owned file starts with ``<!-- <kit>-custom:``.
- Install overwrites kit-managed files and never overwrites a user-owned one.
  A kit-managed file the previous manifest lists and the new payload no longer
  has is removed, and the summary says so.
- An existing primary file that is not kit-managed is copied to
  ``<file>.bak-<UTC timestamp>`` before it is replaced; the backup is recorded.
- A skill folder is replaced as a whole when its ``SKILL.md`` is kit-managed.
- Custom rules: every ``*.md`` in the chosen folder is copied into ``rules/`` as
  ``<kit>--<name>.md`` with the user-owned signature added when missing. A file
  that would replace a kit-managed one is refused. For ``concat`` harnesses the
  combined primary file is regenerated from scratch on every install and
  configure, so a custom rule appears exactly once.

Server registration
~~~~~~~~~~~~~~~~~~~
Environment per server:

- aside: ``ASIDE_HARNESS=<harness>``; for Kimi with a non-default home also
  ``KIMI_CODE_HOME``.
- dispatch: ``SLATE_AGENT_STATE_HOME`` (Claude: the Claude home; Codex and
  Kimi: ``<home>/slate-agent-kit``); ``DISPATCH_EXTRA_ROOTS`` when roots were
  given; for Kimi with a non-default home also ``KIMI_CODE_HOME``.
- palette: ``PALETTE_EXTRA_ROOTS`` when roots were given.

The roots question is asked for every harness. For Kimi, whose plugin runtime
starts servers outside any project, an empty answer is allowed only after a
warning that dispatch and palette will reject every project.

``claude``
  ``claude mcp remove <server> -s user`` then
  ``claude mcp add <server> -s user --transport stdio -e <K=V>... -- <binary>``.
  The palette server's read-only tools (the names printed by
  ``palette --read-only-tools``) are added to ``permissions.allow`` in
  ``settings.json`` as ``mcp__palette__<tool>``. When the home is not the
  default, the CLI runs with ``CLAUDE_CONFIG_DIR`` set to it if the installed
  Claude Code honors that variable; otherwise registration is skipped and the
  commands are printed.

``codex``
  ``codex mcp remove`` then ``codex mcp add`` with ``CODEX_HOME`` set. Then in
  ``config.toml``: ``[mcp_servers.aside]`` gets ``tool_timeout_sec = 1800`` and
  ``default_tools_approval_mode = "approve"``; ``[features.code_mode]`` lists
  ``mcp__aside`` in both ``excluded_tool_namespaces`` and
  ``direct_only_tool_namespaces``, keeping every other entry. A scalar
  ``code_mode`` key under ``[features]`` is reported and the edit is refused.
  Each read-only tool of the palette server is pre-approved as
  ``[mcp_servers.palette.tools.<tool>]`` with ``approval_mode = "approve"``;
  the server-level approval of palette stays at the default.

``kimi``
  The plugin folder ``<home>/plugins/managed/slate-agent-kit-mcp/`` holds
  ``kimi.plugin.json`` and ``SKILL.md``. The manifest has ``name``
  (``slate-agent-kit-mcp``), ``version`` (the kit version), ``description``,
  ``keywords``, ``mcpServers`` with one entry per server (``command`` the
  binary, ``args`` empty, ``cwd`` the plugin folder, ``env`` as above), and
  ``interface`` (``displayName``, ``shortDescription``, ``developerName``). ``<home>/plugins/installed.json`` gets or updates the
  plugin's entry and keeps every other entry; an unreadable registry is backed
  up before it is rebuilt.

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

Every configuration edit keeps all keys, comments and formatting it does not
change, and records the key's previous value (or its absence) in the manifest.

Manifest
~~~~~~~~
``<home>/.<kit>-manifest.toml`` records: kit and version; every installed file
and folder; every backup; every configuration key edited with its previous
value; every registration. An older line-format manifest
(``<home>/.<kit>-manifest``) is read once for compatibility and replaced.

Uninstall
~~~~~~~~~
- Removes the kit-managed files and folders the manifest lists after checking
  their signatures; lists the user-owned ones and removes them only if the
  user chooses to (default: keep).
- Unregisters the servers, removes the Kimi plugin entry and the Claude
  permission entries it added.
- Restores each edited configuration key to its recorded previous value, or
  removes it if it did not exist, but only when its current value is still the
  one the installer wrote; otherwise it reports the key and leaves it.
- Removes a binary only when no other kit's manifest in any harness home lists
  it.
- Runs the legacy cleanups.

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
the order legacy cleanup, stale files, binaries, payload, prefs, native
configuration, server registration, manifest; a step that fails leaves the
earlier steps' results in place and the manifest describing them.

Compatibility
-------------
The manifest format, the descriptor fields and the command names are stable
within a major version of a kit. An older line-format manifest and 12.x or
0.7.x prefs files are read once and replaced. A new descriptor field is
compatible; a removed or renamed one is not.

Conformance
-----------
``cargo test -p slate-setup`` covers the descriptor, the manifest, the prefs
parser and migration, the configuration edits for each harness, and the
install, reinstall, upgrade-from-previous-release and uninstall flows in a
scratch home with stand-in harness CLIs; the tests pass on Linux, macOS and
Windows.

References
----------
- ``RFC-0001`` for the levels the prefs files hold.
- ``spec/prefs.rst`` for the prefs files this installer writes.
- ``shared/setup`` and ``tooling/kit-scripts/`` for the implementation and the
  kit entry points.
