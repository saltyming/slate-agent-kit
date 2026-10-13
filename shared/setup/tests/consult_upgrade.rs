//! Upgrading a home where an earlier release installed the aside server and its prefs.
//!
//! Each test installs the current fixture kit, then writes into the home and the
//! manifest what the earlier installer left (the aside registration, binary,
//! Codex keys, Kimi plugin entry or aside prefs file), and installs again.

mod support;

use slate_setup::binaries::exe_name;
use slate_setup::config::{ConfigDoc, ConfigRecord, TomlDoc};
use slate_setup::env::Harness;
use slate_setup::json::Json;
use slate_setup::manifest::{Manifest, Registration};
use std::fs;
use std::path::{Path, PathBuf};
use support::{ReleaseServer, Sandbox, fixture, read};

fn manifest(sb: &Sandbox) -> Manifest {
    Manifest::load(&sb.home(), &sb.kit(), sb.harness)
        .unwrap()
        .unwrap()
}

fn aside_bin(sb: &Sandbox) -> PathBuf {
    sb.bin_dir.join(exe_name(&sb.env.platform(), "aside"))
}

/// Records an aside binary (and, for Claude Code and Codex, its registration) as the earlier installer did.
fn add_aside_past(sb: &Sandbox, m: &mut Manifest) {
    fs::create_dir_all(&sb.bin_dir).unwrap();
    fs::write(aside_bin(sb), "old aside").unwrap();
    m.record_binary(&aside_bin(sb));
    if sb.harness != Harness::Kimi {
        m.record_registration(Registration {
            kind: "mcp".into(),
            harness: sb.harness.name().into(),
            server: "aside".into(),
            path: None,
        });
    }
}

fn record(
    file: &Path,
    path: &[&str],
    previous: Option<&str>,
    written: &str,
    added: &[&str],
) -> ConfigRecord {
    ConfigRecord {
        file: file.to_path_buf(),
        path: path.iter().map(|s| s.to_string()).collect(),
        previous: previous.map(str::to_string),
        written: written.into(),
        created_parents: 0,
        added: added.iter().map(|s| s.to_string()).collect(),
    }
}

/// Writes the aside table and code-mode entries into `config.toml` and their records into
/// the manifest, as the earlier Codex installer did.
fn add_codex_aside_keys(sb: &Sandbox, m: &mut Manifest) {
    let file = sb.home().join("config.toml");
    let mut doc = TomlDoc::parse(&fs::read_to_string(&file).unwrap()).unwrap();
    let bin = aside_bin(sb).to_string_lossy().into_owned();
    doc.set(&["mcp_servers", "aside", "command"], &Json::String(bin))
        .unwrap();
    doc.set(&["mcp_servers", "aside", "args"], &Json::Array(vec![]))
        .unwrap();
    doc.set(
        &["mcp_servers", "aside", "env", "ASIDE_HARNESS"],
        &Json::str("codex"),
    )
    .unwrap();
    doc.set(
        &["mcp_servers", "aside", "tool_timeout_sec"],
        &Json::Number("1800".into()),
    )
    .unwrap();
    doc.set(
        &["mcp_servers", "aside", "default_tools_approval_mode"],
        &Json::str("approve"),
    )
    .unwrap();
    doc.append_string(
        &["features", "code_mode", "excluded_tool_namespaces"],
        "mcp__aside",
    )
    .unwrap();
    doc.append_string(
        &["features", "code_mode", "direct_only_tool_namespaces"],
        "mcp__aside",
    )
    .unwrap();
    fs::write(&file, doc.render()).unwrap();
    for r in [
        record(
            &file,
            &["mcp_servers", "aside", "tool_timeout_sec"],
            None,
            "1800",
            &[],
        ),
        record(
            &file,
            &["mcp_servers", "aside", "default_tools_approval_mode"],
            None,
            "\"approve\"",
            &[],
        ),
        record(
            &file,
            &["features", "code_mode", "excluded_tool_namespaces"],
            Some("[\"mcp__other\"]"),
            "[\"mcp__other\",\"mcp__aside\"]",
            &["mcp__aside"],
        ),
        record(
            &file,
            &["features", "code_mode", "direct_only_tool_namespaces"],
            None,
            "[\"mcp__aside\"]",
            &["mcp__aside"],
        ),
    ] {
        m.config.push(r);
    }
}

fn started(harness: Harness) -> (Sandbox, ReleaseServer) {
    let mut sb = Sandbox::new(harness);
    let server = ReleaseServer::start(&sb.env.platform(), None, true, true);
    sb.use_release(&server);
    (sb, server)
}

