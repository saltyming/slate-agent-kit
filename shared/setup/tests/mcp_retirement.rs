//! `slate-setup mcp` retires what its own manifest recorded and the release no
//! longer ships, the way an install does, and records the binaries it installs.
//!
//! Each test runs `mcp` once for a harness, writes into its mcp manifest and the
//! harness what an earlier release's `install-mcp.sh` left behind (an `aside`
//! registration, with or without its binary), and runs `mcp` again with the
//! stand-in harness CLIs. Nothing touches the real home.

mod support;

use slate_setup::binaries::exe_name;
use slate_setup::env::Harness;
use slate_setup::json::Json;
use slate_setup::layout::Layout;
use slate_setup::manifest::{Manifest, Registration};
use slate_setup::mcp::MCP_KIT;
use std::fs;
use std::path::PathBuf;
use support::{Outcome, ReleaseServer, Sandbox, read};

const ALL: [Harness; 4] = [Harness::Claude, Harness::Codex, Harness::Kimi, Harness::Agy];

/// The harnesses whose servers are registered through a CLI the stand-in replaces.
const WITH_CLI: [Harness; 3] = [Harness::Claude, Harness::Codex, Harness::Agy];

fn started(h: Harness) -> (Sandbox, ReleaseServer) {
    let mut sb = Sandbox::new(h);
    let server = ReleaseServer::start(&sb.env.platform(), None, false, true);
    sb.use_release(&server);
    (sb, server)
}

fn mcp(sb: &Sandbox, extra: &[&str]) -> Outcome {
    // Kimi's plugin runtime starts servers outside any project, so a root is always given.
    let roots = sb.dir.path().to_string_lossy().into_owned();
    let mut args = vec!["--harness", sb.harness.name(), "--roots", roots.as_str()];
    args.extend_from_slice(extra);
    sb.run("mcp", &args)
}

fn aside_bin(sb: &Sandbox) -> PathBuf {
    sb.bin_dir.join(exe_name(&sb.env.platform(), "aside"))
}

/// The layout of `h` in the sandbox's user home.
fn layout_in(sb: &Sandbox, h: Harness) -> Layout {
    Layout::new(h, &sb.env.default_home(h))
}

fn manifest_in(sb: &Sandbox, h: Harness) -> Manifest {
    Manifest::load(&layout_in(sb, h).manifest_dir, MCP_KIT, h)
        .unwrap()
        .unwrap_or_else(|| panic!("{}: no mcp manifest", h.name()))
}

fn manifest(sb: &Sandbox) -> Manifest {
    manifest_in(sb, sb.harness)
}

fn manifest_file(sb: &Sandbox) -> PathBuf {
    sb.layout().manifest_path(MCP_KIT)
}

fn plugin_file_in(sb: &Sandbox, h: Harness) -> PathBuf {
    sb.env
        .default_home(h)
        .join("plugins/managed/slate-agent-kit-mcp/kimi.plugin.json")
}

fn plugin_file(sb: &Sandbox) -> PathBuf {
    plugin_file_in(sb, sb.harness)
}

/// A first `mcp` run, so the home has binaries, registrations and a manifest.
fn first_run(sb: &Sandbox) {
    let out = mcp(sb, &[]);
    assert_eq!(out.code, 0, "{}: {}", sb.harness.name(), out.out);
}

/// What an earlier release left for `aside` in this harness and its mcp
/// manifest: a registration (Kimi: an entry in the plugin), and, when
/// `with_binary`, the binary file with the manifest listing it.
fn seed_aside(sb: &Sandbox, with_binary: bool) {
    seed_aside_in(sb, sb.harness, with_binary);
}

