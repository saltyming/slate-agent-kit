//! The plan of an install or configure run, and its summary.
//!
//! Owns turning the payload, the current state of the harness home and the
//! answers into typed actions (files, prefs, custom rules, binaries, servers,
//! configuration keys, cleanups), deciding for each file whether it is created,
//! updated, backed up or skipped, and printing the summary that is confirmed
//! before anything changes. It reads the file system but never writes it.
//!
//! Main entry points: [`build`], [`Plan`] and [`print_summary`].

use crate::binaries::{self, Mode};
use crate::config::{ConfigDoc, TomlDoc};
use crate::descriptor::LoadMode;
use crate::env::{Env, Harness};
use crate::error::{Error, Result};
use crate::harness::{SubagentPrefs, claude, codex, kimi};
use crate::kit::{Kit, PrefsState, on_path};
use crate::legacy;
use crate::manifest::Manifest;
use crate::options::Kind;
use crate::prefs::{self, Action as PrefsActionKind, doc::PrefsDoc, schema};
use crate::rules::{self, CustomDecision, CustomRule};
use crate::signature::{Owner, classify, classify_skill};
use crate::ui::{Style, Ui};
use crate::util::read_text_opt;
use crate::wizard::{Choices, CustomChoice};
use std::fs;
use std::path::{Path, PathBuf};

/// The part of the payload a file action installs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The primary instruction file.
    Primary,
    /// A rule file.
    Rule,
    /// A skill folder.
    Skill,
}

/// What is written.
#[derive(Debug, Clone)]
pub enum Content {
    /// A text file.
    Text(String),
    /// A skill folder copied from the payload.
    Skill {
        /// The folder in the payload.
        src: PathBuf,
        /// Files under it, relative.
        files: Vec<PathBuf>,
    },
}

/// What happens to the destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The destination is new.
    Create,
    /// A kit-managed destination is replaced.
    Update,
    /// The destination already has this content.
    Unchanged,
    /// The destination is not kit-managed; it is copied to a `.bak-` file, then replaced.
    BackupReplace,
    /// The destination is left alone; the string says why.
    Skip(String),
}

/// One payload file or folder to install.
#[derive(Debug, Clone)]
pub struct FileAction {
    /// Destination.
    pub dest: PathBuf,
    /// Which part of the payload it is.
    pub role: Role,
    /// What is written.
    pub content: Content,
    /// What happens to the destination.
    pub decision: Decision,
}

/// One prefs file and what happens to it.
#[derive(Debug, Clone)]
pub struct PrefsAction {
    /// Installed path.
    pub dest: PathBuf,
    /// The plan for its text.
    pub plan: prefs::FilePlan,
}

/// Everything an install or configure run will do.
#[derive(Debug, Clone)]
pub struct Plan {
    /// Install or configure.
    pub kind: Kind,
    /// Binaries to obtain, when any.
    pub binaries: Option<binaries::Request>,
    /// Servers to register.
    pub servers: Vec<String>,
    /// Workspace roots.
    pub roots: Vec<String>,
    /// Payload files and folders.
    pub files: Vec<FileAction>,
    /// Files of the earlier install that this payload no longer ships.
    pub stale: Vec<PathBuf>,
    /// Prefs files.
    pub prefs: Vec<PrefsAction>,
    /// Custom rules to copy.
    pub custom: Vec<CustomRule>,
    /// The custom rules folder to remember.
    pub custom_dir: Option<PathBuf>,
    /// The subagent default to write natively; `None` leaves the native keys alone.
    pub subagent: Option<SubagentPrefs>,
    /// Configuration keys that would change, as display lines by file.
    pub config_preview: Vec<(PathBuf, Vec<String>)>,
    /// Leftovers of earlier releases that will be cleaned up.
    pub legacy: Vec<String>,
    /// Facts to show with the summary.
    pub notes: Vec<String>,
}

fn decide_text(kit_name: &str, dest: &Path, new_text: &str) -> Result<Decision> {
    match read_text_opt(dest) {
        Ok(None) => Ok(Decision::Create),
        Ok(Some(existing)) => Ok(match classify(kit_name, &existing) {
            Owner::Managed if existing == new_text => Decision::Unchanged,
            Owner::Managed => Decision::Update,
            Owner::User | Owner::Unrecognized => Decision::BackupReplace,
        }),
        // A file that is not UTF-8 cannot be a managed Markdown file.
        Err(_) => Ok(Decision::BackupReplace),
    }
}

