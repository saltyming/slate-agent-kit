//! End-to-end install and uninstall for Kimi Code in an isolated home.

mod support;

use slate_setup::env::Harness;
use slate_setup::json::Json;
use std::fs;
use support::{ReleaseServer, Sandbox, fixture};

fn table(path: &std::path::Path) -> toml::Table {
    toml::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

const OTHER_PLUGIN_REGISTRY: &str = "{\n  \"version\": 1,\n  \"plugins\": [\n    {\n      \"id\": \"someone-elses-plugin\",\n      \"root\": \"/x\",\n      \"enabled\": true\n    }\n  ]\n}\n";

#[test]
fn install_and_uninstall_with_the_plugin_registry() {
    let mut sb = Sandbox::new(Harness::Kimi);
    let server = ReleaseServer::start(&sb.env.platform(), None, true, true);
    sb.use_release(&server);
    let home = sb.home();
    fs::create_dir_all(home.join("plugins")).unwrap();
    fs::copy(
        fixture("configs/kimi-config.toml"),
        home.join("config.toml"),
    )
    .unwrap();
    fs::write(home.join("plugins/installed.json"), OTHER_PLUGIN_REGISTRY).unwrap();
    let original = table(&home.join("config.toml"));
    let roots = sb.dir.path().join("work");
    fs::create_dir_all(&roots).unwrap();
    let roots_arg = roots.to_string_lossy().into_owned();

    let out = sb.run(
        "install",
        &[
            "--roots",
            roots_arg.as_str(),
            "--set",
            "subagent.model=alias-two",
            "--set",
            "subagent.effort=high",
        ],
    );
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(out.out.contains("Restart Kimi Code"), "{}", out.out);
    assert!(sb.cli_calls().is_empty(), "Kimi has no CLI to call");

    // The plugin folder.
    let plugin = home.join("plugins/managed/slate-agent-kit-mcp");
    let manifest =
        Json::parse(&fs::read_to_string(plugin.join("kimi.plugin.json")).unwrap()).unwrap();
    assert_eq!(
        manifest.get("version").and_then(Json::as_str),
        Some("13.0.0")
    );
    let servers = manifest.get("mcpServers").unwrap();
    for name in ["aside", "dispatch", "palette"] {
        let s = servers.get(name).unwrap_or_else(|| panic!("{name}"));
        let command = s.get("command").and_then(Json::as_str).unwrap();
        assert!(std::path::Path::new(command).is_file(), "{command}");
    }
    assert_eq!(
        servers
            .get("aside")
            .unwrap()
            .get("env")
            .unwrap()
            .get("ASIDE_HARNESS")
            .and_then(Json::as_str),
        Some("kimi")
    );
    assert!(
        servers
            .get("aside")
            .unwrap()
            .get("env")
            .unwrap()
            .get("KIMI_CODE_HOME")
            .is_none(),
        "default home"
    );
    assert_eq!(
        servers
            .get("palette")
            .unwrap()
            .get("env")
            .unwrap()
            .get("PALETTE_EXTRA_ROOTS")
            .and_then(Json::as_str),
        Some(roots_arg.as_str())
    );
    assert!(
        fs::read_to_string(plugin.join("SKILL.md"))
            .unwrap()
            .contains("palette tools")
    );

    // The registry keeps the other plugin and gains ours.
    let registry =
        Json::parse(&fs::read_to_string(home.join("plugins/installed.json")).unwrap()).unwrap();
    let ids: Vec<String> = registry
        .get("plugins")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.get("id").and_then(Json::as_str).unwrap().to_string())
        .collect();
    assert_eq!(ids, ["someone-elses-plugin", "slate-agent-kit-mcp"]);

    // config.toml: secondary model set, force and everything else untouched.
    let text = fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(text.contains("max_steps = 50 # keep"));
    let cfg = table(&home.join("config.toml"));
    assert_eq!(
        cfg["secondary_model"]["default_model"].as_str(),
        Some("alias-two")
    );
    assert_eq!(
        cfg["secondary_model"]["default_effort"].as_str(),
        Some("high")
    );
    assert!(cfg["secondary_model"].get("force").is_none());
    assert!(cfg["secondary_model"].get("models").is_none());
    assert_eq!(cfg["default_model"].as_str(), Some("alias-one"));

    // Reinstall keeps the plugin's installedAt.
    let first_installed = registry.get("plugins").unwrap().as_array().unwrap()[1]
        .get("installedAt")
        .cloned()
        .unwrap();
    let out = sb.run(
        "install",
        &[
            "--roots",
            roots_arg.as_str(),
            "--set",
            "subagent.model=alias-two",
            "--set",
            "subagent.effort=high",
        ],
    );
    assert_eq!(out.code, 0, "{}", out.out);
    let registry =
        Json::parse(&fs::read_to_string(home.join("plugins/installed.json")).unwrap()).unwrap();
    let plugins = registry.get("plugins").unwrap().as_array().unwrap();
    assert_eq!(plugins.len(), 2);
    assert_eq!(plugins[1].get("installedAt"), Some(&first_installed));

    // Configure changes only what it is told to: the effort, in prefs and in config.toml.
    let out = sb.run(
        "configure",
        &[
            "--roots",
            roots_arg.as_str(),
            "--set",
            "subagent.effort=medium",
        ],
    );
    assert_eq!(out.code, 0, "{}", out.out);
    let cfg = table(&home.join("config.toml"));
    assert_eq!(
        cfg["secondary_model"]["default_effort"].as_str(),
        Some("medium")
    );
    assert_eq!(
        cfg["secondary_model"]["default_model"].as_str(),
        Some("alias-two")
    );
    assert!(
        support::read(&sb.prefs_path("subagent")).contains("## Reasoning effort\n\n**medium**\n")
    );
    assert!(plugin.join("kimi.plugin.json").is_file());

    // Uninstall.
    let out = sb.run("uninstall", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(!plugin.exists());
    assert!(
        !home.join("plugins/managed").exists(),
        "created by the install, empty afterwards"
    );
    let registry =
        Json::parse(&fs::read_to_string(home.join("plugins/installed.json")).unwrap()).unwrap();
    assert_eq!(
        registry.get("plugins").unwrap().as_array().unwrap().len(),
        1
    );
    assert_eq!(table(&home.join("config.toml")), original);
    assert!(!home.join("AGENTS.md").exists());
}

#[test]
fn an_unknown_model_alias_is_rejected_before_anything_changes() {
    let mut sb = Sandbox::new(Harness::Kimi);
    let server = ReleaseServer::start(&sb.env.platform(), None, true, true);
    sb.use_release(&server);
    let home = sb.home();
    fs::create_dir_all(&home).unwrap();
    fs::copy(
        fixture("configs/kimi-config.toml"),
        home.join("config.toml"),
    )
    .unwrap();
    let out = sb.run("install", &["--set", "subagent.model=primary"]);
    assert_eq!(out.code, 2, "{}", out.out);
    assert!(out.out.contains("alias-one, alias-two"), "{}", out.out);
    assert_eq!(sb.tree(&home), ["config.toml"], "nothing else was written");
    assert!(!sb.bin_dir.exists(), "no binary was downloaded");
}

#[test]
fn a_config_without_models_skips_the_step_with_an_explanation() {
    let sb = Sandbox::new(Harness::Kimi);
    let home = sb.home();
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("config.toml"), "default_model = \"x\"\n").unwrap();
    // Interactive run: no roots (confirmed after the warning), then the defaults; the
    // subagent model questions are skipped and explained.
    let answers = format!("\ny\n{}", "\n".repeat(40));
    let answers = answers.as_str();
    let out = sb.run_scripted("install", &["--binaries", "skip"], answers);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(out.out.contains("no [models] aliases"), "{}", out.out);
    assert!(
        !fs::read_to_string(home.join("config.toml"))
            .unwrap()
            .contains("secondary_model")
    );
}