/// [`seed_aside`] for harness `h` of a sandbox that installs into several.
fn seed_aside_in(sb: &Sandbox, h: Harness, with_binary: bool) {
    let layout = layout_in(sb, h);
    let mut m = manifest_in(sb, h);
    match h {
        Harness::Kimi => {
            let plugin = plugin_file_in(sb, h);
            let mut j = Json::parse(&read(&plugin)).unwrap();
            if let Some(Json::Object(servers)) = j.get_mut("mcpServers") {
                servers.insert(
                    0,
                    (
                        "aside".to_string(),
                        Json::parse("{\"command\": \"/b/aside\"}").unwrap(),
                    ),
                );
            }
            fs::write(&plugin, j.to_pretty("  ")).unwrap();
        }
        Harness::Codex => {
            let config = layout.config_file.clone();
            let text = read(&config) + "\n[mcp_servers.aside]\ncommand = \"/b/aside\"\n";
            fs::write(&config, text).unwrap();
            m.record_registration(aside_registration(h));
        }
        Harness::Agy => {
            let path = sb
                .env
                .default_home(h)
                .join("config")
                .join("mcp_config.json");
            let registry = fs::read_to_string(&path).unwrap().replacen(
                "{\"mcpServers\":{",
                "{\"mcpServers\":{\"aside\":{},",
                1,
            );
            fs::write(&path, registry).unwrap();
            m.record_registration(aside_registration(h));
        }
        Harness::Claude => m.record_registration(aside_registration(h)),
    }
    if with_binary {
        fs::write(aside_bin(sb), "old aside").unwrap();
        m.record_binary(&aside_bin(sb));
    }
    m.save(&layout.manifest_dir).unwrap();
}

fn aside_registration(h: Harness) -> Registration {
    Registration {
        kind: "mcp".into(),
        harness: h.name().into(),
        server: "aside".into(),
        path: None,
    }
}

/// True when the harness no longer has `aside`: the CLI was asked to remove it
/// (Claude Code, Codex, agy) or the Kimi plugin lacks it.
fn aside_unregistered(sb: &Sandbox) -> bool {
    match sb.harness {
        Harness::Kimi => {
            let j = Json::parse(&read(&plugin_file(sb))).unwrap();
            j.get("mcpServers").unwrap().get("aside").is_none()
                && !read(&plugin_file(sb).with_file_name("SKILL.md")).contains("aside")
        }
        _ => sb.removed_servers().contains(&"aside".to_string()),
    }
}

/// The wording of the summary line that retires `aside` in this harness.
fn retire_line(h: Harness) -> &'static str {
    match h {
        Harness::Kimi => "remove from the Kimi plugin aside",
        _ => "unregister aside",
    }
}

fn registered_servers(sb: &Sandbox) -> Vec<String> {
    let mut v: Vec<String> = manifest(sb)
        .registrations
        .iter()
        .filter(|r| r.kind == "mcp")
        .map(|r| r.server.clone())
        .collect();
    v.sort();
    v
}

#[test]
fn an_aside_registration_alone_is_retired_and_its_unlisted_binary_is_left() {
    for h in ALL {
        let (sb, _server) = started(h);
        first_run(&sb);
        seed_aside(&sb, false);
        // The file is there, but no manifest lists it, so it is not this command's to remove.
        fs::write(aside_bin(&sb), "someone's aside").unwrap();
        sb.clear_log();

        let out = mcp(&sb, &[]);
        let name = h.name();
        assert_eq!(out.code, 0, "{name}: {}", out.out);
        assert!(out.out.contains("no longer ships"), "{name}: {}", out.out);
        assert!(out.out.contains(retire_line(h)), "{name}: {}", out.out);
        assert!(aside_unregistered(&sb), "{name}: {}", out.out);
        assert!(aside_bin(&sb).is_file(), "{name}: not listed, so it stays");
        let m = manifest(&sb);
        assert!(
            m.registrations.iter().all(|r| r.server != "aside"),
            "{name}"
        );
        assert!(!m.lists_binary(&aside_bin(&sb)), "{name}");
        if h != Harness::Kimi {
            assert_eq!(registered_servers(&sb), ["dispatch", "palette"], "{name}");
        }
        let added: Vec<String> = sb.added_servers().into_iter().map(|(n, _)| n).collect();
        if h != Harness::Kimi {
            assert_eq!(added, ["dispatch", "palette"], "{name}");
        }
    }
}

