//! The `mcp` command: binaries and server registration for one or more
//! harnesses, without a kit.
//!
//! Owns the flow used by slate's `tooling/install-mcp.sh`: obtain the aside,
//! dispatch and palette binaries (and `agent-guard` beside them on Linux and
//! macOS; it is not a server and is never registered), register the servers
//! with each named harness, or unregister them. Registrations and edited keys are recorded in
//! `<home>/.slate-agent-kit-mcp-manifest.toml` so the reverse is exact. It never
//! touches instruction files, prefs or the subagent configuration.
//!
//! Main entry points: [`run`] and [`MCP_KIT`].

use crate::binaries::{self, HttpFetch, Mode, NoFetch};
use crate::created;
use crate::env::{Env, Harness};
use crate::error::{Error, Result};
use crate::harness::Ctx;
use crate::manifest::Manifest;
use crate::options::Options;
use crate::report::Report;
use crate::serve;
use crate::ui::Ui;
use crate::wizard::parse_roots;
use std::path::PathBuf;

/// The manifest name of the `mcp` command.
pub const MCP_KIT: &str = "slate-agent-kit-mcp";

fn config_file(harness: Harness, home: &std::path::Path) -> PathBuf {
    match harness {
        Harness::Claude => home.join("settings.json"),
        Harness::Codex | Harness::Kimi => home.join("config.toml"),
    }
}

/// Runs the `mcp` command and returns its report.
pub fn run(env: &Env, ui: &mut Ui, opts: &Options) -> Result<Report> {
    let mut harnesses = opts.harnesses.clone();
    harnesses.sort();
    harnesses.dedup();
    if opts.home.is_some() && harnesses.len() != 1 {
        return Err(Error::usage(
            "`--home` applies to exactly one `--harness`; use CLAUDE_CONFIG_DIR, CODEX_HOME or KIMI_CODE_HOME for several",
        ));
    }
    if opts.uninstall && harnesses.is_empty() {
        return Err(Error::usage(
            "`mcp --uninstall` needs at least one `--harness`",
        ));
    }
    let bin_dir = opts
        .bin_dir
        .clone()
        .unwrap_or_else(|| env.default_bin_dir());
    let servers: Vec<String> = crate::descriptor::KNOWN_SERVERS
        .iter()
        .map(|s| s.to_string())
        .collect();
    let home_of = |h: Harness| opts.home.clone().unwrap_or_else(|| env.harness_home(h));

    let roots = match &opts.roots {
        Some(raw) => parse_roots(raw).map_err(|e| Error::usage(format!("--roots: {e}")))?,
        None if !opts.uninstall && !harnesses.is_empty() && ui.interactive() => {
            let sep = if cfg!(windows) { ';' } else { ':' };
            let raw = ui.ask_text(&format!("Workspace roots for dispatch and palette, separated by '{sep}' (none: no roots)"), "");
            parse_roots(&raw).map_err(|e| Error::usage(format!("workspace roots: {e}")))?
        }
        None => Vec::new(),
    };

    let mode = opts.binaries.unwrap_or(Mode::Prebuilt);
    let request = binaries::Request {
        mode,
        names: servers.clone(),
        guard: true,
        bin_dir: bin_dir.clone(),
        slate_version: opts
            .slate_version
            .clone()
            .unwrap_or_else(|| "latest".to_string()),
        slate_dir: opts.slate_dir.clone(),
    };
    if !opts.uninstall && mode == Mode::Build && request.slate_dir.is_none() {
        return Err(
            Error::usage("`--binaries build` needs `--slate-dir <slate checkout>`")
                .with_state("nothing was changed"),
        );
    }

    ui.blank();
    ui.title("Summary");
    if opts.uninstall {
        for h in &harnesses {
            ui.line(&format!(
                "  Unregister aside, dispatch and palette from {} ({})",
                h.product(),
                home_of(*h).display()
            ));
        }
    } else {
        if mode != Mode::Skip {
            ui.line(&format!(
                "  Binaries ({}) into {}: {}",
                mode.name(),
                bin_dir.display(),
                request.installed_names(&env.platform()).join(", ")
            ));
        }
        for h in &harnesses {
            ui.line(&format!(
                "  Register {} with {} ({})",
                servers.join(", "),
                h.product(),
                home_of(*h).display()
            ));
        }
        if harnesses.is_empty() && mode == Mode::Skip {
            ui.line("  Nothing to do.");
        }
    }
    if opts.dry_run {
        ui.blank();
        ui.line("Dry run: nothing was changed.");
        return Ok(Report::default());
    }
    if !ui.confirm("Apply these changes?") {
        return Err(
            Error::new(crate::error::Kind::Aborted, "cancelled").with_state("nothing was changed")
        );
    }

    let mut report = Report::default();
    ui.blank();
    ui.title("Applying");
    if opts.uninstall {
        for h in &harnesses {
            let home = home_of(*h);
            let manifest =
                Manifest::load(&home, MCP_KIT, *h)?.unwrap_or_else(|| Manifest::new(MCP_KIT));
            let names: Vec<String> = {
                let recorded: Vec<String> = manifest
                    .registrations
                    .iter()
                    .filter(|r| r.harness == h.name() && r.kind == "mcp")
                    .map(|r| r.server.clone())
                    .collect();
                if recorded.is_empty() {
                    servers.clone()
                } else {
                    recorded
                }
            };
            serve::unserve(env, ui, *h, &home, &names, &manifest, &mut report)?;
            Manifest::delete(&home, MCP_KIT, *h)?;
            created::remove_empty(ui, &manifest.created_dirs);
        }
        return Ok(report);
    }

    if mode != Mode::Skip {
        let fetch: Box<dyn binaries::Fetch> = if mode == Mode::Prebuilt {
            Box::new(HttpFetch::new()?)
        } else {
            Box::new(NoFetch)
        };
        let got = binaries::install(env, ui, &request, fetch.as_ref())?;
        for p in &got.paths {
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
    }
    for h in &harnesses {
        let home = home_of(*h);
        let mut manifest =
            Manifest::load(&home, MCP_KIT, *h)?.unwrap_or_else(|| Manifest::new(MCP_KIT));
        let ctx = Ctx {
            harness: *h,
            home: home.clone(),
            default_home: env.default_home(*h),
            bin_dir: bin_dir.clone(),
            roots: roots.clone(),
            servers: servers.clone(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        };
        let input = serve::Input {
            config_file: config_file(*h, &home),
            ctx: &ctx,
            subagent: None,
        };
        let watch = created::watch_install(
            *h,
            &created::Targets {
                home: &home,
                rules: false,
                skills: false,
                bin_dir: Some(&bin_dir),
                servers: true,
            },
        );
        let manual_before = report.manual.len();
        ui.blank();
        ui.line(h.product());
        let _registered = serve::serve(env, ui, &input, &mut manifest, &mut report)?;
        if report.manual.len() > manual_before {
            report.problems.push(
                Error::command(format!(
                    "the servers could not be registered with {}",
                    h.product()
                ))
                .with_state("the binaries are installed; nothing is registered")
                .with_fix("run the commands listed below, or fix the cause and re-run"),
            );
        } else {
            report.done(h.name(), format!("registered in {}", home.display()));
        }
        watch.record(&mut manifest);
        manifest.roots = roots.clone();
        manifest.version = env!("CARGO_PKG_VERSION").to_string();
        manifest.save(&home)?;
    }
    Ok(report)
}
