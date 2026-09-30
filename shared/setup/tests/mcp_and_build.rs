//! The `mcp` command, `--binaries build`, and binaries shared between kits.

mod support;

use slate_setup::binaries::exe_name;
use slate_setup::env::Harness;
use slate_setup::ui::{Ui, new_sink, sink_text};
use std::fs;
use std::path::Path;
use support::{ReleaseServer, Sandbox, fake_cli, fixture};

fn mcp(sb: &Sandbox, args: &[&str]) -> (i32, String) {
    let mut argv: Vec<String> = vec![
        "slate-setup".into(),
        "mcp".into(),
        "--bin-dir".into(),
        sb.bin_dir.to_string_lossy().into_owned(),
    ];
    argv.extend(args.iter().map(|s| s.to_string()));
    argv.push("--yes".into());
    let sink = new_sink();
    let code = slate_setup::run(
        argv.into_iter().map(Into::into),
        &sb.env,
        Some(Ui::captured(sink.clone())),
    );
    (code, sink_text(&sink))
}

#[test]
fn mcp_installs_binaries_registers_every_harness_and_uninstalls() {
    let mut sb = Sandbox::new(Harness::Claude);
    let platform = sb.env.platform();
    let server = ReleaseServer::start(&platform, None, false, true);
    sb.use_release(&server);
    // A Codex config with comments and a Kimi registry with another plugin.
    let codex_home = sb.env.default_home(Harness::Codex);
    fs::create_dir_all(&codex_home).unwrap();
    fs::copy(
        fixture("configs/codex-config.toml"),
        codex_home.join("config.toml"),
    )
    .unwrap();
    let original: toml::Table =
        toml::from_str(&fs::read_to_string(codex_home.join("config.toml")).unwrap()).unwrap();
    let roots = sb.dir.path().join("work");
    fs::create_dir_all(&roots).unwrap();
    let roots_arg = roots.to_string_lossy().into_owned();

    let (code, out) = mcp(
        &sb,
        &[
            "--harness",
            "claude,codex,kimi",
            "--roots",
            roots_arg.as_str(),
        ],
    );
    assert_eq!(code, 0, "{out}");
    for name in ["aside", "dispatch", "palette"] {
        assert!(
            sb.bin_dir.join(exe_name(&platform, name)).is_file(),
            "{name}"
        );
    }
    let claude_home = sb.env.default_home(Harness::Claude);
    let settings = fs::read_to_string(claude_home.join("settings.json")).unwrap();
    assert!(
        settings.contains("mcp__palette__palette_status"),
        "{settings}"
    );
    let cfg: toml::Table =
        toml::from_str(&fs::read_to_string(codex_home.join("config.toml")).unwrap()).unwrap();
    assert_eq!(
        cfg["mcp_servers"]["aside"]["tool_timeout_sec"].as_integer(),
        Some(1800)
    );
    assert!(
        sb.env
            .default_home(Harness::Kimi)
            .join("plugins/managed/slate-agent-kit-mcp/kimi.plugin.json")
            .is_file()
    );
    assert_eq!(
        sb.added_servers().len(),
        6,
        "three servers for Claude and three for Codex"
    );
    // The plugin version is the installer's own when there is no kit.
    assert!(
        claude_home
            .join(".slate-agent-kit-mcp-manifest.toml")
            .is_file()
    );

    let (code, out) = mcp(
        &sb,
        &[
            "--uninstall",
            "--harness",
            "claude",
            "--harness",
            "codex",
            "--harness",
            "kimi",
        ],
    );
    assert_eq!(code, 0, "{out}");
    let removed = sb.removed_servers();
    assert!(removed.iter().filter(|s| s.as_str() == "palette").count() >= 2);
    assert!(!settings_has_palette(&claude_home.join("settings.json")));
    let cfg_after: toml::Table =
        toml::from_str(&fs::read_to_string(codex_home.join("config.toml")).unwrap()).unwrap();
    assert_eq!(cfg_after, original);
    assert!(
        !sb.env
            .default_home(Harness::Kimi)
            .join("plugins/managed/slate-agent-kit-mcp")
            .exists()
    );
    assert!(
        !claude_home
            .join(".slate-agent-kit-mcp-manifest.toml")
            .exists()
    );
    assert!(
        sb.bin_dir.join(exe_name(&platform, "aside")).exists(),
        "mcp --uninstall does not remove binaries"
    );
}

fn settings_has_palette(path: &Path) -> bool {
    fs::read_to_string(path)
        .map(|t| t.contains("mcp__palette__"))
        .unwrap_or(false)
}