#[test]
fn an_aside_registration_with_its_binary_removes_both_and_the_servers_are_registered_after() {
    for h in ALL {
        let (sb, _server) = started(h);
        first_run(&sb);
        seed_aside(&sb, true);
        sb.clear_log();

        let out = mcp(&sb, &[]);
        let name = h.name();
        assert_eq!(out.code, 0, "{name}: {}", out.out);
        assert!(out.out.contains(retire_line(h)), "{name}: {}", out.out);
        assert!(
            out.out
                .contains(&format!("remove {}", aside_bin(&sb).display())),
            "{name}: {}",
            out.out
        );
        assert!(aside_gone(&sb), "{name}: {}", out.out);
        let m = manifest(&sb);
        assert!(!m.lists_binary(&aside_bin(&sb)), "{name}");
        assert!(
            m.registrations.iter().all(|r| r.server != "aside"),
            "{name}"
        );
        // The retirement came before the registrations.
        if h != Harness::Kimi {
            let calls: Vec<Vec<String>> = sb.cli_calls();
            let first_add = calls.iter().position(|c| c[1] == "add").unwrap();
            let aside_removal = calls
                .iter()
                .position(|c| c.get(1).map(String::as_str) == Some("remove") && c[2] == "aside")
                .unwrap();
            assert!(aside_removal < first_add, "{name}: {calls:?}");
        }
        // The release's servers are still installed, recorded and registered.
        for server in ["dispatch", "palette"] {
            let bin = sb.bin_dir.join(exe_name(&sb.env.platform(), server));
            assert!(bin.is_file() && m.lists_binary(&bin), "{name}: {server}");
        }
    }
}

/// The binary and the registration of `aside` are both gone.
fn aside_gone(sb: &Sandbox) -> bool {
    aside_unregistered(sb) && !aside_bin(sb).exists()
}

#[test]
fn a_binary_another_installation_lists_stays_and_is_no_longer_recorded_here() {
    for h in ALL {
        let (sb, _server) = started(h);
        first_run(&sb);
        seed_aside(&sb, true);
        // A second installation in another harness home also lists the binary.
        let other = if h == Harness::Claude {
            Harness::Codex
        } else {
            Harness::Claude
        };
        let other_dir =
            slate_setup::layout::Layout::new(other, &sb.env.default_home(other)).manifest_dir;
        fs::create_dir_all(&other_dir).unwrap();
        let mut theirs = Manifest::new("slate-agent-kit-mcp");
        theirs.record_binary(&aside_bin(&sb));
        theirs.save(&other_dir).unwrap();
        sb.clear_log();

        let out = mcp(&sb, &[]);
        let name = h.name();
        assert_eq!(out.code, 0, "{name}: {}", out.out);
        assert!(aside_unregistered(&sb), "{name}: {}", out.out);
        assert!(
            aside_bin(&sb).is_file(),
            "{name}: the other installation needs it"
        );
        assert!(out.out.contains("also listed by"), "{name}: {}", out.out);
        assert!(out.out.contains("also lists it"), "{name}: {}", out.out);
        assert!(!manifest(&sb).lists_binary(&aside_bin(&sb)), "{name}");
    }
}

