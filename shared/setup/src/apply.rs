//! Carrying out an install or configure plan.
//!
//! Owns the order of the steps (legacy cleanup, stale files, binaries, payload
//! files, prefs and custom rules, server registration and native configuration),
//! the recording of every change in the manifest, and the rule that the manifest
//! is written even when a step fails, so uninstall can still reverse what
//! happened. It does not decide what to do; `plan` did.
//!
//! Main entry points: [`apply`].

use crate::binaries::{self, Fetch, HttpFetch, Mode, NoFetch};
use crate::created;
use crate::env::{Env, Harness};
use crate::error::{IoContext, Result};
use crate::harness::Ctx;
use crate::kit::Kit;
use crate::legacy;
use crate::manifest::{EntryKind, Manifest};
use crate::options::Kind;
use crate::plan::{Content, Decision, FileAction, Plan, Role};
use crate::prefs::Action as PrefsActionKind;
use crate::report::Report;
use crate::rules::CustomDecision;
use crate::serve;
use crate::ui::Ui;
use crate::util::{backup_file, write_atomic};
use std::fs;
use std::path::Path;

fn install_skill(src: &Path, files: &[std::path::PathBuf], dest: &Path) -> Result<()> {
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dest.with_file_name(format!(".{name}.new-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let build = (|| -> Result<()> {
        for rel in files {
            let to = tmp.join(rel);
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent).ctx(|| format!("creating {}", parent.display()))?;
            }
            fs::copy(src.join(rel), &to).ctx(|| format!("copying {}", src.join(rel).display()))?;
        }
        if dest.exists() {
            fs::remove_dir_all(dest).ctx(|| format!("replacing {}", dest.display()))?;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).ctx(|| format!("creating {}", parent.display()))?;
        }
        fs::rename(&tmp, dest).ctx(|| format!("installing {}", dest.display()))
    })();
    if build.is_err() {
        let _ = fs::remove_dir_all(&tmp);
    }
    build
}

fn write_file_action(
    ui: &mut Ui,
    a: &FileAction,
    manifest: &mut Manifest,
    report: &mut Report,
) -> Result<()> {
    match &a.decision {
        Decision::Skip(why) => {
            report.note(format!("{} was not installed: {why}", a.dest.display()));
            return Ok(());
        }
        Decision::Unchanged => {}
        Decision::BackupReplace => {
            let backup = backup_file(&a.dest)?;
            manifest.record_backup(&backup, &a.dest);
            report.note(format!(
                "{} was not managed by this kit; its previous content is saved as {}",
                a.dest.display(),
                backup.display()
            ));
        }
        Decision::Create | Decision::Update => {}
    }
    if a.decision != Decision::Unchanged {
        match &a.content {
            Content::Text(t) => write_atomic(&a.dest, t.as_bytes())?,
            Content::Skill { src, files } => install_skill(src, files, &a.dest)?,
        }
    }
    let kind = if a.role == Role::Skill {
        EntryKind::Dir
    } else {
        EntryKind::File
    };
    manifest.record_file(&a.dest, kind, false);
    ui.detail(&a.dest.display().to_string());
    Ok(())
}

