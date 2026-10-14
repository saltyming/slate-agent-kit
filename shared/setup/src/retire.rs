//! Retiring the servers an earlier release installed and the new kit no longer lists.
//!
//! Owns finding those servers in the previous manifest (its registrations and
//! binaries, and for Kimi the plugin it wrote, since the manifest records the
//! plugin and not its servers), the plan for each one, and carrying it out:
//! restore the configuration keys recorded for that server, unregister it,
//! remove its binary unless another kit's manifest lists it, and drop from the
//! manifest only what succeeded. It never touches a server the new
//! installation registers. The installation it runs for is a [`Context`]: a
//! kit's install and configure build one from the kit, and `slate-setup mcp`
//! builds one from its own manifest, so both retire the same way.
//!
//! Main entry points: [`plan`], [`apply`], [`Context`], [`Plan`] and
//! [`RetiredServer`].

use crate::binaries::{self, exe_name};
use crate::config::{ConfigRecord, Restored};
use crate::env::{Env, Harness};
use crate::error::{Error, Result};
use crate::harness::{self, agy, claude, codex, kimi};
use crate::kit::Kit;
use crate::layout::Layout;
use crate::manifest::{Manifest, manifest_path, other_manifests_listing};
use crate::report::Report;
use crate::serve::restore_records;
use crate::ui::Ui;
use std::fs;
use std::path::{Path, PathBuf};

/// The installation a retirement runs for.
///
/// Holds what the plan and the steps need to know about it, so a kit's install
/// and `slate-setup mcp` share one retirement: the manifest the earlier release
/// wrote, the servers that are registered now and so are never retired, the
/// harness and its home, and where manifests live, with the identity of the
/// caller's own so that it is not counted as another owner of a binary.
#[derive(Debug, Clone)]
pub struct Context {
    /// The manifest of the earlier installation, if any.
    pub prior: Option<Manifest>,
    /// The servers this installation registers; none of them is retired.
    pub keep: Vec<String>,
    /// The harness.
    pub harness: Harness,
    /// The harness home.
    pub home: PathBuf,
    /// The harness's manifest folder, searched with the other harnesses' for
    /// manifests that list a binary.
    pub manifest_dir: PathBuf,
    /// The caller's own manifest file, left out of that search.
    pub own_manifest: PathBuf,
    /// The command that runs the installation again, named in the advice for a
    /// step that could not finish.
    pub rerun: String,
    /// The other manifests the same run changes, each with the servers it
    /// keeps. A manifest listed here that lists a retired server's binary is
    /// retiring that server in this run too, so it does not keep the binary.
    pub siblings: Vec<Sibling>,
}

/// A manifest changed by the same run as the [`Context`] it belongs to, such as
/// the `mcp` manifest of another harness named by `--harness a,b`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sibling {
    /// The manifest file.
    pub manifest: PathBuf,
    /// The servers that run keeps for it; it retires the others.
    pub keep: Vec<String>,
}

impl Context {
    /// The context of a kit's install or configure.
    pub fn for_kit(kit: &Kit) -> Context {
        Context {
            prior: kit.prior.clone(),
            keep: kit.servers(),
            harness: kit.harness,
            home: kit.home.clone(),
            manifest_dir: kit.layout.manifest_dir.clone(),
            own_manifest: manifest_path(&kit.layout.manifest_dir, kit.name()),
            rerun: "slate-setup configure".to_string(),
            siblings: Vec::new(),
        }
    }

    /// The context of `slate-setup mcp` for `harness`: its own manifest
    /// ([`crate::mcp::MCP_KIT`]) in the layout's manifest folder, `prior` as it
    /// was loaded, and `keep`, the servers `mcp` registers. `others` are the
    /// `mcp` manifests of the other harnesses in the same run, which keep the
    /// same servers.
    pub fn for_mcp(
        harness: Harness,
        home: &Path,
        layout: &Layout,
        prior: Option<Manifest>,
        keep: &[String],
        others: &[PathBuf],
    ) -> Context {
        Context {
            prior,
            keep: keep.to_vec(),
            harness,
            home: home.to_path_buf(),
            manifest_dir: layout.manifest_dir.clone(),
            own_manifest: layout.manifest_path(crate::mcp::MCP_KIT),
            rerun: format!("slate-setup mcp --harness {}", harness.name()),
            siblings: others
                .iter()
                .map(|manifest| Sibling {
                    manifest: manifest.clone(),
                    keep: keep.to_vec(),
                })
                .collect(),
        }
    }