#[test]
fn a_custom_home_passes_kimi_code_home_to_the_servers_and_missing_roots_are_warned() {
    let mut sb = Sandbox::new(Harness::Kimi);
    let server = ReleaseServer::start(&sb.env.platform(), None, true, true);
    sb.use_release(&server);
    let custom = sb.dir.path().join("kimi-elsewhere");
    let out = sb.run("install", &["--home", custom.to_str().unwrap()]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(
        out.out
            .contains("dispatch and palette will reject every project"),
        "{}",
        out.out
    );
    let manifest = Json::parse(
        &fs::read_to_string(custom.join("plugins/managed/slate-agent-kit-mcp/kimi.plugin.json"))
            .unwrap(),
    )
    .unwrap();
    let aside_env = manifest
        .get("mcpServers")
        .unwrap()
        .get("aside")
        .unwrap()
        .get("env")
        .unwrap();
    assert_eq!(
        aside_env.get("KIMI_CODE_HOME").and_then(Json::as_str),
        Some(custom.to_str().unwrap())
    );
    let dispatch_env = manifest
        .get("mcpServers")
        .unwrap()
        .get("dispatch")
        .unwrap()
        .get("env")
        .unwrap();
    assert_eq!(
        dispatch_env.get("KIMI_CODE_HOME").and_then(Json::as_str),
        Some(custom.to_str().unwrap())
    );
    assert!(dispatch_env.get("DISPATCH_EXTRA_ROOTS").is_none());
}

#[test]
fn a_registry_and_folders_the_install_created_are_removed_with_it() {
    let mut sb = Sandbox::new(Harness::Kimi);
    let server = ReleaseServer::start(&sb.env.platform(), None, true, true);
    sb.use_release(&server);
    let roots = sb.dir.path().to_string_lossy().into_owned();
    let out = sb.run("install", &["--roots", roots.as_str()]);
    assert_eq!(out.code, 0, "{}", out.out);
    let home = sb.home();
    assert!(home.join("plugins/installed.json").is_file());
    let out = sb.run_scripted("uninstall", &[], "y\ny\n");
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(
        !home.exists(),
        "nothing of the install is left: {:?}\n{}",
        sb.tree(&home),
        out.out
    );
    assert!(!sb.bin_dir.exists());
}