#[test]
fn a_dry_run_shows_the_retirement_and_changes_nothing() {
    for h in ALL {
        let (sb, _server) = started(h);
        first_run(&sb);
        seed_aside(&sb, true);
        let before_manifest = fs::read(manifest_file(&sb)).unwrap();
        let before_tree = sb.tree(&sb.user_home);
        let before_plugin = (h == Harness::Kimi).then(|| fs::read(plugin_file(&sb)).unwrap());
        sb.clear_log();

        let out = mcp(&sb, &["--dry-run"]);
        let name = h.name();
        assert_eq!(out.code, 0, "{name}: {}", out.out);
        assert!(
            out.out.contains("Servers the new release no longer ships"),
            "{name}: {}",
            out.out
        );
        assert!(out.out.contains(retire_line(h)), "{name}: {}", out.out);
        assert!(
            out.out
                .contains(&format!("remove {}", aside_bin(&sb).display())),
            "{name}: {}",
            out.out
        );
        assert!(
            out.out.contains("Dry run: nothing was changed."),
            "{name}: {}",
            out.out
        );
        assert!(sb.cli_calls().is_empty(), "{name}: no CLI ran");
        assert!(aside_bin(&sb).is_file(), "{name}");
        assert_eq!(
            fs::read(manifest_file(&sb)).unwrap(),
            before_manifest,
            "{name}"
        );
        assert_eq!(sb.tree(&sb.user_home), before_tree, "{name}");
        if let Some(bytes) = before_plugin {
            assert_eq!(fs::read(plugin_file(&sb)).unwrap(), bytes, "{name}");
        }
    }
}

#[test]
fn running_mcp_again_retires_nothing_more() {
    for h in ALL {
        let (sb, _server) = started(h);
        first_run(&sb);
        seed_aside(&sb, true);
        let out = mcp(&sb, &[]);
        assert_eq!(out.code, 0, "{}: {}", h.name(), out.out);
        let settled = manifest(&sb);
        sb.clear_log();

        let out = mcp(&sb, &[]);
        let name = h.name();
        assert_eq!(out.code, 0, "{name}: {}", out.out);
        assert!(!out.out.contains("no longer ships"), "{name}: {}", out.out);
        assert!(
            !sb.removed_servers().contains(&"aside".to_string()),
            "{name}"
        );
        let again = manifest(&sb);
        assert_eq!(
            again.registrations.len(),
            settled.registrations.len(),
            "{name}"
        );
        assert_eq!(again.binaries, settled.binaries, "{name}");
        assert_eq!(again.config.len(), settled.config.len(), "{name}");
    }
}

#[test]
fn a_cli_that_refuses_the_removal_is_a_note_and_the_binary_still_goes() {
    for h in WITH_CLI {
        let (mut sb, _server) = started(h);
        first_run(&sb);
        seed_aside(&sb, true);
        sb.env
            .child_env
            .insert("FAKE_CLI_FAIL".into(), "mcp remove aside".into());
        sb.clear_log();

        let out = mcp(&sb, &[]);
        let name = h.name();
        assert_eq!(out.code, 0, "a refusal is no failure: {name}: {}", out.out);
        assert!(out.out.contains("did not succeed"), "{name}: {}", out.out);
        assert!(
            out.out.contains("refusing"),
            "the CLI's message: {name}: {}",
            out.out
        );
        assert!(out.out.contains("mcp remove aside"), "{name}: {}", out.out);
        let m = manifest(&sb);
        assert!(
            m.registrations.iter().all(|r| r.server != "aside"),
            "{name}"
        );
        assert!(!aside_bin(&sb).exists(), "{name}");
        assert!(!m.lists_binary(&aside_bin(&sb)), "{name}");

        // It is not tried again.
        sb.clear_log();
        let out = mcp(&sb, &[]);
        assert_eq!(out.code, 0, "{name}: {}", out.out);
        assert!(
            !sb.removed_servers().contains(&"aside".to_string()),
            "{name}"
        );
    }
}

#[test]
fn a_cli_that_cannot_run_keeps_the_registration_and_the_binary_recorded_and_prints_the_command() {
    for h in WITH_CLI {
        let (mut sb, _server) = started(h);
        first_run(&sb);
        seed_aside(&sb, true);
        let program_var = if h == Harness::Agy {
            "AGY_BIN"
        } else if h == Harness::Codex {
            "CODEX_BIN"
        } else {
            "CLAUDE_BIN"
        };
        sb.env.set_var(program_var, "/definitely/not/there");

        let out = mcp(&sb, &[]);
        let name = h.name();
        assert!(
            out.out.contains("mcp remove aside"),
            "the command is printed: {name}: {}",
            out.out
        );
        assert!(aside_bin(&sb).is_file(), "{name}: still registered");
        let m = manifest(&sb);
        assert!(
            m.registrations.iter().any(|r| r.server == "aside"),
            "{name}"
        );
        assert!(m.lists_binary(&aside_bin(&sb)), "{name}");
        assert!(
            out.out.contains("stays while aside is still registered"),
            "{name}: {}",
            out.out
        );
    }
}