fn roots(sb: &Sandbox) -> String {
    sb.dir.path().to_string_lossy().into_owned()
}

#[test]
fn claude_unregisters_aside_and_removes_its_binary() {
    let (sb, _server) = started(Harness::Claude);
    let r = roots(&sb);
    let out = sb.run("install", &["--roots", r.as_str()]);
    assert_eq!(out.code, 0, "{}", out.out);
    let mut m = manifest(&sb);
    add_aside_past(&sb, &mut m);
    m.save(&sb.home()).unwrap();

    sb.clear_log();
    let out = sb.run("install", &["--roots", r.as_str()]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(out.out.contains("unregister aside"), "{}", out.out);
    assert!(sb.removed_servers().contains(&"aside".to_string()));
    let added: Vec<String> = sb.added_servers().into_iter().map(|(n, _)| n).collect();
    assert_eq!(added, ["dispatch", "palette"]);
    assert!(!aside_bin(&sb).exists());
    let m = manifest(&sb);
    assert!(m.registrations.iter().all(|r| r.server != "aside"), "{m:?}");
    assert!(!m.lists_binary(&aside_bin(&sb)));
    assert!(m.lists_binary(&sb.bin_dir.join(exe_name(&sb.env.platform(), "dispatch"))));

    // A later run has nothing left to retire.
    sb.clear_log();
    let out = sb.run("install", &["--roots", r.as_str()]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(!sb.removed_servers().contains(&"aside".to_string()));
    assert!(!out.out.contains("no longer ships"), "{}", out.out);
}

#[test]
fn the_binary_stays_while_another_manifest_lists_it() {
    let (sb, _server) = started(Harness::Claude);
    let r = roots(&sb);
    let out = sb.run("install", &["--roots", r.as_str()]);
    assert_eq!(out.code, 0, "{}", out.out);
    let mut m = manifest(&sb);
    add_aside_past(&sb, &mut m);
    m.save(&sb.home()).unwrap();
    // An earlier `install-mcp.sh` run in the Codex home also lists it.
    let codex_home = sb.env.default_home(Harness::Codex);
    let mut other = Manifest::new("slate-agent-kit-mcp");
    other.record_binary(&aside_bin(&sb));
    other.save(&codex_home).unwrap();

    let out = sb.run("install", &["--roots", r.as_str()]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(aside_bin(&sb).exists(), "{}", out.out);
    assert!(out.out.contains("also lists it"), "{}", out.out);
    assert!(sb.removed_servers().contains(&"aside".to_string()));
    let m = manifest(&sb);
    assert!(
        !m.lists_binary(&aside_bin(&sb)),
        "this kit no longer owns it"
    );
    assert!(m.registrations.iter().all(|r| r.server != "aside"));
}

#[test]
fn a_removal_the_cli_refuses_is_a_note_and_is_not_retried() {
    let (mut sb, _server) = started(Harness::Claude);
    let r = roots(&sb);
    let out = sb.run("install", &["--roots", r.as_str()]);
    assert_eq!(out.code, 0, "{}", out.out);
    let mut m = manifest(&sb);
    add_aside_past(&sb, &mut m);
    m.save(&sb.home()).unwrap();
    sb.env
        .child_env
        .insert("FAKE_CLI_FAIL".into(), "mcp remove aside".into());
    let out = sb.run("install", &["--roots", r.as_str()]);
    assert_eq!(
        out.code, 0,
        "a server that is not registered is no failure: {}",
        out.out
    );
    assert!(
        out.out
            .contains("mcp remove aside -s user` did not succeed"),
        "{}",
        out.out
    );
    assert!(
        out.out.contains("refusing"),
        "the CLI's message is shown: {}",
        out.out
    );
    assert!(
        manifest(&sb)
            .registrations
            .iter()
            .all(|r| r.server != "aside")
    );
    assert!(!aside_bin(&sb).exists());
}

#[test]
fn codex_restores_the_aside_keys_unregisters_and_removes_the_binary() {
    let (sb, _server) = started(Harness::Codex);
    let home = sb.home();
    fs::create_dir_all(&home).unwrap();
    fs::copy(
        fixture("configs/codex-config.toml"),
        home.join("config.toml"),
    )
    .unwrap();
    let original: toml::Table = toml::from_str(&read(&home.join("config.toml"))).unwrap();
    let r = roots(&sb);
    let out = sb.run("install", &["--roots", r.as_str()]);
    assert_eq!(out.code, 0, "{}", out.out);
    let mut m = manifest(&sb);
    add_aside_past(&sb, &mut m);
    add_codex_aside_keys(&sb, &mut m);
    m.save(&home).unwrap();
    let before: toml::Table = toml::from_str(&read(&home.join("config.toml"))).unwrap();
    assert!(before["mcp_servers"].get("aside").is_some());

    let out = sb.run("install", &["--roots", r.as_str()]);
    assert_eq!(out.code, 0, "{}", out.out);
    let text = read(&home.join("config.toml"));
    assert!(!text.contains("aside"), "{text}");
    assert!(
        text.contains("excluded_tool_namespaces = [\"mcp__other\"] # another server"),
        "{text}"
    );
    let cfg: toml::Table = toml::from_str(&text).unwrap();
    assert_eq!(
        cfg["features"], original["features"],
        "the code-mode lists are the user's again"
    );
    assert!(cfg["mcp_servers"]["dispatch"]["tools"]["dispatch_consult"].is_table());
    assert!(!aside_bin(&sb).exists());
    let m = manifest(&sb);
    assert!(
        m.config
            .iter()
            .all(|c| !c.path.iter().any(|p| p.contains("aside"))
                && !c.added.iter().any(|a| a.contains("aside"))
                && c.path[0] != "features"),
        "{:?}",
        m.config
    );
    assert!(m.registrations.iter().all(|r| r.server != "aside"));

    // Uninstall afterwards still restores the configuration the user had.
    let out = sb.run("uninstall", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    let after: toml::Table = toml::from_str(&read(&home.join("config.toml"))).unwrap();
    assert_eq!(after, original);
}

#[test]
fn codex_keys_are_restored_only_while_they_hold_what_the_installer_wrote() {
    let (mut sb, _server) = started(Harness::Codex);
    let home = sb.home();
    fs::create_dir_all(&home).unwrap();
    fs::copy(
        fixture("configs/codex-config.toml"),
        home.join("config.toml"),
    )
    .unwrap();
    let r = roots(&sb);
    let out = sb.run("install", &["--roots", r.as_str()]);
    assert_eq!(out.code, 0, "{}", out.out);
    let mut m = manifest(&sb);
    add_aside_past(&sb, &mut m);
    add_codex_aside_keys(&sb, &mut m);
    m.save(&home).unwrap();
    // The user changed the timeout after the earlier install.
    let file = home.join("config.toml");
    let text = read(&file).replace("tool_timeout_sec = 1800", "tool_timeout_sec = 600");
    fs::write(&file, text).unwrap();
    // Without the CLI nothing is unregistered, so the table stays and shows what was restored.
    sb.env.set_var("CODEX_BIN", "/definitely/not/there");
    let out = sb.run("configure", &["--roots", r.as_str()]);
    assert!(
        out.out.contains("mcp_servers.aside.tool_timeout_sec")
            && out.out.contains("was changed after the install"),
        "{}",
        out.out
    );
    assert!(
        out.out.contains("mcp remove aside"),
        "the command is printed: {}",
        out.out
    );
    let cfg: toml::Table = toml::from_str(&read(&file)).unwrap();
    let aside = cfg["mcp_servers"]["aside"].as_table().unwrap();
    assert_eq!(
        aside["tool_timeout_sec"].as_integer(),
        Some(600),
        "left as the user set it"
    );
    assert!(
        aside.get("default_tools_approval_mode").is_none(),
        "restored: it did not exist"
    );
    assert!(
        cfg["features"]["code_mode"]
            .get("direct_only_tool_namespaces")
            .is_none()
    );
    assert_eq!(
        cfg["features"]["code_mode"]["excluded_tool_namespaces"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // Still registered, so the binary and the registration stay recorded.
    assert!(aside_bin(&sb).exists());
    let m = manifest(&sb);
    assert!(m.registrations.iter().any(|r| r.server == "aside"));
    assert!(m.lists_binary(&aside_bin(&sb)));
    assert!(
        m.config.iter().all(|c| c.path[0] != "features"),
        "{:?}",
        m.config
    );
}

#[test]
fn kimi_rewrites_the_plugin_without_aside_even_when_binaries_are_skipped() {
    let (sb, _server) = started(Harness::Kimi);
    let home = sb.home();
    let r = roots(&sb);
    let out = sb.run("install", &["--roots", r.as_str()]);
    assert_eq!(out.code, 0, "{}", out.out);
    let mut m = manifest(&sb);
    add_aside_past(&sb, &mut m);
    m.save(&home).unwrap();
    let plugin = home.join("plugins/managed/slate-agent-kit-mcp/kimi.plugin.json");
    let mut j = Json::parse(&read(&plugin)).unwrap();
    if let Some(Json::Object(servers)) = j.get_mut("mcpServers") {
        servers.insert(
            0,
            (
                "aside".to_string(),
                Json::parse("{\"command\": \"/b/aside\", \"env\": {\"ASIDE_HARNESS\": \"kimi\"}}")
                    .unwrap(),
            ),
        );
    }
    fs::write(&plugin, j.to_pretty("  ")).unwrap();

    let out = sb.run("install", &["--roots", r.as_str(), "--binaries", "skip"]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(
        out.out.contains("remove from the Kimi plugin aside"),
        "{}",
        out.out
    );
    let j = Json::parse(&read(&plugin)).unwrap();
    let servers = j.get("mcpServers").unwrap();
    assert!(servers.get("aside").is_none());
    assert!(servers.get("dispatch").is_some() && servers.get("palette").is_some());
    let skill = read(&plugin.with_file_name("SKILL.md"));
    assert!(!skill.contains("aside"), "{skill}");
    assert!(!aside_bin(&sb).exists());
    assert!(!manifest(&sb).lists_binary(&aside_bin(&sb)));
}

fn aside_prefs(sb: &Sandbox) -> String {
    // The current layout under the old name: the user's values, a note and CRLF line endings.
    read(&fixture(&format!(
        "payload-{}/prefs/consult-prefs.md",
        sb.harness.name()
    )))
    .replace("custom:consult-prefs", "custom:aside-prefs")
    .replace("## Level\n\n**suggest**", "## Level\n\n**auto**")
    .replace("## Codex model\n\n****", "## Codex model\n\n**model-a**")
    .replace("Free-form.", "My own consultation note.")
    .replace('\n', "\r\n")
}

#[test]
fn the_aside_prefs_file_is_renamed_with_its_values_after_confirmation() {
    let sb = Sandbox::new(Harness::Codex);
    let old = sb.prefs_path("aside");
    fs::create_dir_all(old.parent().unwrap()).unwrap();
    let mine = aside_prefs(&sb);
    fs::write(&old, &mine).unwrap();
    let r = roots(&sb);

    // A dry run shows the rename and changes nothing.
    let out = sb.run(
        "install",
        &["--binaries", "skip", "--roots", r.as_str(), "--dry-run"],
    );
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(out.out.contains("rename"), "{}", out.out);
    assert_eq!(fs::read_to_string(&old).unwrap(), mine);
    assert!(!sb.prefs_path("consult").exists());

    let out = sb.run("install", &["--binaries", "skip", "--roots", r.as_str()]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(out.out.contains("renamed"), "{}", out.out);
    assert!(!old.exists());
    let now = fs::read_to_string(sb.prefs_path("consult")).unwrap();
    assert_eq!(
        now,
        mine.replace("custom:aside-prefs", "custom:consult-prefs"),
        "only the signature line changed"
    );
    assert!(now.contains("**model-a**") && now.contains("My own consultation note."));
    let m = manifest(&sb);
    assert!(m.files.iter().all(|f| f.path != old));
    assert!(
        m.files
            .iter()
            .any(|f| f.path == sb.prefs_path("consult") && f.user)
    );

    // The wizard shows the user's values, not the template's.
    let out = sb.run_scripted(
        "configure",
        &["--binaries", "skip", "--roots", r.as_str()],
        "y\n\n",
    );
    assert!(out.out.contains("Choice [auto]"), "{}", out.out);
}

#[test]
fn a_consult_file_wins_and_the_aside_file_is_left_and_reported() {
    let sb = Sandbox::new(Harness::Claude);
    let old = sb.prefs_path("aside");
    fs::create_dir_all(old.parent().unwrap()).unwrap();
    let mine = aside_prefs(&sb);
    fs::write(&old, &mine).unwrap();
    let consult = read(&fixture("payload-claude/prefs/consult-prefs.md"))
        .replace("## Level\n\n**suggest**", "## Level\n\n**on-request**");
    fs::write(sb.prefs_path("consult"), &consult).unwrap();
    let out = sb.run("install", &["--binaries", "skip"]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(
        out.out.contains("both exist") && out.out.contains("ignored"),
        "{}",
        out.out
    );
    let report = out.out.split("\nApplying").nth(1).unwrap_or_default();
    assert!(
        report.contains("both exist"),
        "the final report says it too: {report}"
    );
    assert_eq!(fs::read_to_string(&old).unwrap(), mine);
    assert_eq!(read(&sb.prefs_path("consult")), consult);
}