fn decide_rule(kit_name: &str, dest: &Path, new_text: &str) -> Result<Decision> {
    match decide_text(kit_name, dest, new_text)? {
        Decision::BackupReplace => {
            let user_owned = read_text_opt(dest)
                .ok()
                .flatten()
                .is_some_and(|t| classify(kit_name, &t) == Owner::User);
            if user_owned {
                Ok(Decision::Skip(
                    "a file you own already has this name".into(),
                ))
            } else {
                Ok(Decision::BackupReplace)
            }
        }
        other => Ok(other),
    }
}

fn decide_skill(kit_name: &str, dest: &Path) -> Decision {
    if !dest.exists() {
        return Decision::Create;
    }
    match fs::read_to_string(dest.join("SKILL.md")) {
        Ok(t) if classify_skill(kit_name, &t) == Owner::Managed => Decision::Update,
        _ => Decision::Skip("a skill you own already has this name".into()),
    }
}

fn heading_of(key: &str) -> &'static str {
    schema::find(key).map_or("", |s| s.heading)
}

fn subagent_from_prefs(
    kit: &Kit,
    planned: &[PrefsAction],
    states: &[PrefsState],
    notes: &mut Vec<String>,
) -> Option<SubagentPrefs> {
    let state = states.iter().find(|s| s.name == "subagent")?;
    let action = planned.iter().find(|a| a.plan.name == "subagent")?;
    let text = match (&action.plan.text, &action.plan.action) {
        (Some(t), _) => t.clone(),
        (None, PrefsActionKind::KeepLegacy) => return None,
        (None, _) => state.existing.clone()?,
    };
    let doc = PrefsDoc::parse(&text);
    let model = doc
        .get(heading_of("subagent.model"))
        .unwrap_or_default()
        .trim()
        .to_string();
    let effort = doc
        .get(heading_of("subagent.effort"))
        .unwrap_or_default()
        .trim()
        .to_string();
    let vctx = kit.validation_ctx();
    let effort = if kit.harness == Harness::Claude {
        String::new()
    } else {
        effort
    };
    if let Err(e) = schema::validate_subagent_model(&vctx, &model) {
        notes.push(format!(
            "subagent model in {}: {e}; the harness configuration is left as it is",
            action.dest.display()
        ));
        return None;
    }
    if let Err(e) = schema::validate_subagent_effort(&vctx, &effort) {
        notes.push(format!(
            "subagent effort in {}: {e}; the harness configuration is left as it is",
            action.dest.display()
        ));
        return None;
    }
    Some(SubagentPrefs { model, effort })
}

fn preview_config(kit: &Kit, plan: &Plan) -> Vec<(PathBuf, Vec<String>)> {
    let file = kit.config_file();
    let text = read_text_opt(&file).ok().flatten();
    let manifest = kit
        .prior
        .clone()
        .unwrap_or_else(|| Manifest::new(kit.name()));
    let mut lines = Vec::new();
    let placeholder = vec!["<read-only tools>".to_string()];
    let describe = |c: &crate::config::Change| -> String {
        if c.added.is_empty() {
            let old = c
                .old
                .as_ref()
                .map(|j| j.to_compact())
                .unwrap_or_else(|| "(absent)".to_string());
            format!("{}: {old} -> {}", c.dotted(), c.new.to_compact())
        } else {
            format!("{}: adds {}", c.dotted(), c.added.join(", "))
        }
    };
    let out = match kit.harness {
        Harness::Claude => {
            let Ok(mut doc) = claude::parse_settings(text.as_deref(), &file) else {
                return Vec::new();
            };
            let palette = plan
                .servers
                .iter()
                .any(|s| s == "palette")
                .then_some(placeholder.as_slice());
            claude::settings_edits(&mut doc, &file, palette, plan.subagent.as_ref(), &manifest)
        }
        Harness::Codex => {
            let Ok(mut doc) = TomlDoc::parse(text.as_deref().unwrap_or("")) else {
                return Vec::new();
            };
            // The tables `codex mcp add` will write are simulated so their keys show up.
            for s in &plan.servers {
                if doc.get(&["mcp_servers", s]).ok().flatten().is_none() {
                    let _ = doc.set(
                        &["mcp_servers", s, "command"],
                        &crate::json::Json::str("..."),
                    );
                }
            }
            let opts = codex::Opts {
                servers: &plan.servers,
                palette_tools: plan
                    .servers
                    .iter()
                    .any(|s| s == "palette")
                    .then_some(placeholder.as_slice()),
                subagent: plan.subagent.clone(),
            };
            codex::config_edits(&mut doc, &file, &opts, &manifest)
        }
        Harness::Kimi => {
            let Ok(mut doc) = TomlDoc::parse(text.as_deref().unwrap_or("")) else {
                return Vec::new();
            };
            kimi::config_edits(&mut doc, &file, plan.subagent.as_ref(), &manifest)
        }
    };
    for c in &out.changes {
        lines.push(describe(c));
    }
    for e in &out.refusals {
        lines.push(format!("refused: {e}"));
    }
    for u in &out.undone {
        lines.push(format!("{}: restored to its previous value", u.join(".")));
    }
    if lines.is_empty() {
        Vec::new()
    } else {
        vec![(file, lines)]
    }
}