    /// True when `manifest` is a sibling that retires `server` in this run, so
    /// that its listing of the server's binary is not a reason to keep it.
    fn retires_alongside(&self, manifest: &Path, server: &str) -> bool {
        self.siblings.iter().any(|s| {
            crate::util::same_path(&s.manifest, manifest) && !s.keep.iter().any(|k| k == server)
        })
    }
}

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
pub fn plan(env: &Env, ctx: &Context) -> Plan {
    let Some(prior) = &ctx.prior else {
        return Plan::default();
    };
    let keep = &ctx.keep;
    let platform = env.platform();
    let harness = ctx.harness.name();
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
        ctx.harness == Harness::Kimi && prior.registrations.iter().any(|r| r.kind == "kimi-plugin");
    let plugin = if owns_plugin {
        kimi::plugin_servers(&ctx.home)
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
    let own = &ctx.own_manifest;
    let homes = crate::layout::manifest_dirs(env, std::slice::from_ref(&ctx.manifest_dir));
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
                    match other_manifests_listing(&homes, own, &keepers)
                        .into_iter()
                        .find(|m| !ctx.retires_alongside(m, name))
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
///
/// This is [`apply_servers`] followed by [`apply_binaries`] for one
/// installation. A run that retires from several manifests at once calls the
/// two itself, so that no binary goes while another harness of the run still
/// has its server registered.
pub fn apply(
    env: &Env,
    ui: &mut Ui,
    ctx: &Context,
    plan: &Plan,
    manifest: &mut Manifest,
    report: &mut Report,
) -> Result<()> {
    let gone = apply_servers(env, ui, ctx, plan, manifest, report)?;
    apply_binaries(ui, ctx, plan, &gone, &[], manifest, report);
    Ok(())
}

/// The binaries of the servers of `plan` that are not in `gone`: files that
/// other harnesses of the same run must not remove yet.
pub fn needed_binaries(plan: &Plan, gone: &[String]) -> Vec<PathBuf> {
    plan.servers
        .iter()
        .filter(|s| !gone.contains(&s.name))
        .filter_map(|s| s.binary.as_ref())
        .map(|b| b.path().clone())
        .collect()
}

/// The first part of [`apply`]: restores the recorded keys, unregisters the
/// servers (or removes them from the Kimi plugin) and drops their registrations
/// from `manifest`. Returns the servers that are gone from the harness; the
/// rest are still registered.
pub fn apply_servers(
    env: &Env,
    ui: &mut Ui,
    ctx: &Context,
    plan: &Plan,
    manifest: &mut Manifest,
    report: &mut Report,
) -> Result<Vec<String>> {
    if plan.is_empty() {
        return Ok(Vec::new());
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

    let harness = ctx.harness;
    let home = ctx.home.as_path();
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
            Harness::Claude => claude::unregister(env, ui, home, &registered)?,
            Harness::Codex => codex::unregister(env, ui, home, &registered)?,
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
        match kimi::retire_from_plugin(home, &in_plugin) {
            Ok(removed) => {
                // A server already missing from the plugin is gone as well.
                gone.extend(in_plugin.iter().cloned());
                if !removed.is_empty() {
                    ui.ok(&format!(
                        "removed {} from {}",
                        removed.join(", "),
                        kimi::plugin_root(home).display()
                    ));
                }
            }
            Err(e) => report.problems.push(e),
        }
    }
    manifest
        .registrations
        .retain(|r| !(r.kind == "mcp" && r.harness == harness.name() && gone.contains(&r.server)));
    Ok(gone)
}

/// The second part of [`apply`]: removes the binaries of the servers that are
/// `gone`, unless another manifest lists them, and drops them from `manifest`.
///
/// A binary of a server that is still registered stays recorded, as does one in
/// `held`, the files another harness of the same run still needs
/// ([`needed_binaries`]). A binary that is already absent when its removal
/// comes, because an earlier harness of the run removed it, counts as removed.
pub fn apply_binaries(
    ui: &mut Ui,
    ctx: &Context,
    plan: &Plan,
    gone: &[String],
    held: &[PathBuf],
    manifest: &mut Manifest,
    report: &mut Report,
) {
    for s in &plan.servers {
        let Some(binary) = &s.binary else {
            continue;
        };
        if !gone.contains(&s.name) {
            report.note(format!(
                "{} stays while {} is still registered; run the commands listed below, then `{}`",
                binary.path().display(),
                s.name,
                ctx.rerun
            ));
            continue;
        }
        match binary {
            BinaryPlan::Remove(p) if held.contains(p) => {
                report.note(format!(
                    "{} stays while another harness in this run still has {} registered; run the commands listed below, then `{}`",
                    p.display(),
                    s.name,
                    ctx.rerun
                ));
            }
            BinaryPlan::Remove(p) => match fs::remove_file(p) {
                Ok(()) => {
                    ui.ok(&format!("removed {}", p.display()));
                    manifest.binaries.retain(|b| b != p);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    ui.detail(&format!("{} was already removed", p.display()));
                    manifest.binaries.retain(|b| b != p);
                }
                Err(e) => report.problems.push(
                    Error::io(format!("removing {}", p.display()), e)
                        .with_state(format!(
                            "{} is no longer registered; its binary stays",
                            s.name
                        ))
                        .with_fix(format!(
                            "stop the harness that runs it and re-run `{}`",
                            ctx.rerun
                        )),
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Registration;

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
    fn the_mcp_context_names_its_own_manifest_and_the_command_that_repeats_it() {
        let home = Path::new("/h/.gemini");
        let layout = Layout::new(Harness::Agy, home);
        let keep = vec!["dispatch".to_string()];
        let ctx = Context::for_mcp(Harness::Agy, home, &layout, None, &keep, &[]);
        assert_eq!(
            ctx.own_manifest,
            layout
                .manifest_dir
                .join(".slate-agent-kit-mcp-manifest.toml")
        );
        assert_eq!(ctx.manifest_dir, layout.manifest_dir);
        assert_eq!(ctx.home, home);
        assert_eq!(ctx.keep, keep);
        assert_eq!(ctx.rerun, "slate-setup mcp --harness agy");
        assert!(
            plan(&Env::for_home("/h"), &ctx).is_empty(),
            "without a prior manifest there is nothing to retire"
        );
    }

    #[test]
    fn the_plan_retires_what_the_manifest_recorded_and_the_context_does_not_keep() {
        let mut prior = Manifest::new("slate-agent-kit-mcp");
        for server in ["aside", "dispatch"] {
            prior.record_registration(Registration {
                kind: "mcp".into(),
                harness: "claude".into(),
                server: server.into(),
                path: None,
            });
        }
        // Another harness's registration of a retired server is not this one's to retire.
        prior.record_registration(Registration {
            kind: "mcp".into(),
            harness: "codex".into(),
            server: "elsewhere".into(),
            path: None,
        });
        let home = Path::new("/h/.claude");
        let layout = Layout::new(Harness::Claude, home);
        let keep = vec!["dispatch".to_string(), "palette".to_string()];
        let ctx = Context::for_mcp(Harness::Claude, home, &layout, Some(prior), &keep, &[]);
        let planned = plan(&Env::for_home("/h"), &ctx);
        assert_eq!(planned.servers.len(), 1);
        assert_eq!(planned.servers[0].name, "aside");
        assert!(planned.servers[0].registered);
        assert_eq!(
            planned.servers[0].binary, None,
            "no binary is recorded for it, so none is removed"
        );
    }

    /// A home with a Claude `mcp` manifest that recorded `aside` and its
    /// binary, `Codex`'s `mcp` manifest as a sibling that lists the binary too,
    /// and the context of the Claude run over both.
    struct Run {
        _dir: tempfile::TempDir,
        env: Env,
        ctx: Context,
        binary: PathBuf,
        sibling: PathBuf,
    }

    fn run_over_two_manifests() -> Run {
        let dir = tempfile::tempdir().unwrap();
        let env = Env::for_home(dir.path());
        let binary = dir.path().join(exe_name(&env.platform(), "aside"));
        fs::write(&binary, "aside").unwrap();
        let mut prior = Manifest::new(crate::mcp::MCP_KIT);
        prior.record_binary(&binary);
        let claude_home = env.default_home(Harness::Claude);
        let layout = Layout::new(Harness::Claude, &claude_home);
        let codex = Layout::new(Harness::Codex, &env.default_home(Harness::Codex));
        fs::create_dir_all(&codex.manifest_dir).unwrap();
        let mut theirs = Manifest::new(crate::mcp::MCP_KIT);
        theirs.record_binary(&binary);
        theirs.save(&codex.manifest_dir).unwrap();
        let sibling = codex.manifest_path(crate::mcp::MCP_KIT);
        let keep = vec!["dispatch".to_string()];
        let ctx = Context::for_mcp(
            Harness::Claude,
            &claude_home,
            &layout,
            Some(prior),
            &keep,
            std::slice::from_ref(&sibling),
        );
        Run {
            _dir: dir,
            env,
            ctx,
            binary,
            sibling,
        }
    }

    fn the_binary_plan(run: &Run) -> BinaryPlan {
        let planned = plan(&run.env, &run.ctx);
        assert_eq!(planned.servers.len(), 1);
        planned.servers[0].binary.clone().expect("a binary plan")
    }

    #[test]
    fn a_binary_only_the_runs_own_manifests_list_is_removed() {
        let run = run_over_two_manifests();
        assert_eq!(
            the_binary_plan(&run),
            BinaryPlan::Remove(run.binary.clone())
        );
    }

    #[test]
    fn a_manifest_outside_the_run_keeps_the_binary() {
        let mut run = run_over_two_manifests();
        // The same manifest, but this run does not change it.
        run.ctx.siblings.clear();
        assert_eq!(
            the_binary_plan(&run),
            BinaryPlan::Keep {
                path: run.binary.clone(),
                by: run.sibling.clone()
            }
        );
    }

    #[test]
    fn a_kit_manifest_beside_the_siblings_keeps_the_binary() {
        let run = run_over_two_manifests();
        let kit_dir = run.ctx.manifest_dir.clone();
        let mut kit = Manifest::new("some-agent-kit");
        kit.record_binary(&run.binary);
        kit.save(&kit_dir).unwrap();
        assert_eq!(
            the_binary_plan(&run),
            BinaryPlan::Keep {
                path: run.binary.clone(),
                by: manifest_path(&kit_dir, "some-agent-kit")
            }
        );
    }

    #[test]
    fn a_sibling_that_keeps_the_server_keeps_the_binary() {
        let mut run = run_over_two_manifests();
        run.ctx.siblings[0].keep.push("aside".to_string());
        assert_eq!(
            the_binary_plan(&run),
            BinaryPlan::Keep {
                path: run.binary.clone(),
                by: run.sibling.clone()
            }
        );
    }

    #[test]
    fn a_binary_another_harness_of_the_run_still_needs_is_held_and_one_already_gone_counts_as_removed()
     {
        let dir = tempfile::tempdir().unwrap();
        let present = dir.path().join("present");
        let absent = dir.path().join("absent");
        fs::write(&present, "x").unwrap();
        let servers = vec![
            RetiredServer {
                name: "held".into(),
                registered: true,
                in_plugin: false,
                binary: Some(BinaryPlan::Remove(present.clone())),
            },
            RetiredServer {
                name: "gone".into(),
                registered: true,
                in_plugin: false,
                binary: Some(BinaryPlan::Remove(absent.clone())),
            },
        ];
        let planned = Plan {
            servers,
            keys: Vec::new(),
        };
        let gone = vec!["held".to_string(), "gone".to_string()];
        assert!(needed_binaries(&planned, &gone).is_empty());
        assert_eq!(
            needed_binaries(&planned, &["gone".to_string()]),
            vec![present.clone()]
        );

        let home = Path::new("/h/.claude");
        let layout = Layout::new(Harness::Claude, home);
        let ctx = Context::for_mcp(Harness::Claude, home, &layout, None, &[], &[]);
        let mut manifest = Manifest::new(crate::mcp::MCP_KIT);
        manifest.record_binary(&present);
        manifest.record_binary(&absent);
        let mut report = Report::default();
        let mut ui = Ui::captured(crate::ui::new_sink());
        apply_binaries(
            &mut ui,
            &ctx,
            &planned,
            &gone,
            std::slice::from_ref(&present),
            &mut manifest,
            &mut report,
        );
        assert!(report.problems.is_empty(), "{:?}", report.problems);
        assert!(present.is_file(), "held by another harness of the run");
        assert!(manifest.lists_binary(&present), "and still recorded");
        assert!(!manifest.lists_binary(&absent), "already gone: forgotten");
        assert!(
            report
                .notes
                .iter()
                .any(|n| n.contains("another harness in this run still has held registered")),
            "{:?}",
            report.notes
        );

        // Without the hold the file goes.
        apply_binaries(
            &mut ui,
            &ctx,
            &planned,
            &gone,
            &[],
            &mut manifest,
            &mut report,
        );
        assert!(report.problems.is_empty(), "{:?}", report.problems);
        assert!(!present.exists());
        assert!(!manifest.lists_binary(&present));
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
