//! `agent-guard`, the helper the servers start beside themselves on Linux and macOS.
//!
//! Covers its placement in the binary folder by `install` and `mcp` in `prebuilt`
//! and `build` mode, its manifest entry, replacement, removal and absence from
//! every registration, and the release that does not ship it.

mod support;

use slate_setup::binaries::exe_name;
use slate_setup::env::Harness;
use slate_setup::ui::{Ui, new_sink, sink_text};
use std::fs;
use support::{ReleaseServer, Sandbox, fake_cli};

const GUARD: &str = "agent-guard";

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

fn guard_path(sb: &Sandbox) -> std::path::PathBuf {
    sb.bin_dir.join(exe_name(&sb.env.platform(), GUARD))
}

fn manifest_text(sb: &Sandbox) -> String {
    fs::read_to_string(sb.home().join(format!(".{}-manifest.toml", sb.kit()))).unwrap()
}

fn warning_lines(out: &str) -> usize {
    out.lines()
        .filter(|l| l.contains("has no agent-guard"))
        .count()
}

/// Points the installer at a checkout whose stand-in cargo writes the built binaries.
fn use_fake_checkout(sb: &mut Sandbox) -> std::path::PathBuf {
    let slate = sb.dir.path().join("slate-checkout");
    fs::create_dir_all(&slate).unwrap();
    sb.env.set_var("CARGO_BIN", fake_cli().to_string_lossy());
    sb.env
        .set_var("CARGO_TARGET_DIR", slate.join("target").to_string_lossy());
    slate
}

