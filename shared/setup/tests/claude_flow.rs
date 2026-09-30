//! End-to-end install, reinstall, configure and uninstall for Claude Code in an isolated home.

mod support;

use slate_setup::binaries::exe_name;
use slate_setup::env::Harness;
use slate_setup::json::Json;
use std::fs;
use support::{ReleaseServer, Sandbox, fixture, norm, read};

fn json(path: &std::path::Path) -> Json {
    Json::parse(&fs::read_to_string(path).unwrap()).unwrap()
}

fn allow_list(settings: &Json) -> Vec<String> {
    settings
        .get("permissions")
        .and_then(|p| p.get("allow"))
        .and_then(Json::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn install_reinstall_configure_uninstall() {
    let mut sb = Sandbox::new(Harness::Claude);
    let platform = sb.env.platform();
    let server = ReleaseServer::start(&platform, None, true, true);
    sb.use_release(&server);
    let home = sb.home();
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("CLAUDE.md"), "# My own instructions\n").unwrap();
    fs::copy(
        fixture("configs/claude-settings.json"),
        home.join("settings.json"),
    )
    .unwrap();
    fs::create_dir_all(&sb.bin_dir).unwrap();
    fs::write(
        sb.bin_dir.join(exe_name(&platform, "workslate")),
        "old binary",
    )
    .unwrap();
    let db_dir = home.join("projects").join("-work-proj").join("workslate");
    fs::create_dir_all(&db_dir).unwrap();
    fs::write(db_dir.join("workslate.db"), "db").unwrap();
    let roots = sb.dir.path().join("work");
    fs::create_dir_all(&roots).unwrap();
    let roots_arg = roots.to_string_lossy().into_owned();

    let args = [
        "--roots",
        roots_arg.as_str(),
        "--set",
        "aside.level=auto",
        "--set",
        "aside.backend=claude",
        "--set",
        "aside.claude.model=model-c",
        "--set",
        "subagent.model=sonnet",
        "--set",
        "git.signing=no-gpg-sign",
    ];
    let out = sb.run("install", &args);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(out.out.contains("Restart Claude Code"), "{}", out.out);

    // Payload files.
    assert_eq!(
        read(&home.join("CLAUDE.md")),
        read(&sb.payload.join("CLAUDE.md"))
    );
    let backups: Vec<_> = fs::read_dir(&home)
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("CLAUDE.md.bak-")
        })
        .collect();
    assert_eq!(
        backups.len(),
        1,
        "the user's own CLAUDE.md is backed up once"
    );
    assert_eq!(
        fs::read_to_string(backups[0].path()).unwrap(),
        "# My own instructions\n"
    );
    for rule in [
        "claude-agent-kit--task-execution.md",
        "claude-agent-kit--aside.md",
    ] {
        assert_eq!(
            read(&home.join("rules").join(rule)),
            read(&sb.payload.join("rules").join(rule))
        );
    }
    assert!(home.join("skills/palette-init/SKILL.md").is_file());
    assert!(home.join("skills/palette-init/notes/extra.md").is_file());

    // Prefs.
    let aside = read(&sb.prefs_path("aside"));
    assert!(aside.contains("## Level\n\n**auto**\n"), "{aside}");
    assert!(aside.contains("## Backend\n\n**claude**\n"));
    assert!(aside.contains("## Claude model\n\n**model-c**\n"));
    assert!(
        aside.contains("## Codex model\n\n****\n"),
        "only the chosen backend changes"
    );
    assert!(read(&sb.prefs_path("git")).contains("## Commit signing\n\n**no-gpg-sign**\n"));
    assert!(read(&sb.prefs_path("subagent")).contains("## Default model\n\n**sonnet**\n"));

    // Binaries.
    for name in ["aside", "dispatch", "palette"] {
        assert!(
            sb.bin_dir.join(exe_name(&platform, name)).is_file(),
            "{name}"
        );
    }
    assert!(!sb.bin_dir.join(exe_name(&platform, "workslate")).exists());
    assert!(!db_dir.exists());

    // Registration through the CLI.
    let added = sb.added_servers();
    let names: Vec<_> = added.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["aside", "dispatch", "palette"]);
    assert_eq!(added[0].1, ["ASIDE_HARNESS=claude"]);
    assert!(
        added[1]
            .1
            .contains(&format!("SLATE_AGENT_STATE_HOME={}", home.display())),
        "{:?}",
        added[1].1
    );
    assert!(
        added[1]
            .1
            .contains(&format!("DISPATCH_EXTRA_ROOTS={}", roots.display()))
    );
    assert_eq!(
        added[2].1,
        [format!("PALETTE_EXTRA_ROOTS={}", roots.display())]
    );
    assert!(sb.removed_servers().contains(&"workslate".to_string()));
    assert!(
        sb.cli_env("CLAUDE_CONFIG_DIR").is_empty(),
        "the default home needs no CLAUDE_CONFIG_DIR"
    );

    // settings.json: only the intended keys changed, the rest and the formatting stayed.
    let settings_text = fs::read_to_string(home.join("settings.json")).unwrap();
    let settings = Json::parse(&settings_text).unwrap();
    assert_eq!(
        allow_list(&settings),
        [
            "Bash(ls:*)",
            "mcp__palette__palette_status",
            "mcp__palette__palette_read"
        ]
    );
    let env = settings.get("env").unwrap();
    assert_eq!(
        env.get("CLAUDE_CODE_SUBAGENT_MODEL").and_then(Json::as_str),
        Some("sonnet")
    );
    assert_eq!(env.get("KEEP_ME").and_then(Json::as_str), Some("1"));
    assert_eq!(settings.get("model").and_then(Json::as_str), Some("opus"));
    assert!(
        norm(&settings_text).starts_with("{\n    \"model\": \"opus\",\n"),
        "four-space indent is kept"
    );
    let hooks = settings.get("hooks").unwrap();
    assert!(hooks.get("Stop").is_none());
    let pre = hooks.get("PreToolUse").unwrap().as_array().unwrap();
    assert_eq!(pre.len(), 1);
    assert!(
        pre[0].to_compact().contains("my-own-hook") && !pre[0].to_compact().contains("workslate")
    );
    let settings_backups = fs::read_dir(&home)
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("settings.json.bak-")
        })
        .count();
    assert_eq!(settings_backups, 1);

    // Manifest.
    let manifest = fs::read_to_string(home.join(".claude-agent-kit-manifest.toml")).unwrap();
    assert!(manifest.contains("version = \"13.0.0\""), "{manifest}");

    // Reinstall: nothing in the configuration changes again.
    sb.clear_log();
    let out = sb.run("install", &args);
    assert_eq!(out.code, 0, "{}", out.out);
    assert_eq!(
        fs::read_to_string(home.join("settings.json")).unwrap(),
        settings_text
    );
    assert_eq!(
        read(&sb.prefs_path("aside")),
        aside,
        "existing prefs are kept"
    );
    assert_eq!(sb.added_servers().len(), 3);
    let backups_after: usize = fs::read_dir(&home)
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("CLAUDE.md.bak-")
        })
        .count();
    assert_eq!(
        backups_after, 1,
        "a managed CLAUDE.md is replaced without another backup"
    );

    // Configure with a blank subagent model restores the key to absent.
    let out = sb.run("configure", &["--set", "subagent.model="]);
    assert_eq!(out.code, 0, "{}", out.out);
    let env_after = json(&home.join("settings.json"))
        .get("env")
        .unwrap()
        .to_compact();
    assert_eq!(env_after, "{\"KEEP_ME\":\"1\"}");
    assert!(read(&sb.prefs_path("subagent")).contains("## Default model\n\n****\n"));

    // Uninstall keeps the user's files and removes everything else.
    sb.clear_log();
    let out = sb.run("uninstall", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(!home.join("CLAUDE.md").exists());
    assert!(!home.join("rules/claude-agent-kit--aside.md").exists());
    assert!(!home.join("skills/palette-init").exists());
    assert!(
        !home.join("skills").exists(),
        "the install created skills/, so it is removed once empty"
    );
    assert!(
        sb.prefs_path("aside").is_file(),
        "prefs are user-owned and stay"
    );
    assert!(!home.join(".claude-agent-kit-manifest.toml").exists());
    for name in ["aside", "dispatch", "palette"] {
        assert!(
            !sb.bin_dir.join(exe_name(&platform, name)).exists(),
            "{name}"
        );
    }
    let removed = sb.removed_servers();
    for s in ["aside", "dispatch", "palette"] {
        assert!(removed.contains(&s.to_string()), "{s} in {removed:?}");
    }
    assert_eq!(
        allow_list(&json(&home.join("settings.json"))),
        ["Bash(ls:*)"]
    );
    assert!(out.out.contains("files you own"), "{}", out.out);
}

