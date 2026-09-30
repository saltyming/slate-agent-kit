//! `slate-setup`: the installer for the slate agent kits.
//!
//! Installs one kit into one harness home (Claude Code, Codex or Kimi Code): the
//! kit's instruction files, rules and skills; the aside, dispatch and palette MCP
//! servers; the prefs files; and the harness's native configuration. Also
//! reconfigures and uninstalls. Every run has one shape: detect, ask, summarise
//! and confirm, apply, report.
//!
//! This file owns the top-level flow ([`run`]); the modules own the steps.

#![warn(missing_docs)]

pub mod apply;
pub mod binaries;
pub mod cli;
pub mod config;
pub mod created;
pub mod descriptor;
pub mod env;
pub mod error;
pub mod harness;
pub mod json;
pub mod kit;
pub mod legacy;
pub mod manifest;
pub mod mcp;
pub mod options;
pub mod plan;
pub mod prefs;
pub mod report;
pub mod rules;
pub mod serve;
pub mod signature;
pub mod ui;
pub mod uninstall;
pub mod util;
pub mod wizard;

use env::Env;
use error::{Error, Kind as ErrorKind, Result};
use kit::Kit;
use options::{Kind, Options};
use std::ffi::OsString;
use ui::Ui;

fn print_error(ui: &mut Ui, e: &Error) {
    ui.fail(&e.to_string());
    if let Some(state) = &e.state {
        ui.detail(&format!("state: {state}"));
    }
    if let Some(fix) = &e.fix {
        ui.detail(&format!("fix:   {fix}"));
    }
}

fn print_detected(ui: &mut Ui, env: &Env, kit: &Kit, states: &[kit::PrefsState]) {
    ui.title("Detected");
    ui.line(&format!(
        "  Harness  {} at {}",
        kit.harness.product(),
        kit.home.display()
    ));
    let installed = match &kit.prior {
        Some(m) if m.version.is_empty() && m.from_legacy => {
            "an earlier release (older manifest)".to_string()
        }
        Some(m) if m.version.is_empty() => "an incomplete install".to_string(),
        Some(m) => format!("version {}", m.version),
        None => "not installed".to_string(),
    };
    ui.line(&format!(
        "  Kit      {} {} (installed: {installed})",
        kit.payload.desc.kit, kit.payload.desc.version
    ));
    if let Some(program) = env.cli_program(kit.harness) {
        match env.which(&program) {
            Some(p) => ui.line(&format!("  CLI      {program} at {}", p.display())),
            None => ui.line(&format!(
                "  CLI      {program} not found; the registration commands will be printed instead"
            )),
        }
    }
    if !states.is_empty() {
        let list: Vec<String> = states
            .iter()
            .map(|s| {
                let state = match s.state {
                    prefs::State::Missing => "new",
                    prefs::State::Current => "found",
                    prefs::State::Legacy => "old format",
                };
                format!("{} ({state})", s.name)
            })
            .collect();
        ui.line(&format!("  Prefs    {}", list.join(", ")));
    }
}

fn run_kit(kind: Kind, opts: &Options, env: &Env, ui: &mut Ui) -> Result<i32> {
    let kit = Kit::open(env, opts)?;
    let states = kit.prefs_states()?;
    print_detected(ui, env, &kit, &states);
    match kind {
        Kind::Install | Kind::Configure => {
            let choices = wizard::ask(env, ui, &kit, opts, kind, &states)?;
            let plan = plan::build(env, &kit, &choices, kind, &states)?;
            plan::print_summary(ui, env, &kit, &plan);
            if opts.dry_run {
                ui.blank();
                ui.line("Dry run: nothing was changed.");
                return Ok(0);
            }
            if !ui.confirm("Apply these changes?") {
                return Err(
                    Error::new(ErrorKind::Aborted, "cancelled").with_state("nothing was changed")
                );
            }
            let report = apply::apply(env, ui, &kit, &plan);
            let headline = if kind == Kind::Install {
                "Installed"
            } else {
                "Configured"
            };
            report.print(ui, headline, Some(kit.harness.product()));
            Ok(if report.problems.is_empty() { 0 } else { 1 })
        }
        Kind::Uninstall => {
            let mut remove_user = false;
            let preview = uninstall::plan(env, &kit, false);
            if !preview.user.is_empty() && ui.interactive() {
                ui.step(1, 1, "Files you own");
                for d in &preview.user {
                    ui.detail(&d.path.display().to_string());
                }
                remove_user = ui.ask_yes_no(
                    "Remove these too? (prefs and custom rules; default: keep)",
                    false,
                );
            }
            let plan = uninstall::plan(env, &kit, remove_user);
            uninstall::print_summary(ui, &kit, &plan);
            if opts.dry_run {
                ui.blank();
                ui.line("Dry run: nothing was changed.");
                return Ok(0);
            }
            if !ui.confirm("Apply these changes?") {
                return Err(
                    Error::new(ErrorKind::Aborted, "cancelled").with_state("nothing was changed")
                );
            }
            let report = uninstall::apply(env, ui, &kit, &plan);
            report.print(ui, "Uninstalled", Some(kit.harness.product()));
            Ok(if report.problems.is_empty() { 0 } else { 1 })
        }
        Kind::Mcp => Err(Error::usage("`mcp` runs without a kit")),
    }
}

/// Executes a parsed command and returns the process exit code.
pub fn execute(kind: Kind, opts: &Options, env: &Env, ui: &mut Ui) -> i32 {
    let result = match kind {
        Kind::Mcp => mcp::run(env, ui, opts).map(|report| {
            report.print(ui, "Done", None);
            if report.problems.is_empty() { 0 } else { 1 }
        }),
        other => run_kit(other, opts, env, ui),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            ui.blank();
            print_error(ui, &e);
            e.exit_code()
        }
    }
}

/// Parses `args` (with the program name first) and runs the command.
///
/// `ui` is `None` for a real run; tests pass a captured or scripted interface.
pub fn run<I: IntoIterator<Item = OsString>>(args: I, env: &Env, ui: Option<Ui>) -> i32 {
    let parsed = match cli::parse(args) {
        Ok(p) => p,
        Err(cli::ParseExit::Clap { text, code }) => {
            if code == 0 {
                print!("{text}");
            } else {
                eprint!("{text}");
            }
            return code;
        }
        Err(cli::ParseExit::Usage(e)) => {
            eprintln!("error: {e}");
            return e.exit_code();
        }
    };
    let mut ui =
        ui.unwrap_or_else(|| Ui::for_process(parsed.options.yes, env.get("NO_COLOR").is_some()));
    execute(parsed.kind, &parsed.options, env, &mut ui)
}
