//! Kimi Code: the local plugin that exposes the MCP servers, its registry
//! entry, and the `config.toml` edits for the secondary (subagent) model.
//!
//! Owns the plugin folder `<home>/plugins/managed/slate-agent-kit-mcp/`
//! (`kimi.plugin.json` and `SKILL.md`), the `plugins/installed.json` entry
//! that keeps every other entry, and the `[secondary_model]` edits with the
//! alias check. Kimi has no CLI to call.
//!
//! Main entry points: [`register`], [`unregister`], [`config_edits`],
//! [`update_registry`], [`remove_from_registry`] and [`plugin_root`].

use super::{Ctx, EditOutcome, Outcome, SubagentPrefs, server_binary, server_env};
use crate::config::{Desired, TomlDoc, apply_desired};
use crate::env::Env;
use crate::error::{Error, IoContext, Result};
use crate::json::{Json, detect_indent};
use crate::manifest::{Manifest, Registration};
use crate::ui::Ui;
use crate::util::{backup_path, now_iso, read_text_opt, write_atomic};
use std::path::{Path, PathBuf};

/// The plugin id and folder name.
pub const PLUGIN_ID: &str = "slate-agent-kit-mcp";

/// `<home>/plugins/managed/slate-agent-kit-mcp`.
pub fn plugin_root(home: &Path) -> PathBuf {
    home.join("plugins").join("managed").join(PLUGIN_ID)
}

/// `<home>/plugins/installed.json`.
pub fn registry_path(home: &Path) -> PathBuf {
    home.join("plugins").join("installed.json")
}