#[test]
fn a_prebuilt_install_places_records_and_removes_the_guard_on_unix() {
    let mut sb = Sandbox::new(Harness::Claude);
    let platform = sb.env.platform();
    let server = ReleaseServer::start(&platform, None, true, true);
    sb.use_release(&server);
    let out = sb.run("install", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert_eq!(guard_path(&sb).is_file(), !cfg!(windows), "{}", out.out);
    assert_eq!(manifest_text(&sb).contains(GUARD), !cfg!(windows));
    assert_eq!(warning_lines(&out.out), 0, "{}", out.out);
    let names: Vec<String> = sb.added_servers().into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["aside", "dispatch", "palette"], "not registered");
    let requests = server.requests.lock().unwrap().clone();
    assert_eq!(
        requests.iter().any(|r| r.contains(GUARD)),
        !cfg!(windows),
        "{requests:?}"
    );

    let out = sb.run("uninstall", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(!guard_path(&sb).exists(), "{}", out.out);
}

#[test]
fn an_upgrade_replaces_the_guard() {
    let mut sb = Sandbox::new(Harness::Codex);
    let server = ReleaseServer::start(&sb.env.platform(), None, true, true);
    sb.use_release(&server);
    fs::create_dir_all(&sb.bin_dir).unwrap();
    fs::write(guard_path(&sb), "old guard").unwrap();
    let out = sb.run("install", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    let now = fs::read(guard_path(&sb)).unwrap();
    if cfg!(windows) {
        assert_eq!(now, b"old guard", "Windows leaves an unrelated file alone");
    } else {
        assert_ne!(now, b"old guard");
        assert!(now.len() > 1024, "the release binary replaced it");
    }
}

#[test]
fn a_release_without_the_guard_installs_the_servers_with_one_warning() {
    let mut sb = Sandbox::new(Harness::Claude);
    let server = ReleaseServer::start_without_guard(&sb.env.platform(), true);
    sb.use_release(&server);
    let out = sb.run("install", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(!guard_path(&sb).exists());
    for name in ["aside", "dispatch", "palette"] {
        assert!(
            sb.bin_dir
                .join(exe_name(&sb.env.platform(), name))
                .is_file(),
            "{name}"
        );
    }
    assert_eq!(
        warning_lines(&out.out),
        usize::from(!cfg!(windows)),
        "{}",
        out.out
    );
    if !cfg!(windows) {
        assert!(
            out.out.contains("dispatch refuses to start backends"),
            "{}",
            out.out
        );
        assert!(out.out.contains("aside runs unguarded"), "{}", out.out);
    }
    assert_eq!(sb.added_servers().len(), 3);
}

#[test]
fn the_latest_release_fallback_without_a_guard_warns_once_more() {
    let mut sb = Sandbox::new(Harness::Codex);
    let server = ReleaseServer::start_without_guard(&sb.env.platform(), false);
    sb.use_release(&server);
    let out = sb.run("install", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(
        out.out.contains("slate release v0.7.0 does not exist"),
        "{}",
        out.out
    );
    assert_eq!(
        warning_lines(&out.out),
        usize::from(!cfg!(windows)),
        "{}",
        out.out
    );
}

#[cfg(unix)]
#[test]
fn a_guard_checksum_mismatch_fails_before_any_binary_is_replaced() {
    let mut sb = Sandbox::new(Harness::Claude);
    let platform = sb.env.platform();
    let server = ReleaseServer::start(&platform, Some(GUARD), true, true);
    sb.use_release(&server);
    let out = sb.run("install", &[]);
    assert_eq!(out.code, 1, "{}", out.out);
    assert!(
        out.out.contains("checksum mismatch for agent-guard-"),
        "{}",
        out.out
    );
    assert!(out.out.contains("no binary was replaced"), "{}", out.out);
    assert!(!sb.bin_dir.join("aside").exists());
    assert!(!guard_path(&sb).exists());
    assert!(sb.added_servers().is_empty());
}

#[test]
fn a_build_install_places_the_guard_from_the_checkout() {
    let mut sb = Sandbox::new(Harness::Claude);
    let slate = use_fake_checkout(&mut sb);
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
    assert_eq!(guard_path(&sb).is_file(), !cfg!(windows));
    assert_eq!(manifest_text(&sb).contains(GUARD), !cfg!(windows));
    let builds: Vec<Vec<String>> = sb
        .cli_calls()
        .into_iter()
        .filter(|c| c[0] == "build")
        .collect();
    for call in &builds {
        for test_support in ["stub-backend", "stub-parent"] {
            assert!(!call.iter().any(|a| a == test_support), "{call:?}");
        }
    }
    assert_eq!(builds.len(), if cfg!(windows) { 1 } else { 2 });
    let names: Vec<String> = sb.added_servers().into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["aside", "dispatch", "palette"], "not registered");

    let out = sb.run("uninstall", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(!guard_path(&sb).exists());
    assert!(
        !sb.bin_dir
            .join(exe_name(&sb.env.platform(), "aside"))
            .exists()
    );
}

#[test]
fn mcp_places_the_guard_and_never_registers_it() {
    let mut sb = Sandbox::new(Harness::Claude);
    let server = ReleaseServer::start(&sb.env.platform(), None, false, true);
    sb.use_release(&server);
    let (code, out) = mcp(&sb, &["--harness", "claude,codex"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(guard_path(&sb).is_file(), !cfg!(windows), "{out}");
    assert_eq!(
        out.contains("agent-guard"),
        !cfg!(windows),
        "summary lists it: {out}"
    );
    let names: Vec<String> = sb.added_servers().into_iter().map(|(n, _)| n).collect();
    assert_eq!(names.len(), 6);
    assert!(names.iter().all(|n| n != GUARD), "{names:?}");
    let manifest = fs::read_to_string(
        sb.env
            .default_home(Harness::Claude)
            .join(".slate-agent-kit-mcp-manifest.toml"),
    )
    .unwrap();
    assert!(!manifest.contains(GUARD), "{manifest}");
}

#[test]
fn mcp_build_mode_and_an_old_release_follow_the_same_rules() {
    let mut sb = Sandbox::new(Harness::Codex);
    let slate = use_fake_checkout(&mut sb);
    let (code, out) = mcp(
        &sb,
        &[
            "--binaries",
            "build",
            "--slate-dir",
            slate.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "{out}");
    assert_eq!(guard_path(&sb).is_file(), !cfg!(windows), "{out}");

    let mut old = Sandbox::new(Harness::Codex);
    let server = ReleaseServer::start_without_guard(&old.env.platform(), false);
    old.use_release(&server);
    let (code, out) = mcp(&old, &[]);
    assert_eq!(code, 0, "{out}");
    assert!(!guard_path(&old).exists());
    assert_eq!(warning_lines(&out), usize::from(!cfg!(windows)), "{out}");
}

#[test]
fn skip_mode_touches_nothing() {
    let sb = Sandbox::new(Harness::Claude);
    let out = sb.run("install", &["--binaries", "skip"]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(!sb.bin_dir.exists(), "{}", out.out);
    assert_eq!(warning_lines(&out.out), 0);
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
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

/// Rewrites a kit's manifest as an installer from before the guard would have: no guard entry.
fn strip_guard_from_manifest(path: &std::path::Path) {
    let mut table: toml::Table = toml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let binaries = table["binaries"].as_array_mut().unwrap();
    binaries.retain(|b| !b.as_str().unwrap().contains(GUARD));
    fs::write(path, toml::to_string(&table).unwrap()).unwrap();
}

/// A Codex kit and a Claude kit in one scratch home, sharing the binary folder. The Codex
/// kit is installed first and its manifest is then rewritten without the guard.
fn two_kits_with_an_older_first() -> (Sandbox, Sandbox) {
    let mut claude = Sandbox::new(Harness::Claude);
    let server = Box::leak(Box::new(ReleaseServer::start(
        &claude.env.platform(),
        None,
        true,
        true,
    )));
    claude.use_release(server);
    let mut codex = Sandbox::new(Harness::Codex);
    codex.env = claude.env.clone();
    codex.payload = claude.dir.path().join("dist-codex");
    copy_tree(&support::fixture("payload-codex"), &codex.payload);
    codex.bin_dir = claude.bin_dir.clone();
    codex.log = claude.log.clone();
    codex.dir = tempfile::tempdir().unwrap();

    let out = codex.run("install", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    strip_guard_from_manifest(&codex.home().join(".codex-agent-kit-manifest.toml"));
    let out = claude.run("install", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    (claude, codex)
}

#[test]
fn the_guard_stays_while_a_kit_that_predates_it_still_lists_its_servers() {
    let (claude, codex) = two_kits_with_an_older_first();
    assert_eq!(guard_path(&claude).is_file(), !cfg!(windows));
    let out = claude.run("uninstall", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    let platform = claude.env.platform();
    for name in ["aside", "dispatch", "palette"] {
        assert!(
            claude.bin_dir.join(exe_name(&platform, name)).exists(),
            "{name}"
        );
    }
    // Windows has no guard; elsewhere it stays for the older kit's servers.
    assert_eq!(guard_path(&claude).exists(), !cfg!(windows), "{}", out.out);
    if !cfg!(windows) {
        assert!(
            out.out.contains("also lists it or a server that needs it"),
            "{}",
            out.out
        );
    }
    drop(codex);
}

#[test]
fn the_guard_is_removed_when_no_other_kit_lists_it_or_a_server() {
    let (claude, codex) = two_kits_with_an_older_first();
    // The older kit goes away first: it does not list the guard, so the guard stays for the other.
    let out = codex.run("uninstall", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert_eq!(guard_path(&claude).exists(), !cfg!(windows));
    // The kit that lists the guard is now the only one left: uninstalling it removes the guard.
    let out = claude.run("uninstall", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(!guard_path(&claude).exists(), "{}", out.out);
    assert!(
        !claude
            .bin_dir
            .join(exe_name(&claude.env.platform(), "aside"))
            .exists()
    );
}
