//! Server registration and native configuration as one step, and its reversal.
//!
//! Owns the sequence shared by `install`, `configure` and `mcp`: register the
//! servers with the harness, read the palette server's read-only tools, then
//! edit the harness configuration file and record every edited key. Also owns
//! the reverse for `uninstall`. It does not choose servers or values.
//!
//! Main entry points: [`serve`], [`unserve`] and [`Input`].

use crate::binaries::{binary_path, read_only_tools};
use crate::config::{ConfigDoc, ConfigRecord, JsonDoc, Restored, TomlDoc, restore};
use crate::env::{Env, Harness};
use crate::error::{Error, Result};
use crate::harness::{self, Ctx, EditOutcome, SubagentPrefs, claude, codex, kimi};
use crate::manifest::Manifest;
use crate::report::Report;
use crate::ui::Ui;
use crate::util::{read_text_opt, write_atomic};
use std::path::{Path, PathBuf};

/// What the step needs.
pub struct Input<'a> {
    /// Registration context.
    pub ctx: &'a Ctx,
    /// The harness configuration file.
    pub config_file: PathBuf,
    /// The subagent default; `None` leaves the native keys alone.
    pub subagent: Option<SubagentPrefs>,
}

fn handle_outcome(
    ui: &mut Ui,
    out: harness::Outcome,
    manifest: &mut Manifest,
    report: &mut Report,
) {
    if let Some(f) = out.failure {
        report.problems.push(f);
    }
    for r in out.registered {
        manifest.record_registration(r);
    }
    for (orig, backup) in &out.backups {
        manifest.record_backup(backup, orig);
    }
    for n in out.notes {
        report.note(n);
    }
    for m in out.manual {
        report.manual(m);
    }
    if !out.servers_done.is_empty() {
        ui.ok(&format!("registered {}", out.servers_done.join(", ")));
    }
}

fn describe_change(c: &crate::config::Change) -> String {
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
}

fn finish_edit(
    ui: &mut Ui,
    out: EditOutcome,
    original: Option<&str>,
    new_text: impl FnOnce() -> String,
    file: &Path,
    manifest: &mut Manifest,
    report: &mut Report,
) -> Result<()> {
    for e in out.refusals {
        report.problems.push(e);
    }
    if out.changes.is_empty() && out.undone.is_empty() {
        return Ok(());
    }
    let text = new_text();
    if original != Some(text.as_str()) {
        write_atomic(file, text.as_bytes())?;
    }
    for c in &out.changes {
        manifest.record_config(c.record(file));
        ui.detail(&describe_change(c));
    }
    for path in &out.undone {
        manifest.forget_config(file, path);
        ui.detail(&format!("{}: restored", path.join(".")));
    }
    Ok(())
}

fn apply_config(
    env: &Env,
    ui: &mut Ui,
    input: &Input<'_>,
    registered: &[String],
    tools: Option<&[String]>,
    manifest: &mut Manifest,
    report: &mut Report,
) -> Result<()> {
    let file = &input.config_file;
    let text = read_text_opt(file)?;
    let _ = env;
    match input.ctx.harness {
        Harness::Claude => {
            let mut doc = match claude::parse_settings(text.as_deref(), file) {
                Ok(d) => d,
                Err(e) => {
                    report.problems.push(e);
                    return Ok(());
                }
            };
            let palette = registered
                .iter()
                .any(|s| s == "palette")
                .then_some(tools)
                .flatten();
            let out =
                claude::settings_edits(&mut doc, file, palette, input.subagent.as_ref(), manifest);
            finish_edit(
                ui,
                out,
                text.as_deref(),
                || doc.render_original(),
                file,
                manifest,
                report,
            )
        }
        Harness::Codex => {
            let mut doc = match TomlDoc::parse(text.as_deref().unwrap_or("")) {
                Ok(d) => d,
                Err(e) => {
                    report
                        .problems
                        .push(
                            e.with_state("config.toml was not changed")
                                .with_fix(format!(
                                    "repair the TOML in {} and re-run `slate-setup configure`",
                                    file.display()
                                )),
                        );
                    return Ok(());
                }
            };
            let mut managed: Vec<String> = registered.to_vec();
            for s in &input.ctx.servers {
                let has_table = matches!(
                    doc.get(&["mcp_servers", s]),
                    Ok(Some(crate::json::Json::Object(_)))
                );
                if has_table && !managed.contains(s) {
                    managed.push(s.clone());
                }
            }
            let opts = codex::Opts {
                servers: &managed,
                palette_tools: tools,
                subagent: input.subagent.clone(),
            };
            let out = codex::config_edits(&mut doc, file, &opts, manifest);
            finish_edit(
                ui,
                out,
                text.as_deref(),
                || doc.render(),
                file,
                manifest,
                report,
            )
        }
        Harness::Kimi => {
            let mut doc = match TomlDoc::parse(text.as_deref().unwrap_or("")) {
                Ok(d) => d,
                Err(e) => {
                    report
                        .problems
                        .push(
                            e.with_state("config.toml was not changed")
                                .with_fix(format!(
                                    "repair the TOML in {} and re-run `slate-setup configure`",
                                    file.display()
                                )),
                        );
                    return Ok(());
                }
            };
            let out = kimi::config_edits(&mut doc, file, input.subagent.as_ref(), manifest);
            finish_edit(
                ui,
                out,
                text.as_deref(),
                || doc.render(),
                file,
                manifest,
                report,
            )
        }
    }
}