#[test]
fn a_failed_binary_removal_is_a_problem_and_the_next_run_retries_it() {
    for h in ALL {
        let (sb, _server) = started(h);
        first_run(&sb);
        seed_aside(&sb, true);
        // A folder cannot be removed as a file, on any platform.
        fs::remove_file(aside_bin(&sb)).unwrap();
        fs::create_dir_all(aside_bin(&sb).join("inner")).unwrap();

        let out = mcp(&sb, &[]);
        let name = h.name();
        assert_eq!(out.code, 1, "{name}: {}", out.out);
        assert!(
            out.out.contains("no longer registered; its binary stays"),
            "{name}: {}",
            out.out
        );
        assert!(aside_unregistered(&sb), "{name}: {}", out.out);
        let m = manifest(&sb);
        assert!(
            m.registrations.iter().all(|r| r.server != "aside"),
            "{name}"
        );
        assert!(m.lists_binary(&aside_bin(&sb)), "{name}: still recorded");
        // The servers of this release were still registered.
        if h != Harness::Kimi {
            assert_eq!(registered_servers(&sb), ["dispatch", "palette"], "{name}");
        }

        // Once the obstacle is gone, the next run removes it and forgets it.
        fs::remove_dir_all(aside_bin(&sb)).unwrap();
        fs::write(aside_bin(&sb), "old aside").unwrap();
        let out = mcp(&sb, &[]);
        assert_eq!(out.code, 0, "{name}: {}", out.out);
        assert!(!aside_bin(&sb).exists(), "{name}: {}", out.out);
        assert!(!manifest(&sb).lists_binary(&aside_bin(&sb)), "{name}");
    }
}

#[test]
fn mcp_records_the_binaries_it_installs_in_each_harness_manifest() {
    let mut sb = Sandbox::new(Harness::Claude);
    let platform = sb.env.platform();
    let server = ReleaseServer::start(&platform, None, false, true);
    sb.use_release(&server);
    let out = sb.run("mcp", &["--harness", "claude,codex"]);
    assert_eq!(out.code, 0, "{}", out.out);
    let mut want: Vec<PathBuf> = ["dispatch", "palette"]
        .iter()
        .map(|n| sb.bin_dir.join(exe_name(&platform, n)))
        .collect();
    if !platform.contains("windows") {
        want.push(sb.bin_dir.join("agent-guard"));
    }
    want.sort();
    for h in [Harness::Claude, Harness::Codex] {
        let dir = slate_setup::layout::Layout::new(h, &sb.env.default_home(h)).manifest_dir;
        let m = Manifest::load(&dir, MCP_KIT, h).unwrap().unwrap();
        let mut got = m.binaries.clone();
        got.sort();
        assert_eq!(got, want, "{}", h.name());
    }

    // A run that installs no binaries (`--binaries skip`) keeps what is recorded.
    let out = sb.run("mcp", &["--harness", "claude", "--binaries", "skip"]);
    assert_eq!(out.code, 0, "{}", out.out);
    let dir =
        slate_setup::layout::Layout::new(Harness::Claude, &sb.env.default_home(Harness::Claude))
            .manifest_dir;
    let m = Manifest::load(&dir, MCP_KIT, Harness::Claude)
        .unwrap()
        .unwrap();
    let mut got = m.binaries.clone();
    got.sort();
    assert_eq!(got, want);

    // --uninstall unregisters and drops the manifest; the binaries stay.
    let out = sb.run("mcp", &["--harness", "claude", "--uninstall"]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(want.iter().all(|b| b.exists()));
    assert!(
        Manifest::load(&dir, MCP_KIT, Harness::Claude)
            .unwrap()
            .is_none()
    );
}

#[test]
fn an_unreadable_mcp_manifest_stops_the_run_before_anything_changes() {
    let (sb, _server) = started(Harness::Claude);
    let path = manifest_file(&sb);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "this is = not [valid toml").unwrap();

    let out = mcp(&sb, &[]);
    assert_ne!(out.code, 0, "{}", out.out);
    assert!(out.out.contains("cannot parse the manifest"), "{}", out.out);
    assert!(!sb.bin_dir.exists(), "no binary was installed");
    assert!(sb.cli_calls().is_empty());
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "this is = not [valid toml"
    );
}

