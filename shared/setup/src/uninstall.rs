//! Reversing an install from its manifest.
//!
//! Owns the uninstall plan and its execution: which recorded files are removed
//! (signature-checked), which are user-owned and asked about, unregistering the
//! servers, restoring edited configuration keys, removing binaries that no other
//! kit's manifest lists, and the legacy cleanups. It does not remove anything the
//! manifest does not list except the legacy leftovers named in the descriptor.
//!
//! Main entry points: [`plan`], [`print_summary`] and [`apply`].

use crate::binaries;
use crate::config::ConfigRecord;
use crate::created;
use crate::env::{Env, Harness};
use crate::error::{IoContext, Result};
use crate::kit::Kit;
use crate::legacy;
use crate::manifest::{EntryKind, Manifest, manifest_path, other_manifests_listing};
use crate::report::Report;
use crate::serve;
use crate::signature::{Owner, classify, classify_skill};
use crate::ui::Ui;
use crate::util::read_text_opt;
use std::fs;
use std::path::{Path, PathBuf};

/// One recorded file and what uninstall does with it.
#[derive(Debug, Clone)]
pub struct FileDisposition {
    /// The recorded path.
    pub path: PathBuf,
    /// File or folder.
    pub kind: EntryKind,
    /// Who owns it according to its signature.
    pub owner: Option<Owner>,
}

/// Everything uninstall will do.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    /// Kit-managed files and folders to remove.
    pub managed: Vec<FileDisposition>,
    /// User-owned files and folders, removed only if the user chooses.
    pub user: Vec<FileDisposition>,
    /// Recorded files with an unrecognised signature, which stay.
    pub unrecognized: Vec<FileDisposition>,
    /// Whether the user chose to remove user-owned files.
    pub remove_user: bool,
    /// Servers to unregister.
    pub servers: Vec<String>,
    /// Binaries to remove, and those kept because another kit lists them.
    pub binaries_remove: Vec<PathBuf>,
    /// Binaries that stay, with the manifest that lists them.
    pub binaries_keep: Vec<(PathBuf, PathBuf)>,
    /// Configuration keys to restore.
    pub config: Vec<ConfigRecord>,
    /// Leftovers of earlier releases.
    pub legacy: Vec<String>,
    /// Backups made at install time; they stay.
    pub backups: Vec<PathBuf>,
    /// Folders the installer created; each is removed if it is empty at the end.
    pub created_dirs: Vec<PathBuf>,
}

fn dispose(kit_name: &str, path: &Path, kind: EntryKind) -> Option<FileDisposition> {
    let owner = match kind {
        EntryKind::File => {
            if !path.exists() {
                return None;
            }
            Some(
                read_text_opt(path)
                    .ok()
                    .flatten()
                    .map_or(Owner::Unrecognized, |t| classify(kit_name, &t)),
            )
        }
        EntryKind::Dir => {
            if !path.exists() {
                return None;
            }
            Some(
                fs::read_to_string(path.join("SKILL.md"))
                    .map_or(Owner::Unrecognized, |t| classify_skill(kit_name, &t)),
            )
        }
    };
    Some(FileDisposition {
        path: path.to_path_buf(),
        kind,
        owner,
    })
}

fn all_homes(env: &Env, kit: &Kit) -> Vec<PathBuf> {
    let mut homes: Vec<PathBuf> = Harness::ALL.iter().map(|h| env.harness_home(*h)).collect();
    homes.extend(Harness::ALL.iter().map(|h| env.default_home(*h)));
    homes.push(kit.home.clone());
    homes.sort();
    homes.dedup();
    homes
}

/// Why a kept binary is kept, for the plan: the guard is also kept for kits that list only its servers.
fn keep_reason(binary: &Path) -> &'static str {
    if binaries::is_guard(binary) {
        "also listed, or needed by a server listed, by"
    } else {
        "also listed by"
    }
}

/// Why a kept binary is kept, for the report.
fn keep_note(binary: &Path) -> &'static str {
    if binaries::is_guard(binary) {
        "also lists it or a server that needs it"
    } else {
        "also lists it"
    }
}