fn run_steps(
    env: &Env,
    ui: &mut Ui,
    kit: &Kit,
    plan: &Plan,
    manifest: &mut Manifest,
    report: &mut Report,
    completed: &mut Vec<String>,
) -> Result<()> {
    let desc = &kit.payload.desc;

    if plan.kind == Kind::Install
        && desc.legacy.iter().any(|l| l == "workslate")
        && kit.harness == Harness::Claude
    {
        let notes = legacy::workslate_cleanup(env, ui, &kit.home, &kit.bin_dir, manifest)?;
        for n in notes {
            report.note(n);
        }
        completed.push("legacy cleanup".into());
    }

    for path in &plan.stale {
        if path.is_dir() {
            fs::remove_dir_all(path).ctx(|| format!("removing {}", path.display()))?;
        } else if path.exists() {
            fs::remove_file(path).ctx(|| format!("removing {}", path.display()))?;
        }
        manifest.files.retain(|f| &f.path != path);
        ui.ok(&format!("removed {} (no longer shipped)", path.display()));
    }

    if let Some(req) = &plan.binaries {
        ui.blank();
        ui.line("Binaries");
        let fetch: Box<dyn Fetch> = if req.mode == Mode::Prebuilt {
            Box::new(HttpFetch::new()?)
        } else {
            Box::new(NoFetch)
        };
        let got = binaries::install(env, ui, req, fetch.as_ref())?;
        for p in &got.paths {
            manifest.record_binary(p);
            ui.ok(&format!("installed {}", p.display()));
        }
        for n in got.notes {
            report.note(n);
        }
        report.done(
            "binaries",
            got.paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
        );
        completed.push("binaries".into());
    }

    if !plan.files.is_empty() {
        ui.blank();
        ui.line("Files");
        for a in &plan.files {
            write_file_action(ui, a, manifest, report)?;
        }
        let written = plan
            .files
            .iter()
            .filter(|a| {
                matches!(
                    a.decision,
                    Decision::Create | Decision::Update | Decision::BackupReplace
                )
            })
            .count();
        report.done(
            "files",
            format!("{written} written under {}", kit.home.display()),
        );
        completed.push("payload files".into());
    }

    ui.blank();
    ui.line("Prefs and custom rules");
    for p in &plan.prefs {
        match (&p.plan.action, &p.plan.text) {
            (PrefsActionKind::Create | PrefsActionKind::Update, Some(text)) => {
                write_atomic(&p.dest, text.as_bytes())?;
                ui.ok(&format!("wrote {}", p.dest.display()));
            }
            (PrefsActionKind::Migrate, Some(text)) => {
                let backup = backup_file(&p.dest)?;
                manifest.record_backup(&backup, &p.dest);
                write_atomic(&p.dest, text.as_bytes())?;
                ui.ok(&format!(
                    "migrated {} (old file saved as {})",
                    p.dest.display(),
                    backup.display()
                ));
            }
            _ => {}
        }
        if p.dest.exists() {
            manifest.record_file(&p.dest, EntryKind::File, true);
        }
        for w in &p.plan.warnings {
            report.note(w.clone());
        }
    }
    report.done(
        "prefs",
        plan.prefs
            .iter()
            .map(|p| p.plan.name.clone())
            .collect::<Vec<_>>()
            .join(", "),
    );
    for c in &plan.custom {
        match &c.decision {
            CustomDecision::Create | CustomDecision::Replace => {
                write_atomic(&c.dest, c.text.as_bytes())?;
                ui.ok(&format!("wrote {}", c.dest.display()));
            }
            CustomDecision::Unchanged => {}
            CustomDecision::Refused(_) => continue,
        }
        manifest.record_file(&c.dest, EntryKind::File, true);
        if !manifest.custom_rules.contains(&c.dest) {
            manifest.custom_rules.push(c.dest.clone());
        }
    }
    if !plan.custom.is_empty() {
        let n = plan
            .custom
            .iter()
            .filter(|c| !matches!(c.decision, CustomDecision::Refused(_)))
            .count();
        report.done(
            "custom rules",
            format!(
                "{n} from {}",
                plan.custom_dir
                    .as_deref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            ),
        );
    }
    completed.push("prefs and custom rules".into());

    if !plan.servers.is_empty() || plan.subagent.is_some() {
        ui.blank();
        ui.line("Server registration and configuration");
        let ctx = Ctx {
            harness: kit.harness,
            home: kit.home.clone(),
            default_home: env.default_home(kit.harness),
            bin_dir: kit.bin_dir.clone(),
            roots: plan.roots.clone(),
            servers: plan.servers.clone(),
            version: desc.version.clone(),
        };
        let input = serve::Input {
            ctx: &ctx,
            config_file: kit.config_file(),
            subagent: plan.subagent.clone(),
        };
        let problems_before = report.problems.len();
        let registered = serve::serve(env, ui, &input, manifest, report)?;
        if !registered.is_empty() {
            report.done("servers", registered.join(", "));
        }
        completed.push("server registration and configuration".into());
        let registered_ok = report.problems.len() == problems_before && report.manual.is_empty();
        if kit.harness == Harness::Codex && registered_ok {
            legacy::remove_earlier_codex_binaries(ui, &kit.home, &kit.bin_dir)?;
        }
    }
    Ok(())
}

/// Applies `plan`. Always returns a report; a failed step is recorded in `report.problems`.
pub fn apply(env: &Env, ui: &mut Ui, kit: &Kit, plan: &Plan) -> Report {
    let desc = &kit.payload.desc;
    let mut report = Report::default();
    let mut manifest = kit
        .prior
        .clone()
        .unwrap_or_else(|| Manifest::new(kit.name()));
    manifest.kit = kit.name().to_string();
    let mut completed = Vec::new();
    let watch = created::watch_install(
        kit.harness,
        &created::Targets {
            home: &kit.home,
            rules: true,
            skills: !desc.skills.is_empty() && plan.kind == Kind::Install,
            bin_dir: (plan.binaries.is_some() || !plan.servers.is_empty()).then_some(&kit.bin_dir),
            servers: !plan.servers.is_empty(),
        },
    );
    ui.blank();
    ui.title("Applying");
    let result = run_steps(
        env,
        ui,
        kit,
        plan,
        &mut manifest,
        &mut report,
        &mut completed,
    );

    watch.record(&mut manifest);
    // A run that failed part-way keeps the previous version and settings: the files it
    // already replaced are recorded, but the install as a whole is not the new release.
    if result.is_ok() {
        manifest.version = desc.version.clone();
        manifest.slate_version = desc.slate_version.clone();
        manifest.roots = plan.roots.clone();
        if !plan.servers.is_empty() || manifest.bin_dir.is_none() {
            manifest.bin_dir = Some(kit.bin_dir.clone());
        }
        manifest.custom_rules_dir = plan.custom_dir.clone();
    }
    manifest.custom_rules.retain(|p| p.is_file());
    // A run that failed before recording anything leaves no manifest behind.
    let saved = if kit.prior.is_none() && manifest.is_empty() {
        Ok(())
    } else {
        manifest.save(&kit.home)
    };
    if saved.is_ok() && kit.prior.as_ref().is_some_and(|m| m.from_legacy) {
        let _ = Manifest::delete_legacy(&kit.home, kit.name(), kit.harness);
    }
    let progress = if completed.is_empty() {
        "nothing".to_string()
    } else {
        completed.join(", ")
    };
    if let Err(e) = result {
        let e = if e.state.is_some() {
            e
        } else {
            e.with_state(format!(
                "completed before the failure: {progress}; the manifest records them"
            ))
        };
        let e = if e.fix.is_some() {
            e
        } else {
            e.with_fix(match plan.kind {
                Kind::Configure => "fix the cause above and re-run `slate-setup configure`",
                _ => "fix the cause above and re-run `slate-setup install`; it is safe to repeat",
            })
        };
        report.problems.push(e);
    }
    if let Err(e) = saved {
        report.problems.push(e.with_state(
            "the manifest was not written, so uninstall cannot reverse this run exactly",
        ));
    }
    report
}