// ── one run over several harnesses ───────────────────────────────────────

/// The sets of harnesses `mcp --harness` is run over: a pair with two CLIs, a
/// pair with the Kimi plugin and agy, and all four.
fn runs() -> Vec<Vec<Harness>> {
    vec![
        vec![Harness::Claude, Harness::Codex],
        vec![Harness::Kimi, Harness::Agy],
        ALL.to_vec(),
    ]
}

fn names(hs: &[Harness]) -> String {
    hs.iter().map(|h| h.name()).collect::<Vec<_>>().join(",")
}

fn mcp_over(sb: &Sandbox, hs: &[Harness], extra: &[&str]) -> Outcome {
    let roots = sb.dir.path().to_string_lossy().into_owned();
    let list = names(hs);
    let mut args = vec!["--harness", list.as_str(), "--roots", roots.as_str()];
    args.extend_from_slice(extra);
    sb.run("mcp", &args)
}

/// A sandbox where `mcp` ran over `hs` once and then every harness of `hs`
/// has an `aside` registration and lists the one `aside` binary.
fn aside_in_every_harness(hs: &[Harness]) -> (Sandbox, ReleaseServer) {
    let (sb, server) = started(hs[0]);
    let out = mcp_over(&sb, hs, &[]);
    assert_eq!(out.code, 0, "{}: {}", names(hs), out.out);
    for h in hs {
        seed_aside_in(&sb, *h, true);
    }
    sb.clear_log();
    (sb, server)
}

/// True when harness `h` no longer has `aside` registered.
fn aside_gone_from(sb: &Sandbox, h: Harness) -> bool {
    match h {
        Harness::Kimi => {
            let j = Json::parse(&read(&plugin_file_in(sb, h))).unwrap();
            j.get("mcpServers").unwrap().get("aside").is_none()
        }
        Harness::Codex => !read(&sb.env.default_home(h).join("config.toml")).contains("aside"),
        Harness::Agy => {
            let registry = sb
                .env
                .default_home(h)
                .join("config")
                .join("mcp_config.json");
            !read(&registry).contains("aside")
        }
        Harness::Claude => sb
            .cli_calls()
            .iter()
            .any(|c| c[..] == ["mcp", "remove", "aside", "-s", "user"].map(String::from)[..]),
    }
}

/// How many times the stand-in CLIs were asked to remove `aside`.
fn aside_removals(sb: &Sandbox) -> usize {
    sb.removed_servers()
        .iter()
        .filter(|s| *s == "aside")
        .count()
}

fn with_cli(hs: &[Harness]) -> usize {
    hs.iter().filter(|h| WITH_CLI.contains(h)).count()
}

fn lists_aside_binary(sb: &Sandbox, h: Harness) -> bool {
    manifest_in(sb, h).lists_binary(&aside_bin(sb))
}