fn obj(members: Vec<(&str, Json)>) -> Json {
    Json::Object(
        members
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

fn strings(items: &[&str]) -> Json {
    Json::Array(items.iter().map(|s| Json::str(*s)).collect())
}

/// The content of `kimi.plugin.json` for the servers in `ctx`.
pub fn plugin_manifest(env: &Env, ctx: &Ctx) -> Json {
    let root = plugin_root(&ctx.home);
    let mut servers = Vec::new();
    for name in &ctx.servers {
        let env_members: Vec<(String, Json)> = server_env(ctx, name)
            .into_iter()
            .map(|(k, v)| (k, Json::String(v)))
            .collect();
        servers.push((
            name.clone(),
            obj(vec![
                (
                    "command",
                    Json::String(
                        server_binary(env, &ctx.bin_dir, name)
                            .to_string_lossy()
                            .into_owned(),
                    ),
                ),
                ("args", Json::Array(Vec::new())),
                ("cwd", Json::String(root.to_string_lossy().into_owned())),
                ("env", Json::Object(env_members)),
            ]),
        ));
    }
    obj(vec![
        ("name", Json::str(PLUGIN_ID)),
        ("version", Json::String(ctx.version.clone())),
        (
            "description",
            Json::str("Shared Slate Agent Kit MCP servers for Kimi Code."),
        ),
        (
            "keywords",
            strings(&["slate-agent-kit", "mcp", "aside", "dispatch", "palette"]),
        ),
        ("mcpServers", Json::Object(servers)),
        (
            "interface",
            obj(vec![
                ("displayName", Json::str("Slate Agent Kit MCP")),
                (
                    "shortDescription",
                    Json::str(
                        "aside read-only consultation, dispatch execution delegation and palette planning documents.",
                    ),
                ),
                ("developerName", Json::str("Slate Agent Kit")),
            ]),
        ),
    ])
}

/// The content of the plugin's `SKILL.md`.
pub fn plugin_skill(servers: &[String]) -> String {
    let has = |s: &str| servers.iter().any(|x| x == s);
    let mut lines = vec![
        "# Slate Agent Kit MCP".to_string(),
        String::new(),
        "This local plugin exposes the shared Slate Agent Kit MCP servers to Kimi Code."
            .to_string(),
        String::new(),
    ];
    if has("aside") {
        lines.push("- aside tools are read-only consultation tools.".into());
    }
    if has("dispatch") {
        lines.push("- dispatch tools are write-capable execution delegation tools and follow the dispatch level in the dispatch prefs.".into());
    }
    if has("palette") {
        lines.push("- palette tools read and write the project's planning documents; the read-only ones are safe to call freely.".into());
    }
    lines.push(String::new());
    lines.push("Expected MCP tool prefixes are harness-generated from this plugin id and server name, for example:".into());
    lines.push(String::new());
    for (server, tool) in [
        ("aside", "aside_list"),
        ("aside", "aside_codex"),
        ("aside", "aside_copilot"),
        ("aside", "aside_claude"),
        ("dispatch", "dispatch_submit"),
        ("dispatch", "dispatch_status"),
    ] {
        if has(server) {
            lines.push(format!("- `mcp__plugin-{PLUGIN_ID}_{server}__{tool}`"));
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

/// Result of updating the registry text.
#[derive(Debug)]
pub struct RegistryUpdate {
    /// The new registry text.
    pub text: String,
    /// The old text when it could not be read; the caller backs it up.
    pub unreadable: Option<String>,
}

fn fresh_registry() -> Json {
    obj(vec![
        ("version", Json::Number("1".into())),
        ("plugins", Json::Array(Vec::new())),
    ])
}

/// Adds or updates the plugin's entry in the registry, keeping every other entry.
///
/// An unreadable registry is returned in `unreadable` so it can be backed up
/// before the rebuilt one replaces it.
pub fn update_registry(existing: Option<&str>, root: &Path, now: &str) -> RegistryUpdate {
    let mut unreadable = None;
    let mut indent = "  ".to_string();
    let mut registry = match existing {
        None => fresh_registry(),
        Some(text) => match Json::parse(text) {
            Ok(j) if j.is_object() && matches!(j.get("plugins"), Some(Json::Array(_))) => {
                indent = detect_indent(text);
                j
            }
            _ => {
                unreadable = Some(text.to_string());
                fresh_registry()
            }
        },
    };
    let entry_fields = |installed_at: Json| -> Vec<(&'static str, Json)> {
        vec![
            ("root", Json::String(root.to_string_lossy().into_owned())),
            ("source", Json::str("local")),
            ("enabled", Json::Bool(true)),
            ("installedAt", installed_at),
            ("updatedAt", Json::str(now)),
            ("originalSource", Json::str("local:slate-agent-kit")),
        ]
    };
    if let Some(list) = registry.get_mut("plugins").and_then(Json::as_array_mut) {
        let pos = list
            .iter()
            .position(|p| p.get("id").and_then(Json::as_str) == Some(PLUGIN_ID));
        match pos {
            Some(i) => {
                let installed_at = list[i]
                    .get("installedAt")
                    .cloned()
                    .unwrap_or_else(|| Json::str(now));
                for (k, v) in entry_fields(installed_at) {
                    list[i].set(k, v);
                }
            }
            None => {
                let mut members = vec![("id", Json::str(PLUGIN_ID))];
                members.extend(entry_fields(Json::str(now)));
                list.push(obj(members));
            }
        }
    }
    RegistryUpdate {
        text: registry.to_pretty(&indent),
        unreadable,
    }
}

/// Removes the plugin's entry from the registry text. Returns `None` when the text is unreadable.
pub fn remove_from_registry(text: &str) -> Option<String> {
    let mut registry = Json::parse(text).ok()?;
    let indent = detect_indent(text);
    registry
        .get_mut("plugins")?
        .as_array_mut()?
        .retain(|p| p.get("id").and_then(Json::as_str) != Some(PLUGIN_ID));
    Some(registry.to_pretty(&indent))
}

/// Writes the plugin folder and registers it.
pub fn register(env: &Env, ui: &mut Ui, ctx: &Ctx) -> Result<Outcome> {
    let mut out = Outcome::default();
    let root = plugin_root(&ctx.home);
    ui.detail(&format!("writing the plugin {}", root.display()));
    std::fs::create_dir_all(ctx.state_home())
        .ctx(|| format!("creating {}", ctx.state_home().display()))?;
    let manifest = plugin_manifest(env, ctx).to_pretty("  ");
    write_atomic(&root.join("kimi.plugin.json"), manifest.as_bytes())?;
    write_atomic(
        &root.join("SKILL.md"),
        plugin_skill(&ctx.servers).as_bytes(),
    )?;
    let registry = registry_path(&ctx.home);
    let existing = read_text_opt(&registry)?;
    let update = update_registry(existing.as_deref(), &root, &now_iso());
    if let Some(bad) = &update.unreadable {
        let backup = backup_path(&registry);
        write_atomic(&backup, bad.as_bytes())?;
        out.notes.push(format!(
            "{} was unreadable; it was backed up to {} and rebuilt, so other plugin entries in it could not be kept",
            registry.display(),
            backup.display()
        ));
        out.backups.push((registry.clone(), backup));
    }
    write_atomic(&registry, update.text.as_bytes())?;
    if ctx.roots.is_empty() {
        out.notes.push(
            "no workspace roots were given: Kimi starts plugin servers outside any project, so dispatch and palette reject every project until you run `slate-setup configure --roots <workspace root>`"
                .to_string(),
        );
    }
    out.registered.push(Registration {
        kind: "kimi-plugin".into(),
        harness: "kimi".into(),
        server: String::new(),
        path: Some(root),
    });
    out.servers_done = ctx.servers.clone();
    Ok(out)
}

/// Removes the plugin entry and folder.
///
/// `registry_created` says the installer created `installed.json`; it is then
/// deleted once no plugin is left in it.
pub fn unregister(ui: &mut Ui, home: &Path, registry_created: bool) -> Result<Outcome> {
    let mut out = Outcome::default();
    let registry = registry_path(home);
    if let Some(text) = read_text_opt(&registry)? {
        match remove_from_registry(&text) {
            Some(new) => {
                let empty = Json::parse(&new)
                    .ok()
                    .and_then(|j| j.get("plugins").map(|p| p.is_empty_container()))
                    .unwrap_or(false);
                if registry_created && empty {
                    std::fs::remove_file(&registry)
                        .ctx(|| format!("removing {}", registry.display()))?;
                } else {
                    write_atomic(&registry, new.as_bytes())?;
                }
            }
            None => out.notes.push(format!(
                "{} is unreadable; the plugin entry was not removed from it",
                registry.display()
            )),
        }
    }
    let root = plugin_root(home);
    if root.exists() {
        ui.detail(&format!("removing {}", root.display()));
        std::fs::remove_dir_all(&root).ctx(|| format!("removing {}", root.display()))?;
    }
    out.servers_done.push(PLUGIN_ID.to_string());
    Ok(out)
}

/// The aliases under `[models]` of a Kimi configuration.
pub fn model_aliases(doc: &TomlDoc) -> Vec<String> {
    doc.keys_at(&["models"])
}

/// Edits `config.toml`: `[secondary_model]` `default_model` and `default_effort`.
///
/// The model must be an alias defined under `[models]`; an unknown alias makes
/// Kimi fail at startup, so it is refused. `force` and the `models` pool are
/// never written. The effort is written only together with a model.
pub fn config_edits(
    doc: &mut TomlDoc,
    file: &Path,
    sub: Option<&SubagentPrefs>,
    manifest: &Manifest,
) -> EditOutcome {
    let mut out = EditOutcome::default();
    let Some(sub) = sub else {
        return out;
    };
    if !sub.model.is_empty() {
        let aliases = model_aliases(doc);
        if !aliases.iter().any(|a| a == &sub.model) {
            let detail = if aliases.is_empty() {
                "the configuration defines no [models] aliases".to_string()
            } else {
                format!("available aliases: {}", aliases.join(", "))
            };
            out.refusals.push(
                Error::config(format!(
                    "`{}` is not an alias under [models] in {}; Kimi fails at startup on an unknown secondary model ({detail})",
                    sub.model,
                    file.display()
                ))
                .with_state("secondary_model was not changed")
                .with_fix("set `Model` in the subagent prefs to one of the aliases, or leave it blank"),
            );
            return out;
        }
    }
    let effort = if sub.model.is_empty() {
        ""
    } else {
        sub.effort.as_str()
    };
    for (key, value) in [
        ("default_model", sub.model.as_str()),
        ("default_effort", effort),
    ] {
        let path = ["secondary_model", key];
        let desired = if value.is_empty() {
            Desired::Unset
        } else {
            Desired::Set(Json::str(value))
        };
        let mut work = doc.clone();
        match apply_desired(
            &mut work,
            &path,
            &desired,
            manifest.config_record(file, &path),
        ) {
            Ok((change, undone)) => {
                *doc = work;
                out.changes.extend(change);
                if undone.is_some() {
                    out.undone
                        .push(path.iter().map(|s| s.to_string()).collect());
                }
            }
            Err(e) => out.refusals.push(e.with_fix(format!(
                "make [secondary_model] in {} a table and re-run",
                file.display()
            ))),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::Harness;
    use crate::harness::test_ctx;

    fn ctx(home: &str, default_home: &str) -> Ctx {
        test_ctx(Harness::Kimi, home, default_home)
    }

    #[test]
    fn plugin_manifest_has_the_old_fields_plus_palette_and_the_kit_version() {
        let env = Env::for_home("/h");
        let mut c = ctx("/x/kimi", "/h/.kimi-code");
        c.roots = vec!["/w".into()];
        let m = plugin_manifest(&env, &c);
        assert_eq!(
            m.get("name").and_then(Json::as_str),
            Some("slate-agent-kit-mcp")
        );
        assert_eq!(m.get("version").and_then(Json::as_str), Some("13.0.0"));
        let servers = m.get("mcpServers").unwrap();
        let aside = servers.get("aside").unwrap();
        assert_eq!(
            aside
                .get("env")
                .unwrap()
                .get("ASIDE_HARNESS")
                .and_then(Json::as_str),
            Some("kimi")
        );
        assert_eq!(
            aside
                .get("env")
                .unwrap()
                .get("KIMI_CODE_HOME")
                .and_then(Json::as_str),
            Some("/x/kimi")
        );
        assert_eq!(aside.get("args"), Some(&Json::Array(vec![])));
        let cwd = aside.get("cwd").and_then(Json::as_str).unwrap();
        assert!(
            Path::new(cwd).ends_with(
                Path::new("plugins")
                    .join("managed")
                    .join("slate-agent-kit-mcp")
            )
        );
        let dispatch_env = servers.get("dispatch").unwrap().get("env").unwrap();
        assert!(dispatch_env.get("SLATE_AGENT_STATE_HOME").is_some());
        assert_eq!(
            dispatch_env
                .get("DISPATCH_EXTRA_ROOTS")
                .and_then(Json::as_str),
            Some("/w")
        );
        assert_eq!(
            servers
                .get("palette")
                .unwrap()
                .get("env")
                .unwrap()
                .get("PALETTE_EXTRA_ROOTS")
                .and_then(Json::as_str),
            Some("/w")
        );
        // Member order follows the old writer.
        let keys: Vec<&str> = match &m {
            Json::Object(o) => o.iter().map(|(k, _)| k.as_str()).collect(),
            _ => vec![],
        };
        assert_eq!(
            keys,
            [
                "name",
                "version",
                "description",
                "keywords",
                "mcpServers",
                "interface"
            ]
        );
    }

    #[test]
    fn default_home_omits_kimi_code_home() {
        let env = Env::for_home("/h");
        let m = plugin_manifest(&env, &ctx("/h/.kimi-code", "/h/.kimi-code"));
        assert!(
            m.get("mcpServers")
                .unwrap()
                .get("aside")
                .unwrap()
                .get("env")
                .unwrap()
                .get("KIMI_CODE_HOME")
                .is_none()
        );
    }

    #[test]
    fn registry_gets_a_new_entry_and_keeps_others() {
        let existing = "{\n  \"version\": 1,\n  \"plugins\": [\n    {\"id\": \"other\", \"root\": \"/o\", \"extra\": [1, 2]}\n  ],\n  \"note\": \"x\"\n}\n";
        let u = update_registry(Some(existing), Path::new("/r"), "2026-09-29T00:00:00Z");
        assert!(u.unreadable.is_none());
        let j = Json::parse(&u.text).unwrap();
        let plugins = j.get("plugins").unwrap().as_array().unwrap();
        assert_eq!(plugins.len(), 2);
        assert_eq!(plugins[0].get("extra").unwrap().to_compact(), "[1,2]");
        assert_eq!(plugins[1].get("id").and_then(Json::as_str), Some(PLUGIN_ID));
        assert_eq!(
            plugins[1].get("installedAt").and_then(Json::as_str),
            Some("2026-09-29T00:00:00Z")
        );
        assert_eq!(j.get("note").and_then(Json::as_str), Some("x"));
    }

    #[test]
    fn registry_update_keeps_installed_at_position_and_unknown_fields() {
        let existing = "{\"version\":1,\"plugins\":[{\"id\":\"slate-agent-kit-mcp\",\"root\":\"/old\",\"installedAt\":\"2026-01-01T00:00:00Z\",\"custom\":true},{\"id\":\"z\"}]}";
        let u = update_registry(Some(existing), Path::new("/new"), "2026-09-29T00:00:00Z");
        let j = Json::parse(&u.text).unwrap();
        let plugins = j.get("plugins").unwrap().as_array().unwrap();
        assert_eq!(plugins[0].get("root").and_then(Json::as_str), Some("/new"));
        assert_eq!(
            plugins[0].get("installedAt").and_then(Json::as_str),
            Some("2026-01-01T00:00:00Z")
        );
        assert_eq!(
            plugins[0].get("updatedAt").and_then(Json::as_str),
            Some("2026-09-29T00:00:00Z")
        );
        assert_eq!(plugins[0].get("custom"), Some(&Json::Bool(true)));
        assert_eq!(plugins[1].get("id").and_then(Json::as_str), Some("z"));
    }

    #[test]
    fn an_unreadable_registry_is_reported_for_backup_and_rebuilt() {
        let u = update_registry(Some("{ not json"), Path::new("/r"), "now");
        assert_eq!(u.unreadable.as_deref(), Some("{ not json"));
        let j = Json::parse(&u.text).unwrap();
        assert_eq!(j.get("plugins").unwrap().as_array().unwrap().len(), 1);
        let u = update_registry(Some("{\"plugins\": 3}"), Path::new("/r"), "now");
        assert!(u.unreadable.is_some());
    }

    #[test]
    fn removal_keeps_every_other_entry() {
        let text = "{\"version\":1,\"plugins\":[{\"id\":\"a\"},{\"id\":\"slate-agent-kit-mcp\"},{\"id\":\"b\"}]}";
        let new = remove_from_registry(text).unwrap();
        let j = Json::parse(&new).unwrap();
        let ids: Vec<_> = j
            .get("plugins")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.get("id").and_then(Json::as_str).unwrap().to_string())
            .collect();
        assert_eq!(ids, ["a", "b"]);
        assert!(remove_from_registry("garbage").is_none());
    }

    const KIMI_CONFIG: &str = "# kimi\ndefault_model = \"k3\"\n\n[models.k3]\nprovider = \"kimi\"\n\n[models.\"k2-thinking\"]\nprovider = \"kimi\"\n\n[secondary_model]\nforce = false # never touched\n";

    #[test]
    fn an_unknown_alias_is_refused_and_nothing_changes() {
        let file = Path::new("/h/config.toml");
        let mut doc = TomlDoc::parse(KIMI_CONFIG).unwrap();
        let out = config_edits(
            &mut doc,
            file,
            Some(&SubagentPrefs {
                model: "primary".into(),
                effort: "high".into(),
            }),
            &Manifest::new("k"),
        );
        assert_eq!(out.refusals.len(), 1);
        assert!(out.refusals[0].to_string().contains("k3, k2-thinking"));
        assert!(out.changes.is_empty());
        assert_eq!(doc.render(), KIMI_CONFIG);
    }

    #[test]
    fn no_models_table_means_the_step_is_refused_with_an_explanation() {
        let file = Path::new("/h/config.toml");
        let mut doc = TomlDoc::parse("default_model = \"x\"\n").unwrap();
        let out = config_edits(
            &mut doc,
            file,
            Some(&SubagentPrefs {
                model: "x".into(),
                effort: String::new(),
            }),
            &Manifest::new("k"),
        );
        assert!(
            out.refusals[0]
                .to_string()
                .contains("defines no [models] aliases")
        );
    }

    #[test]
    fn a_known_alias_is_written_with_the_effort_and_force_is_untouched() {
        let file = Path::new("/h/config.toml");
        let mut doc = TomlDoc::parse(KIMI_CONFIG).unwrap();
        let out = config_edits(
            &mut doc,
            file,
            Some(&SubagentPrefs {
                model: "k2-thinking".into(),
                effort: "high".into(),
            }),
            &Manifest::new("k"),
        );
        assert!(out.refusals.is_empty());
        assert_eq!(out.changes.len(), 2);
        let text = doc.render();
        assert!(text.contains("[secondary_model]\nforce = false # never touched\ndefault_model = \"k2-thinking\"\ndefault_effort = \"high\"\n"), "{text}");
        assert!(text.starts_with("# kimi\ndefault_model = \"k3\"\n"));
        let parsed: toml::Table = toml::from_str(&text).unwrap();
        assert_eq!(
            parsed["secondary_model"]["default_model"].as_str(),
            Some("k2-thinking")
        );
        assert!(parsed["secondary_model"].get("models").is_none());
    }

    #[test]
    fn effort_without_a_model_is_not_written() {
        let file = Path::new("/h/config.toml");
        let mut doc = TomlDoc::parse(KIMI_CONFIG).unwrap();
        let out = config_edits(
            &mut doc,
            file,
            Some(&SubagentPrefs {
                model: String::new(),
                effort: "high".into(),
            }),
            &Manifest::new("k"),
        );
        assert!(out.changes.is_empty());
        assert_eq!(doc.render(), KIMI_CONFIG);
    }
}