#[test]
fn mcp_install_only_and_a_missing_cli_is_an_error() {
    let mut sb = Sandbox::new(Harness::Claude);
    let server = ReleaseServer::start(&sb.env.platform(), None, false, true);
    sb.use_release(&server);
    let (code, out) = mcp(&sb, &[]);
    assert_eq!(code, 0, "{out}");
    assert!(
        sb.bin_dir
            .join(exe_name(&sb.env.platform(), "dispatch"))
            .is_file()
    );
    assert!(sb.cli_calls().is_empty());

    sb.env.set_var("CODEX_BIN", "/definitely/not/there");
    let (code, out) = mcp(&sb, &["--harness", "codex", "--binaries", "skip"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("could not be registered with Codex"), "{out}");
    assert!(
        out.contains("CODEX_HOME="),
        "the commands to run by hand are printed: {out}"
    );
}

#[test]
fn mcp_usage_errors() {
    let sb = Sandbox::new(Harness::Claude);
    let (code, out) = mcp(&sb, &["--uninstall"]);
    assert_eq!(code, 2, "{out}");
    let (code, out) = mcp(&sb, &["--harness", "claude,codex", "--home", "/x"]);
    assert_eq!(code, 2, "{out}");
    let (code, out) = mcp(&sb, &["--binaries", "build"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("--slate-dir"), "{out}");
}

#[test]
fn build_mode_runs_cargo_in_the_checkout_and_installs_what_it_built() {
    let mut sb = Sandbox::new(Harness::Codex);
    let platform = sb.env.platform();
    let slate = sb.dir.path().join("slate-checkout");
    fs::create_dir_all(&slate).unwrap();
    // The stand-in cargo writes target/release/<package> relative to the working directory.
    sb.env.set_var("CARGO_BIN", fake_cli().to_string_lossy());
    sb.env
        .set_var("CARGO_TARGET_DIR", slate.join("target").to_string_lossy());
    let out = sb.run(
        "install",
        &[
            "--binaries",
            "build",
            "--slate-dir",
            slate.to_str().unwrap(),
        ],
    );
    assert_eq!(out.code, 0, "{}", out.out);
    let builds: Vec<Vec<String>> = sb
        .cli_calls()
        .into_iter()
        .filter(|c| c[0] == "build")
        .collect();
    assert_eq!(builds.len(), 1);
    assert_eq!(
        builds[0],
        [
            "build",
            "--release",
            "-p",
            "aside",
            "-p",
            "dispatch",
            "-p",
            "palette"
        ]
    );
    for name in ["aside", "dispatch", "palette"] {
        assert!(
            sb.bin_dir.join(exe_name(&platform, name)).is_file(),
            "{name}"
        );
    }
    assert!(!sb.env.platform().is_empty());
}

#[test]
fn build_mode_without_a_checkout_fails_before_changing_anything() {
    let sb = Sandbox::new(Harness::Codex);
    let out = sb.run("install", &["--binaries", "build"]);
    assert_eq!(out.code, 2, "{}", out.out);
    assert!(out.out.contains("--slate-dir"), "{}", out.out);
    assert!(!sb.home().exists());
}

#[test]
fn a_binary_shared_with_another_kit_stays_until_the_last_kit_is_gone() {
    let mut claude = Sandbox::new(Harness::Claude);
    let platform = claude.env.platform();
    let server = ReleaseServer::start(&platform, None, true, true);
    claude.use_release(&server);
    let out = claude.run("install", &[]);
    assert_eq!(out.code, 0, "{}", out.out);

    // A second kit in another harness home of the same user, sharing the binary folder.
    let codex = {
        let mut sb = Sandbox::new(Harness::Codex);
        sb.env = claude.env.clone();
        sb.payload = claude.dir.path().join("dist-codex");
        copy_tree(&fixture("payload-codex"), &sb.payload);
        sb.bin_dir = claude.bin_dir.clone();
        sb.log = claude.log.clone();
        sb.dir = tempfile::tempdir().unwrap();
        sb
    };
    let out = codex.run("install", &[]);
    assert_eq!(out.code, 0, "{}", out.out);

    let out = claude.run("uninstall", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    for name in ["aside", "dispatch", "palette"] {
        assert!(
            claude.bin_dir.join(exe_name(&platform, name)).exists(),
            "{name} is still listed by the Codex kit"
        );
    }
    assert!(out.out.contains("also lists it"), "{}", out.out);

    let out = codex.run("uninstall", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    for name in ["aside", "dispatch", "palette"] {
        assert!(
            !claude.bin_dir.join(exe_name(&platform, name)).exists(),
            "{name}"
        );
    }
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap().flatten() {
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &dest);
        } else {
            fs::copy(e.path(), dest).unwrap();
        }
    }
}

#[test]
fn configure_migrates_old_prefs_after_confirmation() {
    let sb = Sandbox::new(Harness::Claude);
    let home = sb.home();
    let kit = sb.kit();
    fs::create_dir_all(home.join("rules")).unwrap();
    let old = fixture(&format!("old-prefs-claude/{kit}--aside-prefs.md"));
    let dest = sb.prefs_path("aside");
    fs::copy(old, &dest).unwrap();
    let before = fs::read_to_string(&dest).unwrap();
    // Declining keeps the old file and says so.
    let declined = sb.run_scripted(
        "configure",
        &[
            "--binaries",
            "skip",
            "--roots",
            sb.dir.path().to_str().unwrap(),
        ],
        &format!("n\n{}", "\n".repeat(30)),
    );
    assert_eq!(declined.code, 0, "{}", declined.out);
    assert_eq!(fs::read_to_string(&dest).unwrap(), before);
    assert!(declined.out.contains("keep old layout"), "{}", declined.out);
    // Accepting migrates it (y, then no review, then defaults).
    let accepted = sb.run_scripted(
        "configure",
        &[
            "--binaries",
            "skip",
            "--roots",
            sb.dir.path().to_str().unwrap(),
        ],
        &format!("y\nn\n{}", "\n".repeat(30)),
    );
    assert_eq!(accepted.code, 0, "{}", accepted.out);
    let after = support::read(&dest);
    assert!(after.contains("## Level\n\n**auto**\n"), "{after}");
    assert!(
        fs::read_dir(home.join("rules"))
            .unwrap()
            .flatten()
            .any(|e| e
                .file_name()
                .to_string_lossy()
                .contains("aside-prefs.md.bak-"))
    );
}