/// Builds the plan for `kind` (`Install` or `Configure`).
pub fn build(
    env: &Env,
    kit: &Kit,
    choices: &Choices,
    kind: Kind,
    states: &[PrefsState],
) -> Result<Plan> {
    let desc = &kit.payload.desc;
    let name = kit.name();
    let vctx = kit.validation_ctx();
    let mut notes = Vec::new();

    // Binaries and the servers that get registered.
    let mut servers = kit.servers();
    let mut binaries_req = None;
    match (kind, choices.binaries) {
        (Kind::Install, Mode::Skip) => {
            if !servers.is_empty() {
                notes.push("binaries skipped: no server is registered".to_string());
            }
            servers.clear();
        }
        (Kind::Install, mode) if !servers.is_empty() => {
            if mode == Mode::Build && choices.slate_dir.is_none() {
                return Err(Error::usage(
                    "`--binaries build` needs `--slate-dir <slate checkout>`",
                )
                .with_state("nothing was changed"));
            }
            binaries_req = Some(binaries::Request {
                mode,
                names: servers.clone(),
                bin_dir: kit.bin_dir.clone(),
                slate_version: desc.slate_version.clone(),
                slate_dir: choices.slate_dir.clone(),
            });
        }
        _ => {}
    }

    // Prefs.
    let mut prefs_plans = Vec::new();
    for st in states {
        let input = choices.prefs.get(&st.name).cloned().unwrap_or_default();
        let plan = prefs::plan_file(
            &st.name,
            st.existing.as_deref(),
            &st.template,
            st.state,
            &input,
            &vctx,
        )?;
        notes.extend(plan.warnings.iter().cloned());
        prefs_plans.push(PrefsAction {
            dest: st.dest.clone(),
            plan,
        });
    }
    let subagent = subagent_from_prefs(kit, &prefs_plans, states, &mut notes);

    // Custom rules.
    let prior_custom: Vec<PathBuf> = kit
        .prior
        .as_ref()
        .map(|m| m.custom_rules.clone())
        .unwrap_or_default();
    let (custom, custom_dir) = match &choices.custom {
        CustomChoice::Folder(dir) => {
            let mut reserved: Vec<rules::Reserved> = desc
                .rules
                .iter()
                .map(|r| rules::Reserved {
                    path: kit.rules_dir.join(r),
                    reason: "it would replace a kit-managed file",
                })
                .collect();
            reserved.extend(states.iter().map(|s| rules::Reserved {
                path: s.dest.clone(),
                reason: "the name is reserved for a prefs file",
            }));
            (
                rules::plan_custom_rules(name, &kit.rules_dir, dir, &prior_custom, &reserved)?,
                Some(dir.clone()),
            )
        }
        CustomChoice::Clear => (Vec::new(), None),
        CustomChoice::Keep => (
            Vec::new(),
            kit.prior.as_ref().and_then(|m| m.custom_rules_dir.clone()),
        ),
    };
    for c in &custom {
        if let CustomDecision::Refused(why) = &c.decision {
            notes.push(format!(
                "custom rule {} was not installed: {why}",
                c.src.display()
            ));
        }
    }

    // Payload files.
    let mut files = Vec::new();
    let primary_dest = kit.home.join(kit.harness.primary_file());
    let primary_text = kit.payload.read(&kit.payload.primary_path())?;
    let mut rule_texts = Vec::new();
    for r in &desc.rules {
        rule_texts.push((r.clone(), kit.payload.read(&kit.payload.rule_path(r))?));
    }
    let primary_content = match desc.load {
        LoadMode::RulesDir => primary_text.clone(),
        LoadMode::Concat => {
            let custom_now = rules::effective_custom_rules(name, &prior_custom, &custom);
            let texts: Vec<String> = rule_texts.iter().map(|(_, t)| t.clone()).collect();
            let custom_texts: Vec<String> = custom_now.into_iter().map(|(_, t)| t).collect();
            rules::build_concat(&primary_text, &texts, &custom_texts)
        }
    };
    if kind == Kind::Install || desc.load == LoadMode::Concat {
        let decision = decide_text(name, &primary_dest, &primary_content)?;
        files.push(FileAction {
            dest: primary_dest,
            role: Role::Primary,
            content: Content::Text(primary_content),
            decision,
        });
    }
    if kind == Kind::Install {
        for (r, text) in &rule_texts {
            let dest = kit.rules_dir.join(r);
            let decision = decide_rule(name, &dest, text)?;
            files.push(FileAction {
                dest,
                role: Role::Rule,
                content: Content::Text(text.clone()),
                decision,
            });
        }
        for s in &desc.skills {
            let dest = kit.home.join("skills").join(s);
            files.push(FileAction {
                decision: decide_skill(name, &dest),
                dest,
                role: Role::Skill,
                content: Content::Skill {
                    src: kit.payload.skill_dir(s),
                    files: kit.payload.skill_files(s)?,
                },
            });
        }
    }

    // Files an earlier install put there that this payload does not ship.
    let mut stale = Vec::new();
    if kind == Kind::Install
        && let Some(prior) = &kit.prior
    {
        for f in prior.files.iter().filter(|f| !f.user) {
            if files.iter().any(|a| a.dest == f.path)
                || prefs_plans.iter().any(|p| p.dest == f.path)
            {
                continue;
            }
            let managed = match f.kind {
                crate::manifest::EntryKind::File => read_text_opt(&f.path)
                    .ok()
                    .flatten()
                    .is_some_and(|t| classify(name, &t) == Owner::Managed),
                crate::manifest::EntryKind::Dir => fs::read_to_string(f.path.join("SKILL.md"))
                    .ok()
                    .is_some_and(|t| classify_skill(name, &t) == Owner::Managed),
            };
            if managed {
                stale.push(f.path.clone());
            }
        }
    }

    // Leftovers.
    let mut legacy_found = Vec::new();
    if kind == Kind::Install {
        if desc.legacy.iter().any(|l| l == "workslate") && kit.harness == Harness::Claude {
            legacy_found.extend(legacy::workslate_findings(env, &kit.home, &kit.bin_dir));
        }
        if kit.harness == Harness::Codex {
            for p in legacy::earlier_codex_binaries(&kit.home, &kit.bin_dir) {
                legacy_found.push(format!("the earlier installer's binary {}", p.display()));
            }
        }
    }

    let mut plan = Plan {
        kind,
        binaries: binaries_req,
        servers: if kind == Kind::Configure {
            kit.servers()
        } else {
            servers
        },
        roots: choices.roots.clone(),
        files,
        stale,
        prefs: prefs_plans,
        custom,
        custom_dir,
        subagent,
        config_preview: Vec::new(),
        legacy: legacy_found,
        notes,
    };
    plan.config_preview = preview_config(kit, &plan);
    Ok(plan)
}

