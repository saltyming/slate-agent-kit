//! Retiring the servers an earlier release installed and the new kit no longer lists.
//!
//! Owns finding those servers in the previous manifest (its registrations and
//! binaries, and for Kimi the plugin it wrote, since the manifest records the
//! plugin and not its servers), the plan for each one, and carrying it out:
//! restore the configuration keys recorded for that server, unregister it,
//! remove its binary unless another kit's manifest lists it, and drop from the
//! manifest only what succeeded. It never touches a server the new kit lists.
//!
//! Main entry points: [`plan`], [`apply`], [`Plan`] and [`RetiredServer`].

use crate::binaries::{self, exe_name};
use crate::config::{ConfigRecord, Restored};
use crate::env::{Env, Harness};
use crate::error::{IoContext, Result};
use crate::harness::{self, agy, claude, codex, kimi};
use crate::kit::Kit;
use crate::manifest::{Manifest, manifest_path, other_manifests_listing};
use crate::report::Report;
use crate::serve::restore_records;
use crate::ui::Ui;
use std::fs;
use std::path::PathBuf;

/// What happens to a retired server's binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryPlan {
    /// No other kit's manifest lists it: it is removed.
    Remove(PathBuf),
    /// Another kit's manifest lists it, or a server that needs it: it stays.
    Keep {
        /// The binary.
        path: PathBuf,
        /// The manifest that lists it.
        by: PathBuf,
    },
    /// The manifest lists it but the file is gone already.
    Missing(PathBuf),
}

impl BinaryPlan {
    /// The binary path.
    pub fn path(&self) -> &PathBuf {
        match self {
            BinaryPlan::Remove(p) | BinaryPlan::Missing(p) => p,
            BinaryPlan::Keep { path, .. } => path,
        }
    }
}

/// One server the earlier release installed and the new kit no longer lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetiredServer {
    /// Server name.
    pub name: String,
    /// The manifest records its registration with the harness CLI (Claude Code, Codex, agy).
    pub registered: bool,
    /// The Kimi plugin the installer wrote lists it.
    pub in_plugin: bool,
    /// Its binary, when the manifest lists one.
    pub binary: Option<BinaryPlan>,
}

/// The retirement step of an install or configure run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    /// The servers to retire.
    pub servers: Vec<RetiredServer>,
    /// The recorded configuration keys that belong to them, in manifest order. A
    /// list key shared with other servers carries only the entries of the retired
    /// ones in `added`, so restoring it removes only those.
    pub keys: Vec<ConfigRecord>,
}

impl Plan {
    /// True when nothing is retired.
    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }
}

/// The part of `rec` that belongs to `servers`, if any.
///
/// A key under `mcp_servers.<server>` belongs to that server whole. A list key
/// belongs to a server through the entries the installer added for it: the
/// tool namespace `mcp__<server>` (Codex code mode), a tool permission
/// `mcp__<server>__<tool>` (Claude Code), or an approval `mcp(<server>)` or
/// `mcp(<server>/<tool>)` (agy).
pub fn server_part(rec: &ConfigRecord, servers: &[String]) -> Option<ConfigRecord> {
    if rec.path.len() >= 2 && rec.path[0] == "mcp_servers" && servers.contains(&rec.path[1]) {
        return Some(rec.clone());
    }
    let mine: Vec<String> = rec
        .added
        .iter()
        .filter(|a| {
            servers.iter().any(|s| {
                let ns = format!("mcp__{s}");
                a.as_str() == ns
                    || a.starts_with(&format!("{ns}__"))
                    || a.as_str() == format!("mcp({s})")
                    || a.starts_with(&format!("mcp({s}/"))
            })
        })
        .cloned()
        .collect();
    (!mine.is_empty()).then(|| ConfigRecord {
        added: mine,
        ..rec.clone()
    })
}