/// Builds the uninstall plan from the manifest.
pub fn plan(env: &Env, kit: &Kit, remove_user: bool) -> Plan {
    let mut plan = Plan {
        remove_user,
        ..Plan::default()
    };
    let name = kit.name();
    let Some(m) = &kit.prior else {
        plan.servers = kit.servers();
        plan.legacy = legacy_findings(env, kit);
        return plan;
    };
    for f in &m.files {
        let Some(d) = dispose(name, &f.path, f.kind) else {
            continue;
        };
        match d.owner {
            Some(Owner::Managed) => plan.managed.push(d),
            Some(Owner::User) => plan.user.push(d),
            _ => plan.unrecognized.push(d),
        }
    }
    let mut servers: Vec<String> = m
        .registrations
        .iter()
        .filter(|r| r.harness == kit.harness.name() && r.kind == "mcp")
        .map(|r| r.server.clone())
        .collect();
    if servers.is_empty() {
        servers = kit.servers();
    }
    plan.servers = servers;
    let own = manifest_path(&kit.home, name);
    let homes = all_homes(env, kit);
    for b in &m.binaries {
        if !b.exists() {
            continue;
        }
        let others = other_manifests_listing(&homes, &own, &binaries::keepers_of(env, b));
        match others.into_iter().next() {
            Some(o) => plan.binaries_keep.push((b.clone(), o)),
            None => plan.binaries_remove.push(b.clone()),
        }
    }
    plan.config = m.config.clone();
    plan.created_dirs = m
        .created_dirs
        .iter()
        .filter(|d| d.is_dir())
        .cloned()
        .collect();
    plan.backups = m
        .backups
        .iter()
        .map(|b| b.path.clone())
        .filter(|p| p.exists())
        .collect();
    plan.legacy = legacy_findings(env, kit);
    plan
}

fn legacy_findings(env: &Env, kit: &Kit) -> Vec<String> {
    let mut out = Vec::new();
    if kit.payload.desc.legacy.iter().any(|l| l == "workslate") && kit.harness == Harness::Claude {
        out.extend(legacy::workslate_findings(env, &kit.home, &kit.bin_dir));
    }
    if kit.harness == Harness::Codex {
        for p in legacy::earlier_codex_binaries(&kit.home, &kit.bin_dir) {
            out.push(format!("the earlier installer's binary {}", p.display()));
        }
    }
    out
}

/// Prints the summary the user confirms.
pub fn print_summary(ui: &mut Ui, kit: &Kit, plan: &Plan) {
    ui.blank();
    ui.title("Summary");
    ui.line(&format!(
        "  Uninstall {} from {}",
        kit.name(),
        kit.home.display()
    ));
    if kit.prior.is_none() {
        ui.warn(&format!(
            "no manifest at {}; only registrations and known leftovers are handled",
            kit.manifest_path().display()
        ));
    }
    let list = |ui: &mut Ui, title: &str, items: &[FileDisposition], verb: &str| {
        if items.is_empty() {
            return;
        }
        ui.blank();
        ui.line(&format!("  {title}"));
        for d in items {
            ui.line(&format!("    {verb} {}", d.path.display()));
        }
    };
    list(ui, "Kit files removed", &plan.managed, "remove");
    list(
        ui,
        "Files you own (prefs and custom rules)",
        &plan.user,
        if plan.remove_user { "remove" } else { "keep" },
    );
    list(
        ui,
        "Recorded files with an unrecognised signature stay",
        &plan.unrecognized,
        "keep",
    );
    if !plan.servers.is_empty() {
        ui.blank();
        ui.line(&format!(
            "  Unregister servers: {}",
            plan.servers.join(", ")
        ));
    }
    if !plan.config.is_empty() {
        ui.blank();
        ui.line(
            "  Restore configuration keys (only while they still hold what the installer wrote)",
        );
        for c in &plan.config {
            let prev = c
                .previous
                .as_deref()
                .unwrap_or("(absent: the key is removed)");
            ui.line(&format!(
                "    {} in {}: back to {prev}",
                c.path.join("."),
                c.file.display()
            ));
        }
    }
    if !plan.binaries_remove.is_empty() || !plan.binaries_keep.is_empty() {
        ui.blank();
        ui.line("  Binaries");
        for b in &plan.binaries_remove {
            ui.line(&format!("    remove {}", b.display()));
        }
        for (b, by) in &plan.binaries_keep {
            ui.line(&format!(
                "    keep {} ({} {})",
                b.display(),
                keep_reason(b),
                by.display()
            ));
        }
    }
    if !plan.legacy.is_empty() {
        ui.blank();
        ui.line("  Cleanup of earlier releases");
        for l in &plan.legacy {
            ui.line(&format!("    remove {l}"));
        }
    }
    if !plan.created_dirs.is_empty() {
        ui.blank();
        ui.line("  Folders the installer created (removed when they are empty afterwards)");
        for d in &plan.created_dirs {
            ui.line(&format!("    {}", d.display()));
        }
    }
    if !plan.backups.is_empty() {
        ui.blank();
        ui.line("  Backups made at install time stay:");
        for b in &plan.backups {
            ui.line(&format!("    {}", b.display()));
        }
    }
}

