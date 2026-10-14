//! End-to-end install, reinstall, upgrade, configure and uninstall for the
//! Antigravity CLI (agy) in an isolated home, with a stand-in `agy` that records
//! its arguments and writes `config/mcp_config.json` like the real one.

mod support;

use slate_setup::binaries::exe_name;
use slate_setup::env::{Harness, process_home_var};
use slate_setup::json::Json;
use slate_setup::manifest::{EntryKind, Manifest, read_manifest_file};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use support::{ReleaseServer, Sandbox, read};

const KIT: &str = "agy-agent-kit";

/// The nine `permissions.allow` entries RFC-0023 names.
const APPROVALS: [&str; 9] = [
    "mcp(dispatch/dispatch_consult)",
    "mcp(dispatch/dispatch_status)",
    "mcp(dispatch/dispatch_list)",
    "mcp(dispatch/dispatch_logs)",
    "mcp(dispatch/dispatch_backends)",
    "mcp(palette/palette_status)",
    "mcp(palette/palette_lint)",
    "mcp(palette/palette_layout)",
    "mcp(palette/palette_template)",
];

/// A user's own Antigravity CLI settings, with keys the installer must keep.
const USER_SETTINGS: &str = "{\n  \"agentMode\": \"fast\",\n  \"permissions\": {\n    \"allow\": [\n      \"command(ls)\",\n      \"mcp(other/tool)\"\n    ],\n    \"ask\": [],\n    \"deny\": [\"command(rm)\"]\n  },\n  \"toolPermission\": {\"mode\": \"ask\"},\n  \"enableTerminalSandbox\": true\n}\n";

fn sandbox() -> (Sandbox, ReleaseServer) {
    let mut sb = Sandbox::new(Harness::Agy);
    let server = ReleaseServer::start(&sb.env.platform(), None, true, true);
    sb.use_release(&server);
    (sb, server)
}

fn gemini(sb: &Sandbox) -> PathBuf {
    sb.user_home.join(".gemini")
}

fn settings_path(sb: &Sandbox) -> PathBuf {
    gemini(sb).join("antigravity-cli").join("settings.json")
}

fn manifest_path(sb: &Sandbox) -> PathBuf {
    gemini(sb)
        .join("slate-agent-kit")
        .join(format!(".{KIT}-manifest.toml"))
}