/// Builds the retirement plan from the previous manifest.
pub fn plan(env: &Env, kit: &Kit) -> Plan {
    let Some(prior) = &kit.prior else {
        return Plan::default();
    };
    let keep = kit.servers();
    let platform = env.platform();
    let harness = kit.harness.name();
    let mut names: Vec<String> = Vec::new();
    let add = |n: &str, names: &mut Vec<String>| {
        if !keep.iter().any(|k| k == n) && !names.iter().any(|x| x == n) {
            names.push(n.to_string());
        }
    };
    for r in &prior.registrations {
        if r.kind == "mcp" && r.harness == harness {
            add(&r.server, &mut names);
        }
    }
    let owns_plugin =
        kit.harness == Harness::Kimi && prior.registrations.iter().any(|r| r.kind == "kimi-plugin");
    let plugin = if owns_plugin {
        kimi::plugin_servers(&kit.home)
    } else {
        Vec::new()
    };
    for s in &plugin {
        add(s, &mut names);
    }
    // A binary the manifest lists is a server's or the guard's; one whose
    // server is gone is retired even when its registration never succeeded.
    for b in &prior.binaries {
        if binaries::is_guard(b) {
            continue;
        }
        if let Some(n) = b.file_name().map(|n| n.to_string_lossy().into_owned())
            && let Some(server) = binary_server_name(&platform, &n)
        {
            add(&server, &mut names);
        }
    }
    if names.is_empty() {
        return Plan::default();
    }
    let own = manifest_path(&kit.layout.manifest_dir, kit.name());
    let homes = crate::uninstall::all_manifest_dirs(env, kit);
    let servers = names
        .iter()
        .map(|name| {
            let exe = exe_name(&platform, name);
            let binary = prior
                .binaries
                .iter()
                .find(|b| b.file_name().is_some_and(|f| f.to_string_lossy() == exe))
                .map(|b| {
                    if !b.exists() {
                        return BinaryPlan::Missing(b.clone());
                    }
                    let keepers = binaries::keepers_of(env, b);
                    match other_manifests_listing(&homes, &own, &keepers)
                        .into_iter()
                        .next()
                    {
                        Some(by) => BinaryPlan::Keep {
                            path: b.clone(),
                            by,
                        },
                        None => BinaryPlan::Remove(b.clone()),
                    }
                });
            RetiredServer {
                name: name.clone(),
                registered: prior
                    .registrations
                    .iter()
                    .any(|r| r.kind == "mcp" && r.harness == harness && &r.server == name),
                in_plugin: plugin.contains(name),
                binary,
            }
        })
        .collect();
    let keys = prior
        .config
        .iter()
        .filter_map(|r| server_part(r, &names))
        .collect();
    Plan { servers, keys }
}

/// The server name of a binary file name on `platform`, without the Windows suffix.
fn binary_server_name(platform: &str, file: &str) -> Option<String> {
    let name = if platform.contains("windows") {
        file.strip_suffix(".exe").unwrap_or(file)
    } else {
        file
    };
    (!name.is_empty()).then(|| name.to_string())
}

/// Prints the retirement part of the summary.
pub fn print_summary(ui: &mut Ui, plan: &Plan) {
    if plan.is_empty() {
        return;
    }
    ui.blank();
    ui.line("  Servers the new release no longer ships");
    for s in &plan.servers {
        let how = if s.registered {
            "unregister"
        } else if s.in_plugin {
            "remove from the Kimi plugin"
        } else {
            "forget (no registration recorded)"
        };
        ui.line(&format!("    {how} {}", s.name));
        match &s.binary {
            Some(BinaryPlan::Remove(p)) => ui.line(&format!("    remove {}", p.display())),
            Some(BinaryPlan::Keep { path, by }) => ui.line(&format!(
                "    keep {} (also listed by {})",
                path.display(),
                by.display()
            )),
            Some(BinaryPlan::Missing(_)) | None => {}
        }
    }
    for k in &plan.keys {
        let what = if k.added.is_empty() {
            String::new()
        } else {
            format!(" (removes {})", k.added.join(", "))
        };
        ui.line(&format!(
            "    restore {} in {}{what}, only while it holds what the installer wrote",
            k.path.join("."),
            k.file.display()
        ));
    }
}

/// Updates `manifest` for a settled key: forgets a whole key, or drops the retired entries of a list key.
fn settle_key(manifest: &mut Manifest, part: &ConfigRecord) {
    if part.added.is_empty() {
        manifest.forget_config(&part.file, &part.path);
        return;
    }
    let mut emptied = false;
    if let Some(rec) = manifest
        .config
        .iter_mut()
        .find(|c| c.file == part.file && c.path == part.path)
    {
        rec.added.retain(|a| !part.added.contains(a));
        emptied = rec.added.is_empty();
    }
    if emptied {
        manifest.forget_config(&part.file, &part.path);
    }
}