fn remove_path(ui: &mut Ui, d: &FileDisposition) -> Result<()> {
    match d.kind {
        EntryKind::File => {
            fs::remove_file(&d.path).ctx(|| format!("removing {}", d.path.display()))?
        }
        EntryKind::Dir => {
            fs::remove_dir_all(&d.path).ctx(|| format!("removing {}", d.path.display()))?
        }
    }
    ui.ok(&format!("removed {}", d.path.display()));
    Ok(())
}

fn run_steps(env: &Env, ui: &mut Ui, kit: &Kit, plan: &Plan, report: &mut Report) -> Result<()> {
    let empty = Manifest::new(kit.name());
    let manifest = kit.prior.as_ref().unwrap_or(&empty);
    for d in &plan.managed {
        remove_path(ui, d)?;
    }
    if !plan.managed.is_empty() {
        report.done(
            "removed",
            format!("{} kit files and folders", plan.managed.len()),
        );
    }
    if plan.remove_user {
        for d in &plan.user {
            remove_path(ui, d)?;
        }
        if !plan.user.is_empty() {
            report.done("removed", format!("{} files you own", plan.user.len()));
        }
    } else if !plan.user.is_empty() {
        report.done(
            "kept",
            format!(
                "{} files you own (not managed by the kit from now on)",
                plan.user.len()
            ),
        );
        for d in &plan.user {
            ui.detail(&d.path.display().to_string());
        }
    }
    for d in &plan.unrecognized {
        report.note(format!(
            "{} has no kit signature and was left as it is",
            d.path.display()
        ));
    }

    serve::unserve(
        env,
        ui,
        kit.harness,
        &kit.home,
        &plan.servers,
        manifest,
        report,
    )?;

    for b in &plan.binaries_remove {
        fs::remove_file(b).ctx(|| format!("removing {}", b.display()))?;
        ui.ok(&format!("removed {}", b.display()));
    }
    for (b, by) in &plan.binaries_keep {
        report.note(format!(
            "{} was kept: {} {}",
            b.display(),
            by.display(),
            keep_note(b)
        ));
    }

    let mut mf = kit
        .prior
        .clone()
        .unwrap_or_else(|| Manifest::new(kit.name()));
    if kit.payload.desc.legacy.iter().any(|l| l == "workslate") && kit.harness == Harness::Claude {
        for n in legacy::workslate_cleanup(env, ui, &kit.home, &kit.bin_dir, &mut mf)? {
            report.note(n);
        }
    }
    if kit.harness == Harness::Codex {
        legacy::remove_earlier_codex_binaries(ui, &kit.home, &kit.bin_dir)?;
    }
    Manifest::delete(&kit.home, kit.name(), kit.harness)?;
    created::remove_empty(ui, &plan.created_dirs);
    Ok(())
}

/// Carries out the uninstall plan. Always returns a report.
pub fn apply(env: &Env, ui: &mut Ui, kit: &Kit, plan: &Plan) -> Report {
    let mut report = Report::default();
    ui.blank();
    ui.title("Applying");
    if let Err(e) = run_steps(env, ui, kit, plan, &mut report) {
        report.problems.push(
            e.with_state("the manifest was kept, so the uninstall can be repeated")
                .with_fix("fix the cause above and re-run `slate-setup uninstall`"),
        );
    }
    report
}