#[test]
fn harnesses_retiring_the_same_binary_in_one_run_remove_it_once() {
    for hs in runs() {
        let run = names(&hs);
        let (sb, _server) = aside_in_every_harness(&hs);
        let bin = aside_bin(&sb);

        let out = mcp_over(&sb, &hs, &[]);
        assert_eq!(out.code, 0, "{run}: {}", out.out);
        assert!(!out.out.contains("with problems"), "{run}: {}", out.out);
        // The summary decided to remove it, for every harness that lists it.
        assert_eq!(
            out.out
                .matches(&format!("remove {}", bin.display()))
                .count(),
            hs.len(),
            "{run}: {}",
            out.out
        );
        assert!(
            !out.out.contains(&format!("keep {}", bin.display())),
            "{run}: {}",
            out.out
        );
        // It went once, and every manifest stopped listing it.
        assert!(!bin.exists(), "{run}");
        assert_eq!(
            out.out
                .matches(&format!("removed {}", bin.display()))
                .count(),
            1,
            "{run}: {}",
            out.out
        );
        assert_eq!(aside_removals(&sb), with_cli(&hs), "{run}");
        for h in &hs {
            assert!(aside_gone_from(&sb, *h), "{run}: {}", h.name());
            assert!(!lists_aside_binary(&sb, *h), "{run}: {}", h.name());
            assert!(
                manifest_in(&sb, *h)
                    .registrations
                    .iter()
                    .all(|r| r.server != "aside"),
                "{run}: {}",
                h.name()
            );
        }

        // Repeating the command retires nothing more.
        sb.clear_log();
        let out = mcp_over(&sb, &hs, &[]);
        assert_eq!(out.code, 0, "{run}: {}", out.out);
        assert!(!out.out.contains("no longer ships"), "{run}: {}", out.out);
        assert_eq!(aside_removals(&sb), 0, "{run}");
    }
}

#[test]
fn a_kit_manifest_listing_the_binary_keeps_it_for_every_harness_of_the_run() {
    for hs in runs() {
        let run = names(&hs);
        let (sb, _server) = aside_in_every_harness(&hs);
        let bin = aside_bin(&sb);
        // An installed kit lists the binary too; it is not part of this run.
        let kit_dir = layout_in(&sb, hs[0]).manifest_dir;
        let mut kit = Manifest::new("some-agent-kit");
        kit.record_binary(&bin);
        kit.save(&kit_dir).unwrap();

        let out = mcp_over(&sb, &hs, &[]);
        assert_eq!(out.code, 0, "{run}: {}", out.out);
        assert!(!out.out.contains("with problems"), "{run}: {}", out.out);
        assert!(bin.is_file(), "{run}: the kit needs it");
        assert_eq!(
            out.out
                .matches(&format!("keep {} (also listed by", bin.display()))
                .count(),
            hs.len(),
            "{run}: {}",
            out.out
        );
        assert!(out.out.contains("also lists it"), "{run}: {}", out.out);
        assert!(
            !out.out.contains(&format!("removed {}", bin.display())),
            "{run}"
        );
        for h in &hs {
            assert!(aside_gone_from(&sb, *h), "{run}: {}", h.name());
            assert!(!lists_aside_binary(&sb, *h), "{run}: {}", h.name());
        }

        // Nothing is left to retire, and the kit's file is still there.
        sb.clear_log();
        let out = mcp_over(&sb, &hs, &[]);
        assert_eq!(out.code, 0, "{run}: {}", out.out);
        assert!(!out.out.contains("no longer ships"), "{run}: {}", out.out);
        assert!(bin.is_file(), "{run}");
    }
}

