//! The per-harness layout (RFC-0023) leaves Claude Code, Codex and Kimi Code
//! where they were: the same files under the same home, the manifest beside
//! them, the same state home, and custom rules signed on their first line.

mod support;

use slate_setup::env::Harness;
use std::fs;
use support::{ReleaseServer, Sandbox, fixture, read};

/// The files an install with custom rules leaves in the user's home, as the
/// installer wrote them before the layout existed; backups carry `<TS>`.
fn expected(h: Harness) -> Vec<String> {
    let (dir, kit, primary) = match h {
        Harness::Claude => (".claude", "claude-agent-kit", "CLAUDE.md"),
        Harness::Codex => (".codex", "codex-agent-kit", "AGENTS.md"),
        Harness::Kimi => (".kimi-code", "kimi-agent-kit", "AGENTS.md"),
        Harness::Agy => unreachable!("agy has its own flow tests"),
    };
    let mut v = vec![
        format!(".{kit}-manifest.toml"),
        primary.to_string(),
        format!("{primary}.bak-<TS>"),
    ];
    match h {
        Harness::Claude => {
            v.push("settings.json".into());
            v.push("settings.json.bak-<TS>".into());
        }
        Harness::Codex => v.push("config.toml".into()),
        Harness::Kimi => {
            v.push("config.toml".into());
            v.push("plugins/installed.json".into());
            v.push("plugins/managed/slate-agent-kit-mcp/SKILL.md".into());
            v.push("plugins/managed/slate-agent-kit-mcp/kimi.plugin.json".into());
        }
        Harness::Agy => {}
    }
    for r in [
        "comment-prefs",
        "consult-prefs",
        "consult",
        "dispatch-prefs",
        "git-prefs",
        "signed",
        "style",
        "subagent-prefs",
        "task-execution",
    ] {
        v.push(format!("rules/{kit}--{r}.md"));
    }
    v.push("skills/palette-init/SKILL.md".into());
    v.push("skills/palette-init/notes/extra.md".into());
    let mut v: Vec<String> = v.into_iter().map(|p| format!("{dir}/{p}")).collect();
    v.sort();
    v
}

/// Replaces the time stamp after `.bak-` with `<TS>`.
fn mask_backup(p: &str) -> String {
    match p.find(".bak-") {
        Some(i) => format!("{}<TS>", &p[..i + 5]),
        None => p.to_string(),
    }
}

#[test]
fn claude_codex_and_kimi_install_into_the_same_places_as_before() {
    for h in [Harness::Claude, Harness::Codex, Harness::Kimi] {
        let mut sb = Sandbox::new(h);
        let server = ReleaseServer::start(&sb.env.platform(), None, true, true);
        sb.use_release(&server);
        let home = sb.home();
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join(h.primary_file()), "# My own instructions\n").unwrap();
        let config = match h {
            Harness::Claude => ("configs/claude-settings.json", "settings.json"),
            Harness::Codex => ("configs/codex-config.toml", "config.toml"),
            _ => ("configs/kimi-config.toml", "config.toml"),
        };
        fs::copy(fixture(config.0), home.join(config.1)).unwrap();
        let custom = fixture("custom-rules").to_string_lossy().into_owned();
        let out = sb.run("install", &["--custom-rules", custom.as_str()]);
        assert_eq!(out.code, 0, "{}", out.out);

        let mut tree: Vec<String> = sb
            .tree(&sb.user_home)
            .iter()
            .map(|p| mask_backup(p))
            .collect();
        tree.sort();
        assert_eq!(tree, expected(h), "{}", h.name());

        // The state home is where it was: the home for Claude Code, a folder in it otherwise.
        let state = match h {
            Harness::Claude => home.clone(),
            _ => home.join("slate-agent-kit"),
        };
        assert!(state.is_dir(), "{}", h.name());
        if h != Harness::Kimi {
            let added = sb.added_servers();
            assert!(
                added[0]
                    .1
                    .contains(&format!("SLATE_AGENT_STATE_HOME={}", state.display())),
                "{}: {:?}",
                h.name(),
                added
            );
        }

        // Custom rules get the signature on their first line and no frontmatter.
        let kit = sb.kit();
        assert_eq!(
            read(&home.join("rules").join(format!("{kit}--style.md"))),
            format!("<!-- {kit}-custom:user -->\n# My style rule\n\nPrefer small functions.\n")
        );
    }
}