fn decision_word(d: &Decision) -> String {
    match d {
        Decision::Create => "write".into(),
        Decision::Update => "replace".into(),
        Decision::Unchanged => "keep (identical)".into(),
        Decision::BackupReplace => "back up, then replace".into(),
        Decision::Skip(why) => format!("skip ({why})"),
    }
}

/// Prints the summary the user confirms.
pub fn print_summary(ui: &mut Ui, env: &Env, kit: &Kit, plan: &Plan) {
    ui.blank();
    ui.title("Summary");
    let desc = &kit.payload.desc;
    let installed = kit.prior.as_ref().map(|m| {
        if m.version.is_empty() {
            "an earlier or incomplete install".to_string()
        } else {
            format!("version {}", m.version)
        }
    });
    ui.line(&format!(
        "  {} {} into {} ({})",
        match plan.kind {
            Kind::Install => "Install",
            _ => "Configure",
        },
        desc.kit,
        kit.home.display(),
        kit.harness.product()
    ));
    match installed {
        Some(v) => ui.line(&format!("  Replaces {v} with version {}", desc.version)),
        None => ui.line(&format!(
            "  Version {} (nothing installed yet)",
            desc.version
        )),
    }
    if let Some(b) = &plan.binaries {
        let source = match b.mode {
            Mode::Prebuilt => format!(
                "download from slate release v{} (checksums verified)",
                b.slate_version
            ),
            Mode::Build => format!(
                "build in {}",
                b.slate_dir
                    .as_deref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            ),
            Mode::Skip => "skip".into(),
        };
        ui.blank();
        ui.line("  Binaries");
        ui.line(&format!("    {source}"));
        for n in &b.names {
            ui.line(&format!(
                "    + {}",
                binaries::binary_path(env, &b.bin_dir, n).display()
            ));
        }
    }
    let file_lines: Vec<&FileAction> = plan.files.iter().collect();
    if !file_lines.is_empty() {
        ui.blank();
        ui.line("  Files");
        for f in file_lines {
            let word = decision_word(&f.decision);
            let shown = if f.role == Role::Skill {
                format!("{} (folder)", f.dest.display())
            } else {
                f.dest.display().to_string()
            };
            ui.line(&format!("    {} {shown}", ui.paint(Style::Cyan, &word)));
        }
    }
    if !plan.stale.is_empty() {
        ui.blank();
        ui.line("  Files the new release no longer ships");
        for s in &plan.stale {
            ui.line(&format!("    remove {}", s.display()));
        }
    }
    let visible_prefs: Vec<&PrefsAction> = plan.prefs.iter().collect();
    if !visible_prefs.is_empty() {
        ui.blank();
        ui.line("  Prefs");
        for p in visible_prefs {
            let word = match p.plan.action {
                PrefsActionKind::Keep => "keep as it is",
                PrefsActionKind::Create => "write",
                PrefsActionKind::Update => "change values in",
                PrefsActionKind::Migrate => "migrate (old file saved as .bak)",
                PrefsActionKind::KeepLegacy => "keep old layout",
            };
            ui.line(&format!("    {word} {}", p.dest.display()));
            for c in &p.plan.changes {
                let old = c.old.as_deref().unwrap_or("(none)");
                ui.detail(&format!(
                    "{}: {old} -> {}",
                    c.key,
                    if c.new.is_empty() { "(blank)" } else { &c.new }
                ));
            }
        }
    }
    if !plan.custom.is_empty() {
        ui.blank();
        ui.line("  Custom rules");
        for c in &plan.custom {
            let word = match &c.decision {
                CustomDecision::Create => "write".to_string(),
                CustomDecision::Replace => "replace".to_string(),
                CustomDecision::Unchanged => "keep (identical)".to_string(),
                CustomDecision::Refused(why) => format!("skip ({why})"),
            };
            ui.line(&format!("    {word} {}", c.dest.display()));
        }
    }
    if !plan.servers.is_empty() {
        ui.blank();
        ui.line("  Server registration");
        for s in &plan.servers {
            let bin = binaries::binary_path(env, &kit.bin_dir, s);
            ui.line(&format!("    {s} -> {}", bin.display()));
        }
        if !plan.roots.is_empty() {
            ui.line(&format!("    workspace roots: {}", plan.roots.join(", ")));
        }
        if kit.harness == Harness::Kimi {
            ui.line(&format!(
                "    plugin folder {}",
                kimi::plugin_root(&kit.home).display()
            ));
        }
    }
    for (file, lines) in &plan.config_preview {
        ui.blank();
        ui.line(&format!("  Configuration keys in {}", file.display()));
        for l in lines {
            ui.line(&format!("    {l}"));
        }
    }
    if !plan.legacy.is_empty() {
        ui.blank();
        ui.line("  Cleanup of earlier releases");
        for l in &plan.legacy {
            ui.line(&format!("    remove {l}"));
        }
    }
    for n in &plan.notes {
        ui.warn(n);
    }
    if !plan.servers.is_empty() && !on_path(env, &kit.bin_dir) {
        ui.detail(&format!(
            "{} is not on PATH; the servers are registered by full path, so they still work",
            kit.bin_dir.display()
        ));
    }
}