/// Registers the servers, then edits the harness configuration.
///
/// Returns the servers that were registered in this run.
pub fn serve(
    env: &Env,
    ui: &mut Ui,
    input: &Input<'_>,
    manifest: &mut Manifest,
    report: &mut Report,
) -> Result<Vec<String>> {
    let mut ctx = input.ctx.clone();
    let mut missing = Vec::new();
    ctx.servers.retain(|s| {
        let present = binary_path(env, &ctx.bin_dir, s).is_file();
        if !present {
            missing.push(s.clone());
        }
        present
    });
    if !missing.is_empty() {
        report.note(format!(
            "no binary for {} in {}; those servers were not registered (run `slate-setup install` to fetch them)",
            missing.join(", "),
            ctx.bin_dir.display()
        ));
    }
    let outcome = if ctx.servers.is_empty() {
        harness::Outcome::default()
    } else {
        match ctx.harness {
            Harness::Claude => claude::register(env, ui, &ctx)?,
            Harness::Codex => codex::register(env, ui, &ctx)?,
            Harness::Kimi => kimi::register(env, ui, &ctx)?,
        }
    };
    let registered = outcome.servers_done.clone();
    handle_outcome(ui, outcome, manifest, report);

    let palette_bin = binary_path(env, &ctx.bin_dir, "palette");
    let tools = if ctx.servers.iter().any(|s| s == "palette") && palette_bin.is_file() {
        match read_only_tools(env, &palette_bin) {
            Ok(t) => Some(t),
            Err(e) => {
                report.note(format!(
                    "the palette read-only tools could not be read ({e}); they are not pre-approved"
                ));
                None
            }
        }
    } else {
        None
    };
    let input = Input {
        ctx: &ctx,
        config_file: input.config_file.clone(),
        subagent: input.subagent.clone(),
    };
    apply_config(
        env,
        ui,
        &input,
        &registered,
        tools.as_deref(),
        manifest,
        report,
    )?;
    Ok(registered)
}

/// A restored key path and what happened to it.
type KeyOutcome = (Vec<String>, Result<Restored>);

/// Restores the recorded keys of `file`, newest record first.
///
/// Newest first matters for pruning: the first key written into a table the
/// installer created carries the "table was created" mark, and that table is
/// empty only after the keys written later have been removed.
fn restore_file(file: &Path, records: &[&ConfigRecord], report: &mut Report) -> Result<()> {
    let Some(text) = read_text_opt(file)? else {
        return Ok(());
    };
    let is_json = file
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"));
    let (new_text, outcomes): (String, Vec<KeyOutcome>) = if is_json {
        let mut doc = match JsonDoc::parse(&text) {
            Ok(d) => d,
            Err(e) => {
                report.problems.push(
                    Error::config(format!("{} could not be read: {e}", file.display()))
                        .with_state("its edited keys were not restored"),
                );
                return Ok(());
            }
        };
        let outs = records
            .iter()
            .rev()
            .map(|r| (r.path.clone(), restore(&mut doc, r)))
            .collect();
        (doc.render_original(), outs)
    } else {
        let mut doc = match TomlDoc::parse(&text) {
            Ok(d) => d,
            Err(e) => {
                report.problems.push(
                    Error::config(format!("{} could not be read: {e}", file.display()))
                        .with_state("its edited keys were not restored"),
                );
                return Ok(());
            }
        };
        let outs = records
            .iter()
            .rev()
            .map(|r| (r.path.clone(), restore(&mut doc, r)))
            .collect();
        (doc.render(), outs)
    };
    let mut changed_any = false;
    for (path, outcome) in outcomes {
        match outcome {
            Ok(Restored::Restored) | Ok(Restored::Removed) => changed_any = true,
            Ok(Restored::Nothing) => {}
            Ok(Restored::Changed(cur)) => report.note(format!(
                "{} in {} was changed after the install ({}); it was left as it is",
                path.join("."),
                file.display(),
                cur.unwrap_or_else(|| "removed".into())
            )),
            Err(e) => report.problems.push(e),
        }
    }
    if changed_any && new_text != text {
        write_atomic(file, new_text.as_bytes())?;
    }
    Ok(())
}

/// Unregisters servers and restores every edited configuration key.
pub fn unserve(
    env: &Env,
    ui: &mut Ui,
    harness: Harness,
    home: &Path,
    servers: &[String],
    manifest: &Manifest,
    report: &mut Report,
) -> Result<()> {
    let outcome = match harness {
        Harness::Claude => claude::unregister(env, ui, home, servers)?,
        Harness::Codex => codex::unregister(env, ui, home, servers)?,
        Harness::Kimi => kimi::unregister(
            ui,
            home,
            manifest
                .created_files
                .iter()
                .any(|f| f == &kimi::registry_path(home)),
        )?,
    };
    for n in outcome.notes {
        report.note(n);
    }
    for m in outcome.manual {
        report.manual(m);
    }
    if !outcome.servers_done.is_empty() {
        report.done("unregistered", outcome.servers_done.join(", "));
    }
    let mut files: Vec<PathBuf> = manifest.config.iter().map(|c| c.file.clone()).collect();
    files.sort();
    files.dedup();
    for file in files {
        let records: Vec<&ConfigRecord> =
            manifest.config.iter().filter(|c| c.file == file).collect();
        restore_file(&file, &records, report)?;
    }
    Ok(())
}