#[test]
fn dry_run_changes_nothing_and_prints_the_summary() {
    let sb = Sandbox::new(Harness::Claude);
    let out = sb.run(
        "install",
        &[
            "--binaries",
            "skip",
            "--dry-run",
            "--set",
            "subagent.model=opus",
        ],
    );
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(
        out.out.contains("Summary") && out.out.contains("Dry run: nothing was changed."),
        "{}",
        out.out
    );
    assert!(
        out.out
            .contains("CLAUDE_CODE_SUBAGENT_MODEL: (absent) -> \"opus\""),
        "{}",
        out.out
    );
    assert!(!sb.home().exists(), "nothing is created");
    assert!(sb.cli_calls().is_empty());
}

#[test]
fn a_custom_home_gets_printed_commands_instead_of_registration() {
    let mut sb = Sandbox::new(Harness::Claude);
    let server = ReleaseServer::start(&sb.env.platform(), None, true, true);
    sb.use_release(&server);
    let custom = sb.dir.path().join("elsewhere");
    let out = sb.run("install", &["--home", custom.to_str().unwrap()]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(custom.join("CLAUDE.md").is_file());
    assert!(
        sb.cli_calls().is_empty(),
        "the CLI is not run for a folder it may not use"
    );
    assert!(
        out.out.contains("Run these commands yourself:"),
        "{}",
        out.out
    );
    let expected = if cfg!(windows) {
        format!("$env:CLAUDE_CONFIG_DIR='{}'; ", custom.display())
    } else {
        format!("CLAUDE_CONFIG_DIR={} ", custom.display())
    };
    assert!(out.out.contains(&expected), "{}", out.out);
    assert!(
        out.out
            .contains("mcp add palette -s user --transport stdio"),
        "{}",
        out.out
    );
    // Permissions and the manifest live in the chosen home regardless.
    assert!(custom.join(".claude-agent-kit-manifest.toml").is_file());

    // The user's own CLAUDE_CONFIG_DIR pointing at that home is different: the CLI already acts on it.
    let mut sb2 = Sandbox::new(Harness::Claude);
    let server2 = ReleaseServer::start(&sb2.env.platform(), None, true, true);
    sb2.use_release(&server2);
    let own = sb2.dir.path().join("own-config");
    sb2.env.set_var("CLAUDE_CONFIG_DIR", own.to_string_lossy());
    let out = sb2.run("install", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(own.join("CLAUDE.md").is_file());
    assert_eq!(sb2.added_servers().len(), 3);
}

#[test]
fn a_failed_reinstall_keeps_what_the_first_install_recorded() {
    let mut sb = Sandbox::new(Harness::Claude);
    let platform = sb.env.platform();
    let good = ReleaseServer::start(&platform, None, true, true);
    sb.use_release(&good);
    let out = sb.run("install", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    let home = sb.home();
    let manifest_path = home.join(".claude-agent-kit-manifest.toml");
    assert!(
        fs::read_to_string(&manifest_path)
            .unwrap()
            .contains("version = \"13.0.0\"")
    );

    // A newer kit release whose binaries fail their checksum.
    let kit_toml = fs::read_to_string(sb.payload.join("kit.toml")).unwrap();
    fs::write(
        sb.payload.join("kit.toml"),
        kit_toml.replace("13.0.0", "13.1.0"),
    )
    .unwrap();
    let bad = ReleaseServer::start(&platform, Some("dispatch"), true, true);
    sb.use_release(&bad);
    let out = sb.run("install", &[]);
    assert_eq!(out.code, 1, "{}", out.out);
    assert!(out.out.contains("checksum mismatch"), "{}", out.out);
    let manifest = fs::read_to_string(&manifest_path).unwrap();
    assert!(
        manifest.contains("version = \"13.0.0\""),
        "the old version stays: {manifest}"
    );
    assert!(
        manifest.contains("CLAUDE.md"),
        "the first install's files are still recorded: {manifest}"
    );

    // Uninstall still removes everything the first install put there.
    sb.use_release(&good);
    let out = sb.run("uninstall", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(!home.join("CLAUDE.md").exists());
    assert!(!home.join("rules/claude-agent-kit--aside.md").exists());
    assert!(!home.join("skills/palette-init").exists());
    assert!(!manifest_path.exists());
}