fn allow_list(path: &Path) -> Vec<String> {
    let j = Json::parse(&fs::read_to_string(path).unwrap()).unwrap();
    j.get("permissions")
        .and_then(|p| p.get("allow"))
        .and_then(Json::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The servers in the `mcp_config.json` the stand-in agy keeps, with their `env`.
fn agy_servers(sb: &Sandbox) -> BTreeMap<String, Json> {
    let path = gemini(sb).join("config").join("mcp_config.json");
    let Ok(text) = fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    match Json::parse(&text).unwrap().get("mcpServers") {
        Some(Json::Object(m)) => m.iter().cloned().collect(),
        _ => BTreeMap::new(),
    }
}

/// Every file under `root` with its bytes, keyed by its path relative to `root` with `/` separators.
fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                let rel = p
                    .strip_prefix(base)
                    .unwrap()
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                out.insert(rel, fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// `path` relative to `root`, with `/` separators.
fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or_else(|_| panic!("{} is not under {}", path.display(), root.display()))
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// A manifest's text without its `updated_at` line, which every run rewrites.
fn manifest_without_time(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter(|l| !l.starts_with("updated_at = "))
        .collect::<Vec<_>>()
        .join("\n")
}

fn custom_rules(sb: &Sandbox) -> PathBuf {
    let dir = sb.dir.path().join("my-rules");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("style.md"),
        "# My style rule\n\nPrefer small functions.\n",
    )
    .unwrap();
    fs::write(
        dir.join("rust.md"),
        "---\ntrigger: glob\nglobs: \"*.rs\"\n---\n# Rust\n\nNo unwrap.\n",
    )
    .unwrap();
    dir
}

const FRONT: &str = "---\ntrigger: always_on\ndescription: ";

#[test]
fn install_writes_where_agy_reads_registers_and_records_exactly_what_it_added() {
    let (sb, _server) = sandbox();
    let platform = sb.env.platform();
    let g = gemini(&sb);
    // The user's own settings, an unrelated rule, and a manual the kit did not write.
    fs::create_dir_all(settings_path(&sb).parent().unwrap()).unwrap();
    fs::write(settings_path(&sb), USER_SETTINGS).unwrap();
    fs::create_dir_all(g.join("config").join("rules")).unwrap();
    fs::write(g.join("config").join("AGENTS.md"), "# My own manual\n").unwrap();
    let own_rule = "---\ntrigger: manual\n---\nmine\n";
    fs::write(g.join("config").join("rules").join("mine.md"), own_rule).unwrap();
    let roots = sb.dir.path().join("work");
    fs::create_dir_all(&roots).unwrap();
    let roots_arg = roots.to_string_lossy().into_owned();
    let custom = custom_rules(&sb);
    let custom_arg = custom.to_string_lossy().into_owned();
    let before = files(&sb.user_home);

    let out = sb.run(
        "install",
        &[
            "--roots",
            roots_arg.as_str(),
            "--custom-rules",
            custom_arg.as_str(),
            "--set",
            "dispatch.backend=agy",
            "--set",
            "subagent.model=claude-sonnet-5-5",
        ],
    );
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(out.out.contains("Antigravity CLI"), "{}", out.out);

    // The manual has no frontmatter; rules and prefs start with the block, then the signature.
    let config = g.join("config");
    assert_eq!(
        read(&config.join("AGENTS.md")),
        read(&sb.payload.join("AGENTS.md"))
    );
    assert!(read(&config.join("AGENTS.md")).starts_with("<!-- slate-agent-kit:common -->\n"));
    for rule in [
        "agy-agent-kit--task-execution.md",
        "agy-agent-kit--consult.md",
    ] {
        let text = read(&config.join("rules").join(rule));
        assert_eq!(text, read(&sb.payload.join("rules").join(rule)));
        assert!(text.starts_with(FRONT), "{text}");
        assert!(
            text.contains("\"\n---\n<!-- slate-agent-kit:common -->\n"),
            "{text}"
        );
    }
    for name in ["consult", "dispatch", "subagent", "git", "comment"] {
        let text = read(&sb.prefs_path(name));
        assert!(text.starts_with(FRONT), "{name}: {text}");
        assert!(
            text.contains(&format!(
                "\"\n---\n<!-- agy-agent-kit-custom:{name}-prefs -->\n"
            )),
            "{name}: {text}"
        );
    }
    assert!(read(&sb.prefs_path("dispatch")).contains("## Backend\n\n**agy**\n"));
    assert!(
        read(&sb.prefs_path("subagent")).contains("## Default model\n\n**claude-sonnet-5-5**\n"),
        "the subagent model is kept as written, though agy has no setting for it"
    );
    // Custom rules: the block first and the signature after it, with or without a block of their own.
    assert_eq!(
        read(&config.join("rules").join("agy-agent-kit--style.md")),
        "---\ntrigger: always_on\ndescription: \"Antigravity Agent: style\"\n---\n<!-- agy-agent-kit-custom:user -->\n# My style rule\n\nPrefer small functions.\n"
    );
    assert_eq!(
        read(&config.join("rules").join("agy-agent-kit--rust.md")),
        "---\ntrigger: glob\nglobs: \"*.rs\"\n---\n<!-- agy-agent-kit-custom:user -->\n# Rust\n\nNo unwrap.\n"
    );
    assert!(
        config
            .join("skills")
            .join("palette-init")
            .join("SKILL.md")
            .is_file()
    );
    assert!(
        config
            .join("skills")
            .join("palette-init")
            .join("notes")
            .join("extra.md")
            .is_file()
    );
    assert_eq!(read(&config.join("rules").join("mine.md")), own_rule);

    // Registration: remove then add for each server, flags before the name, in the user's home.
    let state = g.join("slate-agent-kit");
    let bin = |n: &str| {
        sb.bin_dir
            .join(exe_name(&platform, n))
            .to_string_lossy()
            .into_owned()
    };
    let calls = sb.cli_calls();
    assert_eq!(
        calls,
        vec![
            vec!["mcp".to_string(), "remove".into(), "dispatch".into()],
            vec![
                "mcp".to_string(),
                "add".into(),
                "--env".into(),
                format!("SLATE_AGENT_STATE_HOME={}", state.display()),
                "--env".into(),
                "SLATE_HARNESS=agy".into(),
                "--env".into(),
                format!("DISPATCH_EXTRA_ROOTS={}", roots.display()),
                "dispatch".into(),
                bin("dispatch"),
            ],
            vec!["mcp".to_string(), "remove".into(), "palette".into()],
            vec![
                "mcp".to_string(),
                "add".into(),
                "--env".into(),
                format!("PALETTE_EXTRA_ROOTS={}", roots.display()),
                "palette".into(),
                bin("palette"),
            ],
        ]
    );
    let homes = sb.cli_env(process_home_var());
    assert_eq!(homes.len(), 4);
    assert!(
        homes.iter().all(|h| Path::new(h) == sb.user_home.as_path()),
        "{homes:?}"
    );
    let servers = agy_servers(&sb);
    assert_eq!(servers.keys().collect::<Vec<_>>(), ["dispatch", "palette"]);
    assert_eq!(
        servers["dispatch"]
            .get("env")
            .and_then(|e| e.get("SLATE_HARNESS"))
            .and_then(Json::as_str),
        Some("agy")
    );
    assert!(state.is_dir(), "the servers' state home exists");

    // Approvals: the user's entries and keys stay, the nine are added once.
    let mut want: Vec<String> = vec!["command(ls)".into(), "mcp(other/tool)".into()];
    want.extend(APPROVALS.iter().map(|s| s.to_string()));
    assert_eq!(allow_list(&settings_path(&sb)), want);
    let settings = Json::parse(&fs::read_to_string(settings_path(&sb)).unwrap()).unwrap();
    let user = Json::parse(USER_SETTINGS).unwrap();
    for key in ["agentMode", "toolPermission", "enableTerminalSandbox"] {
        assert_eq!(settings.get(key), user.get(key), "{key}");
    }
    for key in ["ask", "deny"] {
        assert_eq!(
            settings.get("permissions").and_then(|p| p.get(key)),
            user.get("permissions").and_then(|p| p.get(key)),
            "permissions.{key}"
        );
    }

    // The manifest lives in the state folder and records every file, backup, registration and key.
    assert!(!g.join(format!(".{KIT}-manifest.toml")).exists());
    let m = read_manifest_file(&manifest_path(&sb)).unwrap();
    assert_eq!(m.version, "13.0.0");
    let regs: Vec<(&str, &str)> = m
        .registrations
        .iter()
        .map(|r| (r.harness.as_str(), r.server.as_str()))
        .collect();
    assert_eq!(regs, [("agy", "dispatch"), ("agy", "palette")]);
    assert_eq!(m.config.len(), 1);
    assert_eq!(m.config[0].file, settings_path(&sb));
    assert_eq!(m.config[0].path, ["permissions", "allow"]);
    assert_eq!(m.config[0].added, APPROVALS);
    assert_eq!(m.backups.len(), 1);
    assert_eq!(m.backups[0].original, config.join("AGENTS.md"));
    assert_eq!(
        m.backups[0].path.parent(),
        Some(state.join("backups").as_path()),
        "backups go to the state folder"
    );
    assert_eq!(
        fs::read_to_string(&m.backups[0].path).unwrap(),
        "# My own manual\n"
    );

    // The file-system difference of the home is exactly what the manifest records,
    // plus the manifest itself and the registry agy writes for the registrations.
    let after = files(&sb.user_home);
    let mut recorded: BTreeSet<String> = BTreeSet::new();
    for f in &m.files {
        match f.kind {
            EntryKind::File => {
                recorded.insert(rel(&sb.user_home, &f.path));
            }
            EntryKind::Dir => {
                for k in files(&f.path).keys() {
                    recorded.insert(format!("{}/{k}", rel(&sb.user_home, &f.path)));
                }
            }
        }
    }
    for b in &m.backups {
        recorded.insert(rel(&sb.user_home, &b.path));
    }
    recorded.insert(rel(&sb.user_home, &manifest_path(&sb)));
    recorded.insert(".gemini/config/mcp_config.json".into());
    let added: BTreeSet<String> = after
        .keys()
        .filter(|k| !before.contains_key(*k))
        .cloned()
        .collect();
    let changed: BTreeSet<String> = before
        .iter()
        .filter(|(k, v)| after.get(*k) != Some(v))
        .map(|(k, _)| k.clone())
        .collect();
    let expected_new: BTreeSet<String> = recorded
        .iter()
        .filter(|k| !before.contains_key(*k))
        .cloned()
        .collect();
    assert_eq!(added, expected_new);
    assert_eq!(
        changed,
        BTreeSet::from([
            ".gemini/antigravity-cli/settings.json".to_string(),
            ".gemini/config/AGENTS.md".to_string(),
        ]),
        "only the recorded key's file and the backed-up manual changed"
    );
    // Everything stays inside the folders RFC-0023 allows.
    for k in added.iter().chain(&changed) {
        assert!(
            k.starts_with(".gemini/config/")
                || k == ".gemini/antigravity-cli/settings.json"
                || k.starts_with(".gemini/slate-agent-kit/"),
            "{k}"
        );
    }

    // Reinstall changes no byte apart from the manifest's time stamp.
    sb.clear_log();
    let out = sb.run(
        "install",
        &[
            "--roots",
            roots_arg.as_str(),
            "--custom-rules",
            custom_arg.as_str(),
        ],
    );
    assert_eq!(out.code, 0, "{}", out.out);
    let again = files(&sb.user_home);
    assert_eq!(
        again.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>()
    );
    let manifest_key = rel(&sb.user_home, &manifest_path(&sb));
    for (k, v) in &after {
        if *k == manifest_key {
            assert_eq!(manifest_without_time(v), manifest_without_time(&again[k]));
        } else {
            assert_eq!(v, &again[k], "{k} changed on reinstall");
        }
    }
}

#[test]
fn a_fresh_home_gets_a_settings_file_and_uninstall_takes_back_exactly_what_was_recorded() {
    let (sb, _server) = sandbox();
    let platform = sb.env.platform();
    let custom = custom_rules(&sb);
    let custom_arg = custom.to_string_lossy().into_owned();
    let out = sb.run("install", &["--custom-rules", custom_arg.as_str()]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert_eq!(allow_list(&settings_path(&sb)), APPROVALS);
    let m = read_manifest_file(&manifest_path(&sb)).unwrap();
    assert!(m.created_files.contains(&settings_path(&sb)));
    assert!(m.created_dirs.contains(&gemini(&sb)));

    // Uninstall without a terminal keeps the user's files and removes the rest.
    sb.clear_log();
    let out = sb.run("uninstall", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert_eq!(
        sb.cli_calls(),
        [
            ["mcp", "remove", "dispatch"].map(String::from).to_vec(),
            ["mcp", "remove", "palette"].map(String::from).to_vec(),
        ]
    );
    assert!(agy_servers(&sb).is_empty());
    assert!(
        !settings_path(&sb).exists(),
        "the settings file the installer created is gone once empty"
    );
    assert!(!manifest_path(&sb).exists());
    for name in ["dispatch", "palette"] {
        assert!(!sb.bin_dir.join(exe_name(&platform, name)).exists());
    }
    // What is left is what the user owns, and the registry agy keeps itself.
    let mut left: Vec<String> = files(&gemini(&sb)).into_keys().collect();
    left.sort();
    assert_eq!(
        left,
        [
            "config/mcp_config.json",
            "config/rules/agy-agent-kit--comment-prefs.md",
            "config/rules/agy-agent-kit--consult-prefs.md",
            "config/rules/agy-agent-kit--dispatch-prefs.md",
            "config/rules/agy-agent-kit--git-prefs.md",
            "config/rules/agy-agent-kit--rust.md",
            "config/rules/agy-agent-kit--style.md",
            "config/rules/agy-agent-kit--subagent-prefs.md",
        ],
        "{}",
        out.out
    );
    assert!(!gemini(&sb).join("config").join("skills").exists());
    assert!(!gemini(&sb).join("antigravity-cli").exists());
    assert!(!gemini(&sb).join("slate-agent-kit").exists());
}

#[test]
fn uninstall_keeps_the_users_permissions_and_settings_keys() {
    let (sb, _server) = sandbox();
    fs::create_dir_all(settings_path(&sb).parent().unwrap()).unwrap();
    fs::write(settings_path(&sb), USER_SETTINGS).unwrap();
    let out = sb.run("install", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    // The user adds an entry of their own after the install.
    let text = fs::read_to_string(settings_path(&sb)).unwrap().replace(
        "\"command(ls)\",",
        "\"command(ls)\",\n      \"command(pwd)\",",
    );
    fs::write(settings_path(&sb), text).unwrap();
    let out = sb.run("uninstall", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert_eq!(
        allow_list(&settings_path(&sb)),
        ["command(ls)", "command(pwd)", "mcp(other/tool)"]
    );
    // Every other key is the user's; the JSON editor keeps key order and
    // indentation, as it does for Claude Code's settings.json.
    let settings = Json::parse(&fs::read_to_string(settings_path(&sb)).unwrap()).unwrap();
    let want = Json::parse(&USER_SETTINGS.replace(
        "\"command(ls)\",",
        "\"command(ls)\",\n      \"command(pwd)\",",
    ))
    .unwrap();
    assert_eq!(settings, want);
    let text = fs::read_to_string(settings_path(&sb)).unwrap();
    assert!(text.starts_with("{\n  \"agentMode\": \"fast\",\n  \"permissions\": {\n"));
}

#[test]
fn upgrading_over_a_previous_manifest_keeps_user_owned_files() {
    let (sb, _server) = sandbox();
    let custom = custom_rules(&sb);
    let custom_arg = custom.to_string_lossy().into_owned();
    let out = sb.run("install", &["--custom-rules", custom_arg.as_str()]);
    assert_eq!(out.code, 0, "{}", out.out);
    let rules = gemini(&sb).join("config").join("rules");

    // The user edits a prefs value by hand and keeps a rule of their own beside the kit's.
    let git = sb.prefs_path("git");
    let edited = fs::read_to_string(&git).unwrap().replace(
        "## Commit signing\n\n**unset**",
        "## Commit signing\n\n**no-gpg-sign**",
    );
    assert!(edited.contains("**no-gpg-sign**"));
    fs::write(&git, &edited).unwrap();
    let style = rules.join("agy-agent-kit--style.md");
    let style_text = fs::read_to_string(&style).unwrap();
    let mine = rules.join("mine.md");
    fs::write(&mine, "---\ntrigger: manual\n---\nmine\n").unwrap();

    // The next release changes one rule and drops the other.
    let desc = sb.payload.join("kit.toml");
    let text = fs::read_to_string(&desc)
        .unwrap()
        .replace("version = \"13.0.0\"", "version = \"13.1.0\"")
        .replace(
            "rules = [\"agy-agent-kit--task-execution.md\", \"agy-agent-kit--consult.md\"]",
            "rules = [\"agy-agent-kit--task-execution.md\"]",
        );
    fs::write(&desc, text).unwrap();
    let rule = sb
        .payload
        .join("rules")
        .join("agy-agent-kit--task-execution.md");
    let new_rule = fs::read_to_string(&rule)
        .unwrap()
        .replace("Fixture rule text.", "Fixture rule text, revised.");
    fs::write(&rule, &new_rule).unwrap();

    let out = sb.run("install", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert_eq!(fs::read_to_string(&git).unwrap(), edited);
    assert_eq!(
        fs::read_to_string(&style).unwrap(),
        style_text,
        "the custom rule copied earlier stays"
    );
    assert_eq!(read(&mine), "---\ntrigger: manual\n---\nmine\n");
    assert_eq!(
        read(&rules.join("agy-agent-kit--task-execution.md")),
        new_rule.replace("\r\n", "\n")
    );
    assert!(
        !rules.join("agy-agent-kit--consult.md").exists(),
        "a kit rule the release no longer ships is removed"
    );
    let m = read_manifest_file(&manifest_path(&sb)).unwrap();
    assert_eq!(m.version, "13.1.0");
    assert!(m.files.iter().any(|f| f.path == style && f.user));
}

#[test]
fn ownership_follows_the_signature_after_the_block_on_disk() {
    let (sb, _server) = sandbox();
    let rules = gemini(&sb).join("config").join("rules");
    fs::create_dir_all(&rules).unwrap();
    // A kit-managed rule written with a BOM and CRLF line endings: replaced, no backup.
    let crlf = "\u{feff}---\r\ntrigger: always_on\r\ndescription: \"old\"\r\n---\r\n<!-- slate-agent-kit:common -->\r\nold text\r\n";
    fs::write(rules.join("agy-agent-kit--task-execution.md"), crlf).unwrap();
    // A file whose block never closes: not recognised, so it is backed up before it is replaced.
    let broken = "---\ntrigger: always_on\n<!-- slate-agent-kit:common -->\nbroken\n";
    fs::write(rules.join("agy-agent-kit--consult.md"), broken).unwrap();
    // A prefs file the user already has, with the block and the custom signature: kept as it is.
    let prefs = "---\ntrigger: always_on\ndescription: \"mine\"\n---\n<!-- agy-agent-kit-custom:git-prefs -->\n# Git\n\n## Commit signing\n\n**default**\n\n## Model attribution\n\n**off**\n\n## Commit message format\n\n**repository**\n\n## PR body format\n\n**repository**\n\n## Branch naming\n\n**repository**\n";
    fs::write(sb.prefs_path("git"), prefs).unwrap();

    let out = sb.run("install", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert_eq!(
        read(&rules.join("agy-agent-kit--task-execution.md")),
        read(
            &sb.payload
                .join("rules")
                .join("agy-agent-kit--task-execution.md")
        )
    );
    assert_eq!(
        read(&rules.join("agy-agent-kit--consult.md")),
        read(&sb.payload.join("rules").join("agy-agent-kit--consult.md"))
    );
    assert_eq!(fs::read_to_string(sb.prefs_path("git")).unwrap(), prefs);
    let m = read_manifest_file(&manifest_path(&sb)).unwrap();
    let backed_up: Vec<&Path> = m.backups.iter().map(|b| b.original.as_path()).collect();
    assert_eq!(
        backed_up,
        [rules.join("agy-agent-kit--consult.md").as_path()]
    );
    assert_eq!(fs::read_to_string(&m.backups[0].path).unwrap(), broken);
    // Uninstall sees the prefs file as the user's and the rules as the kit's.
    let out = sb.run("uninstall", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(sb.prefs_path("git").is_file());
    assert!(!rules.join("agy-agent-kit--task-execution.md").exists());
}

#[test]
fn a_home_other_than_gemini_under_the_process_home_is_refused() {
    let (sb, _server) = sandbox();
    let elsewhere = sb.dir.path().join("elsewhere").join(".gemini");
    let arg = elsewhere.to_string_lossy().into_owned();
    let out = sb.run("install", &["--home", arg.as_str()]);
    assert_ne!(out.code, 0, "{}", out.out);
    assert!(out.out.contains("cannot be honoured"), "{}", out.out);
    assert!(out.out.contains(process_home_var()), "{}", out.out);
    assert!(!elsewhere.exists() && !gemini(&sb).exists());
    let out = sb.run("mcp", &["--harness", "agy", "--home", arg.as_str()]);
    assert_ne!(out.code, 0, "{}", out.out);
    assert!(out.out.contains("cannot be honoured"), "{}", out.out);
    assert!(sb.cli_calls().is_empty());

    // The folder agy itself uses is accepted.
    let own = gemini(&sb).to_string_lossy().into_owned();
    let out = sb.run("install", &["--home", own.as_str()]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(manifest_path(&sb).is_file());
}

#[test]
fn a_shared_binary_stays_while_another_harness_lists_it() {
    let (sb, _server) = sandbox();
    let platform = sb.env.platform();
    let dispatch = sb.bin_dir.join(exe_name(&platform, "dispatch"));
    let claude = sb.add_payload(Harness::Claude);
    // agy and Claude Code share the binary folder.
    let out = sb.run("install", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    let out = sb.run_payload(&claude, "install", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    // Claude Code's uninstall sees agy's manifest in ~/.gemini/slate-agent-kit.
    let out = sb.run_payload(&claude, "uninstall", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(dispatch.is_file(), "{}", out.out);
    assert!(
        out.out.contains(&manifest_path(&sb).display().to_string()),
        "{}",
        out.out
    );
    // Now Claude Code installs again and agy uninstalls: Claude Code's manifest keeps it.
    let out = sb.run_payload(&claude, "install", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    let out = sb.run("uninstall", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(dispatch.is_file(), "{}", out.out);
    assert!(
        out.out.contains(".claude-agent-kit-manifest.toml"),
        "{}",
        out.out
    );
    // The last kit out removes it.
    let out = sb.run_payload(&claude, "uninstall", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(!dispatch.exists(), "{}", out.out);
}

#[test]
fn the_mcp_command_registers_with_agy_and_reverses_it() {
    let (sb, _server) = sandbox();
    let out = sb.run("mcp", &["--harness", "agy", "--slate-version", "0.7.0"]);
    assert_eq!(out.code, 0, "{}", out.out);
    let added = sb.added_servers();
    let names: Vec<&str> = added.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["dispatch", "palette"]);
    assert!(added[0].1.contains(&"SLATE_HARNESS=agy".to_string()));
    assert_eq!(allow_list(&settings_path(&sb)), APPROVALS);
    let mcp_manifest = gemini(&sb)
        .join("slate-agent-kit")
        .join(".slate-agent-kit-mcp-manifest.toml");
    let m = read_manifest_file(&mcp_manifest).unwrap();
    assert_eq!(m.registrations.len(), 2);
    assert_eq!(m.config[0].added, APPROVALS);

    let out = sb.run("mcp", &["--harness", "agy", "--uninstall"]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(agy_servers(&sb).is_empty());
    assert!(!settings_path(&sb).exists());
    assert!(!mcp_manifest.exists());
}

#[test]
fn a_server_the_release_no_longer_ships_is_retired_from_agy() {
    let (sb, _server) = sandbox();
    fs::create_dir_all(settings_path(&sb).parent().unwrap()).unwrap();
    fs::write(settings_path(&sb), USER_SETTINGS).unwrap();
    let out = sb.run("install", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    // An earlier release had registered `aside` and approved one of its tools.
    let mut m: Manifest = read_manifest_file(&manifest_path(&sb)).unwrap();
    let mut reg = m.registrations[0].clone();
    reg.server = "aside".into();
    m.registrations.push(reg);
    m.config[0].added.push("mcp(aside/aside_codex)".into());
    m.save(&gemini(&sb).join("slate-agent-kit")).unwrap();
    let text = fs::read_to_string(settings_path(&sb)).unwrap().replace(
        "\"mcp(palette/palette_template)\"",
        "\"mcp(palette/palette_template)\",\n      \"mcp(aside/aside_codex)\"",
    );
    fs::write(settings_path(&sb), text).unwrap();
    let path = gemini(&sb).join("config").join("mcp_config.json");
    let registry = fs::read_to_string(&path).unwrap().replacen(
        "{\"mcpServers\":{",
        "{\"mcpServers\":{\"aside\":{},",
        1,
    );
    fs::write(&path, registry).unwrap();

    sb.clear_log();
    let out = sb.run("install", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(
        sb.removed_servers().contains(&"aside".to_string()),
        "{}",
        out.out
    );
    assert!(!agy_servers(&sb).contains_key("aside"));
    let allow = allow_list(&settings_path(&sb));
    assert!(!allow.iter().any(|a| a.contains("aside")), "{allow:?}");
    assert!(allow.contains(&"mcp(palette/palette_template)".to_string()));
    let m = read_manifest_file(&manifest_path(&sb)).unwrap();
    assert!(!m.registrations.iter().any(|r| r.server == "aside"));
    assert_eq!(m.config[0].added, APPROVALS);
}

#[test]
fn agy_is_a_dispatch_backend_and_not_a_consultation_backend() {
    let (sb, _server) = sandbox();
    let out = sb.run("install", &["--set", "consult.backend=agy"]);
    assert_ne!(out.code, 0, "{}", out.out);
    assert!(out.out.contains("codex, claude"), "{}", out.out);
    let out = sb.run("install", &["--set", "dispatch.backend=agy"]);
    assert_eq!(out.code, 0, "{}", out.out);
}

#[test]
fn configure_changes_a_prefs_value_and_keeps_the_block_and_the_signature() {
    let (sb, _server) = sandbox();
    let out = sb.run("install", &[]);
    assert_eq!(out.code, 0, "{}", out.out);
    let before = read(&sb.prefs_path("git"));
    let out = sb.run("configure", &["--set", "git.signing=no-gpg-sign"]);
    assert_eq!(out.code, 0, "{}", out.out);
    let after = read(&sb.prefs_path("git"));
    assert!(
        after.contains("## Commit signing\n\n**no-gpg-sign**\n"),
        "{after}"
    );
    assert!(
        after.starts_with(
            "---\ntrigger: always_on\ndescription: \"Antigravity Agent: Git Preferences\"\n---\n<!-- agy-agent-kit-custom:git-prefs -->\n"
        ),
        "{after}"
    );
    // Only the value line changed.
    let diff: Vec<(&str, &str)> = before
        .lines()
        .zip(after.lines())
        .filter(|(a, b)| a != b)
        .collect();
    assert_eq!(diff.len(), 1, "{diff:?}");
}
