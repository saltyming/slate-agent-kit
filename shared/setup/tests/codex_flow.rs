//! End-to-end install, reinstall, configure and uninstall for Codex in an isolated home.

mod support;

use slate_setup::binaries::exe_name;
use slate_setup::env::Harness;
use slate_setup::rules::build_concat;
use std::fs;
use support::{ReleaseServer, Sandbox, fixture, norm, read};

fn table(path: &std::path::Path) -> toml::Table {
    toml::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn copy_custom_rules(sb: &Sandbox) -> std::path::PathBuf {
    let dir = sb.dir.path().join("custom-rules");
    fs::create_dir_all(&dir).unwrap();
    for f in ["style.md", "signed.md"] {
        fs::copy(fixture(&format!("custom-rules/{f}")), dir.join(f)).unwrap();
    }
    dir
}

#[test]
fn install_reinstall_configure_uninstall() {
    let mut sb = Sandbox::new(Harness::Codex);
    let platform = sb.env.platform();
    let server = ReleaseServer::start(&platform, None, true, true);
    sb.use_release(&server);
    let home = sb.home();
    fs::create_dir_all(&home).unwrap();
    fs::copy(
        fixture("configs/codex-config.toml"),
        home.join("config.toml"),
    )
    .unwrap();
    let original = table(&home.join("config.toml"));
    let custom = copy_custom_rules(&sb);
    let roots = sb.dir.path().join("work");
    fs::create_dir_all(&roots).unwrap();
    let (roots_arg, custom_arg) = (
        roots.to_string_lossy().into_owned(),
        custom.to_string_lossy().into_owned(),
    );
    let args = [
        "--roots",
        roots_arg.as_str(),
        "--custom-rules",
        custom_arg.as_str(),
        "--set",
        "subagent.model=model-s",
        "--set",
        "subagent.effort=xhigh",
        "--set",
        "aside.level=on-request",
    ];
    let out = sb.run("install", &args);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(out.out.contains("Restart Codex"), "{}", out.out);

    // The combined primary file: manual, every rule, then the custom rules, each once.
    let agents = read(&home.join("AGENTS.md"));
    let rules: Vec<String> = [
        "codex-agent-kit--task-execution.md",
        "codex-agent-kit--aside.md",
    ]
    .iter()
    .map(|r| read(&sb.payload.join("rules").join(r)))
    .collect();
    let style = read(&home.join("rules/codex-agent-kit--style.md"));
    let signed = read(&home.join("rules/codex-agent-kit--signed.md"));
    assert!(
        style.starts_with("<!-- codex-agent-kit-custom:user -->\n"),
        "{style}"
    );
    let expected = build_concat(
        &read(&sb.payload.join("AGENTS.md")),
        &rules,
        &[norm(&signed), norm(&style)],
    );
    // Custom rules are ordered by file name: signed.md before style.md.
    assert_eq!(agents, expected);
    assert_eq!(agents.matches("Prefer small functions.").count(), 1);
    assert_eq!(agents.matches("No TODOs.").count(), 1);
    // The rules are also copied for reference.
    assert!(
        home.join("rules/codex-agent-kit--task-execution.md")
            .is_file()
    );

    // Registration with CODEX_HOME set, in the order remove then add.
    let added = sb.added_servers();
    assert_eq!(added.len(), 3);
    assert_eq!(added[0].1, ["ASIDE_HARNESS=codex"]);
    let state = home.join("slate-agent-kit");
    assert!(
        added[1]
            .1
            .contains(&format!("SLATE_AGENT_STATE_HOME={}", state.display())),
        "{:?}",
        added[1].1
    );
    assert!(state.is_dir());
    assert!(
        sb.cli_env("CODEX_HOME")
            .iter()
            .all(|h| h == &home.to_string_lossy())
    );

    // config.toml: comments and untouched keys survive; the needed keys are added.
    let text = fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(text.starts_with("# My Codex configuration\n"));
    assert!(text.contains("model = \"model-x\"  # main model"));
    assert!(text.contains("theme = \"dark\" # keep this comment"));
    assert!(text.contains("# another server"));
    let cfg = table(&home.join("config.toml"));
    let aside = cfg["mcp_servers"]["aside"].as_table().unwrap();
    assert_eq!(aside["tool_timeout_sec"].as_integer(), Some(1800));
    assert_eq!(
        aside["default_tools_approval_mode"].as_str(),
        Some("approve")
    );
    assert_eq!(aside["env"]["ASIDE_HARNESS"].as_str(), Some("codex"));
    assert!(
        cfg["mcp_servers"]["dispatch"]
            .get("tool_timeout_sec")
            .is_none(),
        "only aside needs the long timeout"
    );
    let code_mode = cfg["features"]["code_mode"].as_table().unwrap();
    let excluded: Vec<&str> = code_mode["excluded_tool_namespaces"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(excluded, ["mcp__other", "mcp__aside"]);
    assert_eq!(
        code_mode["direct_only_tool_namespaces"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let palette_tools = cfg["mcp_servers"]["palette"]["tools"].as_table().unwrap();
    assert_eq!(
        palette_tools["palette_status"]["approval_mode"].as_str(),
        Some("approve")
    );
    assert_eq!(
        palette_tools["palette_read"]["approval_mode"].as_str(),
        Some("approve")
    );
    assert_eq!(
        cfg["agents"]["default_subagent_model"].as_str(),
        Some("model-s")
    );
    assert_eq!(
        cfg["agents"]["default_subagent_reasoning_effort"].as_str(),
        Some("xhigh")
    );
    assert_eq!(cfg["agents"]["max_threads"].as_integer(), Some(4));

    // Reinstall and configure: the custom rules still appear once and the config stays valid.
    let out = sb.run("install", &args);
    assert_eq!(out.code, 0, "{}", out.out);
    let out = sb.run("configure", &["--custom-rules", custom_arg.as_str()]);
    assert_eq!(out.code, 0, "{}", out.out);
    let agents = read(&home.join("AGENTS.md"));
    assert_eq!(
        agents.matches("Prefer small functions.").count(),
        1,
        "regenerated from scratch, not appended"
    );
    assert_eq!(
        agents.matches("\n---\n").count(),
        4,
        "five chunks (manual, 2 rules, 2 custom rules) have four separators"
    );
    let cfg = table(&home.join("config.toml"));
    assert_eq!(
        cfg["features"]["code_mode"]["excluded_tool_namespaces"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // Uninstall restores the configuration and removes what the kit installed.
    let out = sb.run("uninstall", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert_eq!(table(&home.join("config.toml")), original);
    let text = fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(text.contains("# My Codex configuration") && text.contains("# another server"));
    assert!(!home.join("AGENTS.md").exists());
    assert!(!home.join("rules/codex-agent-kit--aside.md").exists());
    assert!(
        home.join("rules/codex-agent-kit--style.md").is_file(),
        "custom rules are user-owned and stay"
    );
    assert!(sb.prefs_path("git").is_file());
    for name in ["aside", "dispatch", "palette"] {
        assert!(!sb.bin_dir.join(exe_name(&platform, name)).exists());
    }
    assert!(!home.join(".codex-agent-kit-manifest.toml").exists());
    // Folders the install created are gone once empty; the ones that hold your files stay.
    assert!(!home.join("skills").exists());
    assert!(!home.join("slate-agent-kit").exists());
    assert!(
        home.join("rules").is_dir(),
        "prefs and custom rules are still in rules/"
    );
}

#[test]
fn uninstall_removes_the_folders_the_install_created_and_only_those() {
    let mut sb = Sandbox::new(Harness::Codex);
    let server = ReleaseServer::start(&sb.env.platform(), None, true, true);
    sb.use_release(&server);
    let out = sb.run("install", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(sb.home().join("skills").is_dir() && sb.bin_dir.is_dir());
    // Answering yes to removing your own files leaves nothing behind.
    let out = sb.run_scripted("uninstall", &[], "y\ny\n");
    assert_eq!(out.code, 0, "{}", out.out);
    // Only config.toml, which the Codex CLI itself created, is left in the home.
    assert_eq!(sb.tree(&sb.home()), ["config.toml"], "{}", out.out);
    assert!(
        !sb.bin_dir.exists(),
        "the binary folder did not exist before the install"
    );

    // A folder that existed before the install is never removed, even when it ends up empty.
    let mut sb = Sandbox::new(Harness::Codex);
    let server = ReleaseServer::start(&sb.env.platform(), None, true, true);
    sb.use_release(&server);
    fs::create_dir_all(sb.home().join("skills")).unwrap();
    fs::create_dir_all(&sb.bin_dir).unwrap();
    let out = sb.run("install", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    let out = sb.run_scripted("uninstall", &[], "y\ny\n");
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(sb.home().join("skills").is_dir(), "skills/ existed before");
    assert!(sb.bin_dir.is_dir(), "the binary folder existed before");
    assert!(
        !sb.home().join("rules").exists(),
        "rules/ was created by the install"
    );
}

#[test]
fn a_scalar_code_mode_is_reported_and_the_file_is_never_dropped_silently() {
    let mut sb = Sandbox::new(Harness::Codex);
    let server = ReleaseServer::start(&sb.env.platform(), None, true, true);
    sb.use_release(&server);
    let home = sb.home();
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("config.toml"), "[features]\ncode_mode = true\n").unwrap();
    let out = sb.run("install", &[]);
    assert_eq!(out.code, 1, "{}", out.out);
    assert!(out.out.contains("scalar `code_mode`"), "{}", out.out);
    assert!(
        out.out.contains("remove the `code_mode = ...` line"),
        "{}",
        out.out
    );
    let text = fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(text.contains("code_mode = true"));
    // The servers are registered and the manifest records them, so the fix is one edit and a re-run.
    assert_eq!(sb.added_servers().len(), 3);
    assert!(home.join(".codex-agent-kit-manifest.toml").is_file());
}

#[test]
fn a_running_codex_binary_folder_from_the_earlier_windows_installer_is_cleaned_after_registration()
{
    let mut sb = Sandbox::new(Harness::Codex);
    let server = ReleaseServer::start(&sb.env.platform(), None, true, true);
    sb.use_release(&server);
    let old = sb.home().join("slate-agent-kit").join("bin");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("aside.exe"), "old").unwrap();
    let out = sb.run("install", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(
        !old.exists(),
        "the earlier installer's binaries are removed"
    );
}
