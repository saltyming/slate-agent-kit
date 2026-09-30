//! Behaviour that holds for every harness: upgrades from the earlier layout, checksum
//! verification, ownership rules, the interactive questions and the real executable.

mod support;

use slate_setup::binaries::exe_name;
use slate_setup::env::Harness;
use std::fs;
use support::{ReleaseServer, Sandbox, fixture, norm, read};

fn install_old_state(sb: &Sandbox) {
    let home = sb.home();
    let kit = sb.kit();
    let rules = home.join("rules");
    fs::create_dir_all(&rules).unwrap();
    let old = format!("old-prefs-{}", sb.harness.name());
    let mut listed = Vec::new();
    for name in ["aside", "dispatch", "git", "comment"] {
        let file = format!("{kit}--{name}-prefs.md");
        fs::copy(fixture(&format!("{old}/{file}")), rules.join(&file)).unwrap();
        listed.push(rules.join(&file));
    }
    // A rule the earlier release shipped and this one does not.
    let stale = rules.join(format!("{kit}--delegation.md"));
    fs::write(
        &stale,
        "<!-- slate-agent-kit:common -->\n# Old delegation rule\n",
    )
    .unwrap();
    listed.push(stale);
    let primary = home.join(sb.harness.primary_file());
    fs::write(&primary, "<!-- slate-agent-kit:common -->\n# Old manual\n").unwrap();
    listed.push(primary);
    let mut manifest = String::from("## install @ 2026-09-19T10:00:00Z\n");
    for p in &listed {
        manifest.push_str(&format!("{}\n", p.display()));
    }
    let manifest_name = match sb.harness {
        Harness::Kimi => ".kimi-code-agent-kit-manifest".to_string(),
        _ => format!(".{kit}-manifest"),
    };
    fs::write(home.join(manifest_name), manifest).unwrap();
}