#[test]
fn an_mcp_manifest_of_a_harness_outside_the_run_keeps_the_binary() {
    let hs = [Harness::Claude, Harness::Codex];
    let (sb, _server) = aside_in_every_harness(&hs);
    let bin = aside_bin(&sb);
    // agy's mcp manifest is not part of this run, and it lists the binary.
    let agy_dir = layout_in(&sb, Harness::Agy).manifest_dir;
    fs::create_dir_all(&agy_dir).unwrap();
    let mut elsewhere = Manifest::new(MCP_KIT);
    elsewhere.record_binary(&bin);
    elsewhere.save(&agy_dir).unwrap();

    let out = mcp_over(&sb, &hs, &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(bin.is_file(), "{}", out.out);
    assert!(
        out.out
            .contains(&format!("keep {} (also listed by", bin.display())),
        "{}",
        out.out
    );
    for h in hs {
        assert!(aside_gone_from(&sb, h), "{}", h.name());
        assert!(!lists_aside_binary(&sb, h), "{}", h.name());
    }
    // The manifest of the harness that was not in the run still lists it.
    assert!(
        Manifest::load(&agy_dir, MCP_KIT, Harness::Agy)
            .unwrap()
            .unwrap()
            .lists_binary(&bin)
    );
}

#[test]
fn a_dry_run_over_several_harnesses_shows_the_decision_and_changes_nothing() {
    for with_kit in [false, true] {
        let hs = [Harness::Claude, Harness::Codex];
        let (sb, _server) = aside_in_every_harness(&hs);
        let bin = aside_bin(&sb);
        if with_kit {
            let mut kit = Manifest::new("some-agent-kit");
            kit.record_binary(&bin);
            kit.save(&layout_in(&sb, hs[0]).manifest_dir).unwrap();
        }
        let before = sb.tree(&sb.user_home);
        let manifests: Vec<Vec<u8>> = hs
            .iter()
            .map(|h| fs::read(layout_in(&sb, *h).manifest_path(MCP_KIT)).unwrap())
            .collect();

        let out = mcp_over(&sb, &hs, &["--dry-run"]);
        assert_eq!(out.code, 0, "{}", out.out);
        let remove = format!("remove {}", bin.display());
        let keep = format!("keep {} (also listed by", bin.display());
        if with_kit {
            assert_eq!(out.out.matches(&keep).count(), 2, "{}", out.out);
            assert!(!out.out.contains(&remove), "{}", out.out);
        } else {
            assert_eq!(out.out.matches(&remove).count(), 2, "{}", out.out);
            assert!(!out.out.contains(&keep), "{}", out.out);
        }
        assert!(
            out.out.contains("Dry run: nothing was changed."),
            "{}",
            out.out
        );
        assert!(sb.cli_calls().is_empty(), "no CLI ran");
        assert!(bin.is_file());
        assert_eq!(sb.tree(&sb.user_home), before);
        for (h, bytes) in hs.iter().zip(&manifests) {
            assert_eq!(
                &fs::read(layout_in(&sb, *h).manifest_path(MCP_KIT)).unwrap(),
                bytes
            );
        }
    }
}

#[test]
fn a_harness_that_still_has_the_server_registered_holds_the_binary_for_the_whole_run() {
    let hs = [Harness::Claude, Harness::Codex];
    let (mut sb, _server) = aside_in_every_harness(&hs);
    let bin = aside_bin(&sb);
    // Codex cannot be asked to unregister, so its registration stays.
    sb.env.set_var("CODEX_BIN", "/definitely/not/there");

    let out = mcp_over(&sb, &hs, &[]);
    assert!(bin.is_file(), "{}", out.out);
    assert!(
        out.out
            .contains("stays while another harness in this run still has aside registered"),
        "{}",
        out.out
    );
    assert!(
        out.out.contains("mcp remove aside"),
        "the command is printed: {}",
        out.out
    );
    assert!(aside_gone_from(&sb, Harness::Claude));
    assert!(
        manifest_in(&sb, Harness::Codex)
            .registrations
            .iter()
            .any(|r| r.server == "aside")
    );
    for h in hs {
        assert!(lists_aside_binary(&sb, h), "{}: still recorded", h.name());
    }

    // Once Codex can be asked, the next run removes it, once.
    sb.env
        .set_var("CODEX_BIN", support::fake_cli().to_string_lossy());
    sb.clear_log();
    let out = mcp_over(&sb, &hs, &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(!bin.exists(), "{}", out.out);
    assert!(out.out.contains("was already removed"), "{}", out.out);
    assert!(aside_gone_from(&sb, Harness::Codex));
    for h in hs {
        assert!(!lists_aside_binary(&sb, h), "{}", h.name());
    }
}