/// The command that removes `server` by hand, for a removal the CLI refused.
fn manual_remove(env: &Env, harness: Harness, server: &str) -> String {
    let program = env
        .cli_program(harness)
        .unwrap_or_else(|| harness.name().to_string());
    match harness {
        Harness::Claude => format!("{program} {}", claude::remove_args(server).join(" ")),
        Harness::Agy => format!("{program} {}", agy::remove_args(server).join(" ")),
        _ => format!("{program} {}", codex::remove_args(server).join(" ")),
    }
}

/// Carries out the retirement plan, recording in `manifest` only what succeeded.
///
/// The keys are restored before the servers are unregistered, because `codex
/// mcp remove` deletes the whole `[mcp_servers.<server>]` table and a restore
/// afterwards would report every key as changed by the user. A removal the
/// harness CLI ran and refused (it also refuses a server that is not
/// registered) is a note with the command to run by hand, and the manifest no
/// longer records it, so the next run does not fail on it again.
pub fn apply(
    env: &Env,
    ui: &mut Ui,
    kit: &Kit,
    plan: &Plan,
    manifest: &mut Manifest,
    report: &mut Report,
) -> Result<()> {
    if plan.is_empty() {
        return Ok(());
    }
    ui.blank();
    ui.line("Servers the new release no longer ships");

    let mut files: Vec<PathBuf> = plan.keys.iter().map(|k| k.file.clone()).collect();
    files.sort();
    files.dedup();
    for file in files {
        let parts: Vec<&ConfigRecord> = plan.keys.iter().filter(|k| k.file == file).collect();
        let settled = restore_records(&file, &parts, report)?;
        for part in &parts {
            let Some((_, outcome)) = settled.iter().find(|(p, _)| *p == part.path) else {
                continue;
            };
            settle_key(manifest, part);
            let what = match outcome {
                Restored::Restored => "restored",
                Restored::Removed => "removed (it did not exist before)",
                Restored::Nothing => "already as before",
                Restored::Changed(_) => "left as you changed it",
            };
            ui.detail(&format!("{}: {what}", part.path.join(".")));
        }
    }

    let harness = kit.harness;
    let registered: Vec<String> = plan
        .servers
        .iter()
        .filter(|s| s.registered)
        .map(|s| s.name.clone())
        .collect();
    let mut gone: Vec<String> = plan
        .servers
        .iter()
        .filter(|s| !s.registered && !s.in_plugin)
        .map(|s| s.name.clone())
        .collect();
    if !registered.is_empty() {
        let outcome = match harness {
            Harness::Claude => claude::unregister(env, ui, &kit.home, &registered)?,
            Harness::Codex => codex::unregister(env, ui, &kit.home, &registered)?,
            Harness::Agy => agy::unregister(env, ui, &registered)?,
            Harness::Kimi => harness::Outcome::default(),
        };
        for n in outcome.notes {
            report.note(n);
        }
        for m in outcome.manual {
            report.manual(m);
        }
        for (server, message) in &outcome.refused {
            let why = if message.is_empty() {
                String::new()
            } else {
                format!(" ({message})")
            };
            report.note(format!(
                "`{}` did not succeed{why}; the manifest no longer records {server}. If {} still lists it, run that command by hand",
                manual_remove(env, harness, server),
                harness.product()
            ));
        }
        gone.extend(outcome.servers_done);
        gone.extend(outcome.refused.into_iter().map(|(s, _)| s));
    }
    let in_plugin: Vec<String> = plan
        .servers
        .iter()
        .filter(|s| s.in_plugin)
        .map(|s| s.name.clone())
        .collect();
    if !in_plugin.is_empty() {
        match kimi::retire_from_plugin(&kit.home, &in_plugin) {
            Ok(removed) => {
                // A server already missing from the plugin is gone as well.
                gone.extend(in_plugin.iter().cloned());
                if !removed.is_empty() {
                    ui.ok(&format!(
                        "removed {} from {}",
                        removed.join(", "),
                        kimi::plugin_root(&kit.home).display()
                    ));
                }
            }
            Err(e) => report.problems.push(e),
        }
    }
    manifest
        .registrations
        .retain(|r| !(r.kind == "mcp" && r.harness == harness.name() && gone.contains(&r.server)));

    for s in &plan.servers {
        let Some(binary) = &s.binary else {
            continue;
        };
        if !gone.contains(&s.name) {
            report.note(format!(
                "{} stays while {} is still registered; run the commands listed below, then `slate-setup configure`",
                binary.path().display(),
                s.name
            ));
            continue;
        }
        match binary {
            BinaryPlan::Remove(p) => match fs::remove_file(p)
                .ctx(|| format!("removing {}", p.display()))
            {
                Ok(()) => {
                    ui.ok(&format!("removed {}", p.display()));
                    manifest.binaries.retain(|b| b != p);
                }
                Err(e) => report.problems.push(
                    e.with_state(format!(
                        "{} is no longer registered; its binary stays",
                        s.name
                    ))
                    .with_fix("stop the harness that runs it and re-run `slate-setup configure`"),
                ),
            },
            BinaryPlan::Keep { path, by } => {
                report.note(format!(
                    "{} was kept: {} also lists it",
                    path.display(),
                    by.display()
                ));
                manifest.binaries.retain(|b| b != path);
            }
            BinaryPlan::Missing(p) => manifest.binaries.retain(|b| b != p),
        }
    }
    let done: Vec<&str> = plan
        .servers
        .iter()
        .filter(|s| gone.contains(&s.name))
        .map(|s| s.name.as_str())
        .collect();
    if !done.is_empty() {
        report.done("retired", done.join(", "));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(path: &[&str], added: &[&str]) -> ConfigRecord {
        ConfigRecord {
            file: PathBuf::from("c.toml"),
            path: path.iter().map(|s| s.to_string()).collect(),
            previous: None,
            written: "x".into(),
            created_parents: 0,
            added: added.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn keys_belong_to_a_server_by_table_namespace_or_tool_permission() {
        let gone = vec!["aside".to_string()];
        let table = rec(&["mcp_servers", "aside", "tool_timeout_sec"], &[]);
        assert_eq!(server_part(&table, &gone), Some(table.clone()));
        let other_table = rec(&["mcp_servers", "asidex", "x"], &[]);
        assert_eq!(server_part(&other_table, &gone), None);
        let ns = rec(
            &["features", "code_mode", "excluded_tool_namespaces"],
            &["mcp__other", "mcp__aside"],
        );
        assert_eq!(
            server_part(&ns, &gone).unwrap().added,
            vec!["mcp__aside".to_string()]
        );
        let allow = rec(
            &["permissions", "allow"],
            &["mcp__palette__palette_status", "mcp__aside__aside_codex"],
        );
        assert_eq!(
            server_part(&allow, &gone).unwrap().added,
            vec!["mcp__aside__aside_codex".to_string()]
        );
        let subagent = rec(&["agents", "default_subagent_model"], &[]);
        assert_eq!(server_part(&subagent, &gone), None);
        let palette_only = rec(&["permissions", "allow"], &["mcp__palette__palette_status"]);
        assert_eq!(server_part(&palette_only, &gone), None);
        let agy_allow = rec(
            &["permissions", "allow"],
            &[
                "mcp(palette/palette_status)",
                "mcp(aside/aside_codex)",
                "mcp(aside)",
                "mcp(asidex/t)",
            ],
        );
        assert_eq!(
            server_part(&agy_allow, &gone).unwrap().added,
            vec![
                "mcp(aside/aside_codex)".to_string(),
                "mcp(aside)".to_string()
            ]
        );
    }

    #[test]
    fn a_settled_list_key_loses_only_the_retired_entries() {
        let mut m = Manifest::new("k");
        m.config.push(rec(
            &["features", "code_mode", "excluded_tool_namespaces"],
            &["mcp__other", "mcp__aside"],
        ));
        m.config
            .push(rec(&["mcp_servers", "aside", "tool_timeout_sec"], &[]));
        let gone = vec!["aside".to_string()];
        for r in m.config.clone() {
            settle_key(&mut m, &server_part(&r, &gone).unwrap());
        }
        assert_eq!(m.config.len(), 1);
        assert_eq!(m.config[0].added, vec!["mcp__other".to_string()]);
        let only = rec(
            &["features", "code_mode", "direct_only_tool_namespaces"],
            &["mcp__aside"],
        );
        m.config.push(only.clone());
        settle_key(&mut m, &server_part(&only, &gone).unwrap());
        assert_eq!(m.config.len(), 1, "an emptied list record is forgotten");
    }

    #[test]
    fn binary_names_drop_the_windows_suffix_only_on_windows() {
        assert_eq!(
            binary_server_name("x86_64-pc-windows-msvc", "aside.exe").as_deref(),
            Some("aside")
        );
        assert_eq!(
            binary_server_name("aarch64-apple-darwin", "aside").as_deref(),
            Some("aside")
        );
        assert_eq!(
            binary_server_name("aarch64-apple-darwin", "").as_deref(),
            None
        );
    }
}