#[test]
fn upgrading_from_the_earlier_layout_migrates_prefs_and_replaces_the_line_manifest() {
    for harness in [Harness::Claude, Harness::Codex, Harness::Kimi] {
        let mut sb = Sandbox::new(harness);
        let server = ReleaseServer::start(&sb.env.platform(), None, true, true);
        sb.use_release(&server);
        install_old_state(&sb);
        let home = sb.home();
        let kit = sb.kit();
        let roots = sb.dir.path().to_string_lossy().into_owned();
        let out = sb.run("install", &["--roots", roots.as_str()]);
        assert_eq!(out.code, 0, "{harness:?}: {}", out.out);

        // aside: proactive + claude advisor become level auto and backend claude; values carry over.
        let aside = read(&sb.prefs_path("aside"));
        assert!(
            aside.contains("## Level\n\n**auto**\n"),
            "{harness:?}: {aside}"
        );
        assert!(aside.contains("## Backend\n\n**claude**\n"));
        assert!(aside.contains("## Codex model\n\n**model-a**\n"));
        assert!(aside.contains("## Codex reasoning effort\n\n**high**\n"));
        assert!(
            aside.contains("## Codex model fallback\n\n**model-b(high)**\n"),
            "an unbolded fallback line survives: {aside}"
        );
        assert!(aside.contains("## Claude model fallback\n\n**model-c2**\n"));
        assert!(aside.contains("## Claude model\n\n**model-c**\n"));
        assert!(aside.contains("## Claude reasoning effort\n\n**max**\n"));
        assert!(aside.contains("My own aside note, kept verbatim."));
        // dispatch: proactive + auto becomes auto; granularity is dropped.
        let dispatch = read(&sb.prefs_path("dispatch"));
        assert!(dispatch.contains("## Level\n\n**auto**\n"), "{dispatch}");
        assert!(dispatch.contains("## Backend\n\n**opencode**\n"));
        assert!(dispatch.contains("## Model\n\n**model-d**\n"));
        assert!(
            dispatch.contains("## Model fallback\n\n**model-e, model-f**\n"),
            "{dispatch}"
        );
        assert!(!dispatch.to_lowercase().contains("granularity"));
        assert!(dispatch.contains("My own dispatch note."));
        // git and comment carry over unchanged, with Repository overrides and Notes kept.
        let git = read(&sb.prefs_path("git"));
        assert!(git.contains("## Commit signing\n\n**no-gpg-sign**\n"));
        assert!(git.contains("- /work/repo-a: follow the repository log"));
        assert!(git.contains("My own git note."));
        assert!(
            !git.contains("Old wording"),
            "the new template's text replaces the old free text"
        );
        assert!(read(&sb.prefs_path("comment")).contains("## File headers\n\n**structured**\n"));
        // subagent is new, from the template.
        assert!(read(&sb.prefs_path("subagent")).contains("## Level\n\n**suggest**\n"));
        // Every migrated file has a backup of the old file.
        let backups: Vec<String> = fs::read_dir(home.join("rules"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("-prefs.md.bak-"))
            .collect();
        assert_eq!(backups.len(), 4, "{harness:?}: {backups:?}");
        let aside_backup = backups.iter().find(|b| b.contains("aside")).unwrap();
        assert!(read(&home.join("rules").join(aside_backup)).contains("Auto-call policy"));

        // The stale rule is removed, the line manifest replaced by the TOML one.
        assert!(
            !home
                .join("rules")
                .join(format!("{kit}--delegation.md"))
                .exists()
        );
        assert!(home.join(format!(".{kit}-manifest.toml")).is_file());
        assert!(!home.join(format!(".{kit}-manifest")).exists());
        assert!(!home.join(".kimi-code-agent-kit-manifest").exists());
        assert!(!read(&home.join(harness.primary_file())).contains("Old manual"));
    }
}

#[test]
fn migrating_old_values_of_a_backend_the_kit_lacks_warns_once_and_keeps_unknown_sections() {
    for filled in [true, false] {
        let mut sb = Sandbox::new(Harness::Claude);
        let server = ReleaseServer::start(&sb.env.platform(), None, true, true);
        sb.use_release(&server);
        install_old_state(&sb);
        let path = sb.prefs_path("aside");
        let mut old = read(&path);
        if filled {
            old = old.replace(
                "- legacy default model: ****",
                "- legacy default model: **legacy-model**",
            );
        }
        old.push_str("\n## My own section\n\nfirst line\nsecond line\n");
        fs::write(&path, &old).unwrap();
        let roots = sb.dir.path().to_string_lossy().into_owned();
        let out = sb.run("install", &["--roots", roots.as_str()]);
        assert_eq!(out.code, 0, "{}", out.out);
        let lines = legacy_lines(&out.out);
        if filled {
            assert_eq!(lines.len(), 1, "{}", out.out);
            assert!(
                lines[0].contains("`legacy`") && lines[0].contains("backup"),
                "{}",
                lines[0]
            );
        } else {
            assert!(lines.is_empty(), "{}", out.out);
        }
        let aside = read(&path);
        assert!(
            aside.ends_with("## My own section\n\nfirst line\nsecond line\n"),
            "{aside}"
        );
        assert!(!aside.contains("legacy-model"));
    }
}

#[test]
fn a_checksum_mismatch_aborts_and_installs_nothing() {
    let mut sb = Sandbox::new(Harness::Claude);
    let platform = sb.env.platform();
    let server = ReleaseServer::start(&platform, Some("dispatch"), true, true);
    sb.use_release(&server);
    let out = sb.run("install", &[]);
    assert_eq!(out.code, 1, "{}", out.out);
    assert!(
        out.out.contains("checksum mismatch for dispatch-"),
        "{}",
        out.out
    );
    assert!(out.out.contains("no binary was replaced"), "{}", out.out);
    assert!(out.out.contains("fix:"), "{}", out.out);
    assert!(!sb.bin_dir.join(exe_name(&platform, "aside")).exists());
    assert!(sb.added_servers().is_empty(), "nothing is registered");
    assert!(
        !sb.home().join("CLAUDE.md").exists(),
        "the run stops before any file is written"
    );
    assert!(!sb.home().join(".claude-agent-kit-manifest.toml").exists());
}

#[test]
fn a_missing_versioned_release_falls_back_to_the_latest_and_says_so() {
    let mut sb = Sandbox::new(Harness::Claude);
    let server = ReleaseServer::start(&sb.env.platform(), None, false, true);
    sb.use_release(&server);
    let out = sb.run(
        "install",
        &["--home", sb.dir.path().join("h").to_str().unwrap()],
    );
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(
        out.out.contains("slate release v0.7.0 does not exist"),
        "{}",
        out.out
    );
    let requests = server.requests.lock().unwrap().clone();
    assert!(
        requests
            .iter()
            .any(|r| r.contains("/download/v0.7.0/checksums.txt"))
    );
    assert!(
        requests
            .iter()
            .any(|r| r.contains("/latest/download/aside-"))
    );
}

#[test]
fn a_missing_checksum_file_warns_and_still_installs() {
    let mut sb = Sandbox::new(Harness::Codex);
    let server = ReleaseServer::start(&sb.env.platform(), None, true, false);
    sb.use_release(&server);
    let out = sb.run("install", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(out.out.contains("no checksums.txt"), "{}", out.out);
    assert!(
        sb.bin_dir
            .join(exe_name(&sb.env.platform(), "aside"))
            .is_file()
    );
}

#[test]
fn user_owned_files_are_never_overwritten() {
    let mut sb = Sandbox::new(Harness::Claude);
    let server = ReleaseServer::start(&sb.env.platform(), None, true, true);
    sb.use_release(&server);
    let home = sb.home();
    let kit = sb.kit();
    fs::create_dir_all(home.join("rules")).unwrap();
    fs::create_dir_all(home.join("skills/palette-init")).unwrap();
    // The user's own prefs, a rule of their own at a kit rule name, and their own skill.
    let prefs = sb.prefs_path("git");
    let mine = format!(
        "<!-- {kit}-custom:git-prefs -->\n# My git prefs\n\n## Commit signing\n\n**default**\n\n## Model attribution\n\n**on**\n\n## Commit message format\n\n**repository**\n\n## PR body format\n\n**repository**\n\n## Branch naming\n\n**repository**\n"
    );
    fs::write(&prefs, &mine).unwrap();
    let rule = home.join("rules").join(format!("{kit}--aside.md"));
    fs::write(&rule, format!("<!-- {kit}-custom:mine -->\nMy rule\n")).unwrap();
    fs::write(
        home.join("skills/palette-init/SKILL.md"),
        "# my own skill\n",
    )
    .unwrap();
    // A custom rule that collides with a kit-managed name and one that collides with a prefs file.
    let custom = sb.dir.path().join("mine");
    fs::create_dir_all(&custom).unwrap();
    fs::write(
        custom.join("task-execution.md"),
        "# collides with the kit rule\n",
    )
    .unwrap();
    fs::write(custom.join("git-prefs.md"), "# collides with prefs\n").unwrap();
    fs::write(custom.join("ok.md"), "# fine\n").unwrap();
    let custom_arg = custom.to_string_lossy().into_owned();

    let out = sb.run(
        "install",
        &[
            "--custom-rules",
            custom_arg.as_str(),
            "--set",
            "aside.level=auto",
        ],
    );
    assert_eq!(out.code, 0, "{}", out.out);
    assert_eq!(
        fs::read_to_string(&prefs).unwrap(),
        mine,
        "existing prefs are kept as they are"
    );
    assert!(fs::read_to_string(&rule).unwrap().contains("My rule"));
    assert_eq!(
        fs::read_to_string(home.join("skills/palette-init/SKILL.md")).unwrap(),
        "# my own skill\n"
    );
    assert!(
        out.out.contains("a file you own already has this name"),
        "{}",
        out.out
    );
    assert!(
        out.out.contains("a skill you own already has this name"),
        "{}",
        out.out
    );
    assert!(
        out.out.contains("it would replace a kit-managed file"),
        "{}",
        out.out
    );
    assert!(out.out.contains("reserved for a prefs file"), "{}", out.out);
    assert!(
        read(&home.join("rules").join(format!("{kit}--task-execution.md")))
            .contains("Fixture rule text.")
    );
    assert!(home.join("rules").join(format!("{kit}--ok.md")).is_file());
    // A value given with --set is applied to the file that exists, and only that value.
    let after = read(&sb.prefs_path("aside"));
    assert!(after.contains("## Level\n\n**auto**\n"));

    // Uninstall keeps every user-owned file unless asked; --yes never removes them.
    let out = sb.run("uninstall", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert_eq!(fs::read_to_string(&prefs).unwrap(), mine);
    assert!(fs::read_to_string(&rule).unwrap().contains("My rule"));
    assert!(home.join("rules").join(format!("{kit}--ok.md")).is_file());
    assert!(home.join("skills/palette-init/SKILL.md").is_file());
}

/// An aside prefs file in the current layout whose backend and three settings belong to a
/// backend the schema does not have.
fn aside_prefs_with_unknown_backend(sb: &Sandbox) -> String {
    let template = read(&fixture(&format!(
        "payload-{}/prefs/aside-prefs.md",
        sb.harness.name()
    )));
    let mine = template
        .replace("## Backend\n\n**codex**\n", "## Backend\n\n**legacy**\n")
        .replace(
            "## Claude model\n",
            "## Legacy model\n\n**legacy-1**\n\n## Legacy reasoning effort\n\n**high**\n\n## Legacy model fallback\n\n**legacy-2**\n\n## Claude model\n",
        );
    assert!(mine.contains("**legacy**") && mine.contains("## Legacy model\n"));
    fs::create_dir_all(sb.prefs_path("aside").parent().unwrap()).unwrap();
    fs::write(sb.prefs_path("aside"), &mine).unwrap();
    mine
}

/// The distinct warning lines that mention the unknown backend; the plan and the
/// final report each show every warning, so one warning is one distinct line.
fn legacy_lines(out: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = out
        .lines()
        .filter(|l| l.contains("warning") && l.to_lowercase().contains("legacy"))
        .map(str::trim)
        .collect();
    lines.sort_unstable();
    lines.dedup();
    lines
}

#[test]
fn settings_of_a_backend_the_kit_lacks_are_reported_in_one_line_and_left_in_place() {
    for cmd in ["install", "configure"] {
        let sb = Sandbox::new(Harness::Codex);
        let roots = sb.dir.path().to_string_lossy().into_owned();
        if cmd == "configure" {
            let out = sb.run(
                "install",
                &["--binaries", "skip", "--roots", roots.as_str()],
            );
            assert_eq!(out.code, 0, "{}", out.out);
        }
        let mine = aside_prefs_with_unknown_backend(&sb);
        let out = sb.run(cmd, &["--binaries", "skip", "--roots", roots.as_str()]);
        assert_eq!(out.code, 0, "{cmd}: {}", out.out);
        let lines = legacy_lines(&out.out);
        assert_eq!(lines.len(), 1, "{cmd}: {}", out.out);
        for part in [
            "aside",
            "`legacy`",
            "Legacy model",
            "Legacy reasoning effort",
            "Legacy model fallback",
        ] {
            assert!(lines[0].contains(part), "{cmd}: {}", lines[0]);
        }
        assert_eq!(
            read(&sb.prefs_path("aside")),
            mine,
            "{cmd}: the user's file is kept byte for byte"
        );
    }
}

#[test]
fn a_backend_the_kit_lacks_counts_as_unset_when_the_values_are_changed() {
    let sb = Sandbox::new(Harness::Codex);
    let roots = sb.dir.path().to_string_lossy().into_owned();
    aside_prefs_with_unknown_backend(&sb);
    // Change the aside values, press Enter at level, backend and the three codex
    // questions, then take the defaults for everything after that.
    let out = sb.run_scripted(
        "install",
        &["--binaries", "skip", "--roots", roots.as_str()],
        "y\n\n\n\n\n\n",
    );
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(out.out.contains("Choice [codex]"), "{}", out.out);
    assert!(out.out.contains("codex model"), "{}", out.out);
    assert!(!out.out.contains("legacy model (blank"), "{}", out.out);
    let aside = read(&sb.prefs_path("aside"));
    assert!(aside.contains("## Backend\n\n**codex**\n"), "{aside}");
    assert!(
        aside.contains("## Legacy model\n\n**legacy-1**\n"),
        "the other backend's lines stay: {aside}"
    );
    let lines = legacy_lines(&out.out);
    assert_eq!(lines.len(), 1, "{}", out.out);
    assert!(!lines[0].contains("`legacy`"), "{}", lines[0]);
    assert!(
        out.out.contains("aside.backend: legacy -> codex"),
        "{}",
        out.out
    );
}

#[test]
fn uninstall_removes_user_owned_files_only_when_the_user_says_so() {
    let mut sb = Sandbox::new(Harness::Codex);
    let server = ReleaseServer::start(&sb.env.platform(), None, true, true);
    sb.use_release(&server);
    let out = sb.run("install", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    // Interactive: the first question lists the user's files; answer yes, then confirm.
    let out = sb.run_scripted("uninstall", &[], "y\ny\n");
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(!sb.prefs_path("aside").exists());
    assert!(!sb.prefs_path("git").exists());
    // Answering no keeps them.
    let mut sb = Sandbox::new(Harness::Codex);
    let server = ReleaseServer::start(&sb.env.platform(), None, true, true);
    sb.use_release(&server);
    sb.run("install", &[]);
    let out = sb.run_scripted("uninstall", &[], "n\ny\n");
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(sb.prefs_path("aside").is_file());
}

#[test]
fn the_scripted_wizard_asks_conditional_validated_questions_and_confirms() {
    let sb = Sandbox::new(Harness::Codex);
    let roots = sb.dir.path().to_string_lossy().into_owned();
    // Questions in order (all Enter unless noted):
    //  aside: level 3 (auto); backend "nope" (rejected) then 2 (claude);
    //         claude model "cm", effort 9 (rejected) then 3 (high), fallback "a, b"
    //  dispatch: level, backend, model, effort, fallback (defaults)
    //  subagent: level, model "gpt-x", effort 5 (max)
    //  git: signing 2, attribution 2, commit-format, pr-body, branch-naming
    //  comment: headers 2, language, doc-comments
    //  custom rules folder: Enter; confirmation: Enter
    let answers = [
        "3", "nope", "2", "cm", "9", "3", "a, b", "", "", "", "", "", "", "gpt-x", "5", "2", "2",
        "", "", "", "2", "", "", "", "",
    ]
    .join("\n")
        + "\n";
    let out = sb.run_scripted(
        "install",
        &["--binaries", "skip", "--roots", roots.as_str()],
        &answers,
    );
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(
        out.out.contains("[1/6]") && out.out.contains("[6/6]"),
        "{}",
        out.out
    );
    assert!(
        out.out.contains("Not valid: `nope`") && out.out.contains("codex, claude"),
        "{}",
        out.out
    );
    assert!(out.out.contains("Not valid: `9`"), "{}", out.out);
    let aside = read(&sb.prefs_path("aside"));
    assert!(aside.contains("## Level\n\n**auto**\n"), "{aside}");
    assert!(aside.contains("## Backend\n\n**claude**\n"));
    assert!(aside.contains("## Claude model\n\n**cm**\n"));
    assert!(aside.contains("## Claude reasoning effort\n\n**high**\n"));
    assert!(aside.contains("## Claude model fallback\n\n**a, b**\n"));
    assert!(
        !out.out.contains("codex model"),
        "questions for backends that were not chosen are not asked"
    );
    let subagent = read(&sb.prefs_path("subagent"));
    assert!(
        subagent.contains("## Default model\n\n**gpt-x**\n")
            && subagent.contains("## Reasoning effort\n\n**max**\n"),
        "{subagent}"
    );
    let cfg: toml::Table =
        toml::from_str(&fs::read_to_string(sb.home().join("config.toml")).unwrap()).unwrap();
    assert_eq!(
        cfg["agents"]["default_subagent_model"].as_str(),
        Some("gpt-x")
    );
    assert_eq!(
        cfg["agents"]["default_subagent_reasoning_effort"].as_str(),
        Some("max")
    );
    let git = read(&sb.prefs_path("git"));
    assert!(
        git.contains("## Commit signing\n\n**no-gpg-sign**\n")
            && git.contains("## Model attribution\n\n**off**\n")
    );
    assert!(read(&sb.prefs_path("comment")).contains("## File headers\n\n**structured**\n"));
}

#[test]
fn the_same_answers_produce_the_same_prefs_in_every_harness() {
    let mut files: Vec<Vec<String>> = Vec::new();
    for harness in [Harness::Claude, Harness::Codex, Harness::Kimi] {
        let sb = Sandbox::new(harness);
        let out = sb.run(
            "install",
            &[
                "--binaries",
                "skip",
                "--set",
                "aside.level=auto",
                "--set",
                "aside.backend=claude",
                "--set",
                "dispatch.backend=opencode",
                "--set",
                "git.attribution=off",
                "--set",
                "comment.language=korean",
            ],
        );
        assert_eq!(out.code, 0, "{harness:?}: {}", out.out);
        let kit = sb.kit();
        files.push(
            ["aside", "dispatch", "subagent", "git", "comment"]
                .iter()
                .map(|n| norm(&read(&sb.prefs_path(n))).replace(&kit, "KIT"))
                .collect(),
        );
    }
    assert_eq!(files[0], files[1]);
    assert_eq!(files[1], files[2]);
}

#[test]
fn a_real_process_run_resolves_home_and_defaults_from_its_environment() {
    let sb = Sandbox::new(Harness::Claude);
    let exe = env!("CARGO_BIN_EXE_slate-setup");
    let run = |args: &[&str]| {
        let mut cmd = std::process::Command::new(exe);
        cmd.env_clear()
            .env("HOME", &sb.user_home)
            .env("USERPROFILE", &sb.user_home)
            .env("CLAUDE_BIN", support::fake_cli())
            .env("FAKE_CLI_LOG", &sb.log)
            .env("NO_COLOR", "1")
            .args(["install", "--payload"])
            .arg(&sb.payload)
            .args(args)
            .stdin(std::process::Stdio::null());
        if let Some(root) = std::env::var_os("SystemRoot") {
            cmd.env("SystemRoot", root);
        }
        cmd.output().unwrap()
    };
    let dry = run(&["--binaries", "skip", "--dry-run"]);
    let text = String::from_utf8_lossy(&dry.stdout).into_owned();
    assert!(
        dry.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&dry.stderr)
    );
    assert!(text.contains("Dry run: nothing was changed."), "{text}");
    assert!(!text.contains('\u{1b}'), "no colour without a terminal");
    assert!(!sb.home().exists());
    let real = run(&["--binaries", "skip", "--yes"]);
    assert!(
        real.status.success(),
        "{}{}",
        String::from_utf8_lossy(&real.stdout),
        String::from_utf8_lossy(&real.stderr)
    );
    assert!(sb.home().join("CLAUDE.md").is_file());
    let bad = run(&["--set", "aside.level=sometimes", "--yes"]);
    assert_eq!(bad.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&bad.stdout).contains("on-request, suggest, auto")
            || String::from_utf8_lossy(&bad.stderr).contains("on-request, suggest, auto")
    );
}

#[test]
fn a_failing_registration_names_what_failed_the_state_and_the_fix() {
    let mut sb = Sandbox::new(Harness::Codex);
    let server = ReleaseServer::start(&sb.env.platform(), None, true, true);
    sb.use_release(&server);
    sb.env
        .child_env
        .insert("FAKE_CLI_FAIL".into(), "mcp add dispatch".into());
    let out = sb.run("install", &[]);
    assert_eq!(out.code, 1, "{}", out.out);
    assert!(
        out.out.contains("`codex mcp add dispatch` failed"),
        "{}",
        out.out
    );
    assert!(
        out.out.contains("registered before the failure: aside"),
        "{}",
        out.out
    );
    assert!(out.out.contains("fix:"), "{}", out.out);
    // What was done before the failure is in the manifest, so uninstall can reverse it.
    let manifest = fs::read_to_string(sb.home().join(".codex-agent-kit-manifest.toml")).unwrap();
    assert!(manifest.contains("server = \"aside\""), "{manifest}");
    assert!(!manifest.contains("server = \"dispatch\""), "{manifest}");
    assert!(sb.home().join("AGENTS.md").is_file());
}

/// The installer's heading table must match the templates the kits actually ship.
#[test]
fn every_setting_of_the_real_prefs_templates_is_found_and_its_default_is_valid() {
    use slate_setup::prefs::{Input, State, doc::PrefsDoc, plan_file, schema};
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("prefs");
    if !dir.is_dir() {
        eprintln!("skipped: {} does not exist", dir.display());
        return;
    }
    for name in ["aside", "dispatch", "subagent", "git", "comment"] {
        let template = fs::read_to_string(dir.join(format!("{name}-prefs.md.tmpl"))).unwrap();
        let rendered = template.replace("{{KIT_PREFIX}}", "some-kit");
        let doc = PrefsDoc::parse(&rendered);
        for harness in [Harness::Claude, Harness::Codex, Harness::Kimi] {
            let mut ctx = schema::ValidationCtx::new(harness);
            ctx.kimi_models = Some(vec!["alias".into()]);
            for setting in schema::for_file(name) {
                let value = doc.get(setting.heading).unwrap_or_else(|| {
                    panic!(
                        "{name}-prefs.md.tmpl has no value line under `## {}`",
                        setting.heading
                    )
                });
                // The default a fresh install writes must pass the installer's own validation,
                // except a Kimi model, which has to name an alias of the user's config.
                if setting.key == "model" && name == "subagent" && harness == Harness::Kimi {
                    continue;
                }
                if let Err(e) = schema::validate(setting, &ctx, &value) {
                    panic!(
                        "{name}.{} default `{value}` is invalid for {harness:?}: {e}",
                        setting.key
                    );
                }
            }
            let plan = plan_file(
                name,
                None,
                &rendered,
                State::Missing,
                &Input::default(),
                &ctx,
            )
            .unwrap();
            assert_eq!(
                plan.text.as_deref(),
                Some(rendered.as_str()),
                "{name}: an untouched fresh file is the template"
            );
        }
    }
}
