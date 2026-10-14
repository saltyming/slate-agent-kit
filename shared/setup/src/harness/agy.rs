//! The Antigravity CLI: MCP server registration through `agy mcp`, and the
//! per-tool approvals in `antigravity-cli/settings.json`.
//!
//! Owns the `agy mcp add/remove` command lines, the process home the CLI runs
//! with (agy has no home variable of its own; it follows `HOME`, or
//! `USERPROFILE` on Windows), the `permissions.allow` entries for dispatch's
//! consultation and read tools and palette's read-only tools, and the removal
//! of a settings file the installer created once nothing of anyone else's is
//! left in it. It does not edit `config/mcp_config.json`; only the CLI does.
//!
//! Main entry points: [`register`], [`unregister`], [`settings_edits`],
//! [`permission_entries`], [`add_args`], [`remove_args`] and
//! [`remove_created_settings`].

use super::{
    Ctx, DISPATCH_PREAPPROVED_TOOLS, EditOutcome, Outcome, arg_quote, run_cli, server_binary,
    server_env, stderr_text,
};
use crate::config::{JsonDoc, list_add};
use crate::env::{Env, Harness, process_home_var};
use crate::error::{Error, IoContext, Result};
use crate::json::Json;
use crate::manifest::Registration;
use crate::ui::Ui;
use crate::util::read_text_opt;
use std::path::Path;

/// The palette tools pre-approved for agy: the read-only ones.
///
/// agy grants approval per tool as `mcp(<server>/<tool>)`, and RFC-0023 names
/// these four; Claude Code and Codex read the same set from
/// `palette --read-only-tools`.
pub const PALETTE_PREAPPROVED_TOOLS: [&str; 4] = [
    "palette_status",
    "palette_lint",
    "palette_layout",
    "palette_template",
];

/// `agy mcp remove <server>`.
pub fn remove_args(server: &str) -> Vec<String> {
    ["mcp", "remove", server]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// `agy mcp add --env K=V ... <server> <binary>`: the flags come before the name.
pub fn add_args(server: &str, binary: &Path, env: &[(String, String)]) -> Vec<String> {
    let mut a: Vec<String> = vec!["mcp".into(), "add".into()];
    for (k, v) in env {
        a.push("--env".into());
        a.push(format!("{k}={v}"));
    }
    a.push(server.to_string());
    a.push(binary.to_string_lossy().into_owned());
    a
}

/// The process home the CLI runs with: the one the installer resolved, so the
/// registration lands in the same `.gemini` folder the files went to.
fn home_env(env: &Env) -> Vec<(String, String)> {
    vec![(
        process_home_var().to_string(),
        env.home.to_string_lossy().into_owned(),
    )]
}

fn manual_commands(program: &str, ctx: &Ctx, env: &Env) -> Vec<String> {
    let platform = env.platform();
    ctx.servers
        .iter()
        .map(|s| {
            let bin = server_binary(env, &ctx.bin_dir, s);
            let args = add_args(s, &bin, &server_env(ctx, s));
            let quoted: Vec<String> = args.iter().map(|a| arg_quote(&platform, a)).collect();
            format!("{program} {}", quoted.join(" "))
        })
        .collect()
}

/// Registers the servers of `ctx` with agy.
pub fn register(env: &Env, ui: &mut Ui, ctx: &Ctx) -> Result<Outcome> {
    let mut out = Outcome::default();
    let Some(program) = env.cli_program(Harness::Agy) else {
        return Ok(out);
    };
    if env.which(&program).is_none() {
        out.notes.push(format!(
            "`{program}` was not found on PATH; nothing was registered"
        ));
        out.manual = manual_commands(&program, ctx, env);
        return Ok(out);
    }
    std::fs::create_dir_all(ctx.state_home())
        .ctx(|| format!("creating {}", ctx.state_home().display()))?;
    let home = home_env(env);
    for server in &ctx.servers {
        let bin = server_binary(env, &ctx.bin_dir, server);
        ui.detail(&format!("agy mcp add {server}"));
        // A missing registration makes `remove` fail; that is expected.
        run_cli(env, &program, &remove_args(server), &home, &[])?;
        let args = add_args(server, &bin, &server_env(ctx, server));
        let res = run_cli(env, &program, &args, &home, &[])?;
        if !res.status.success() {
            let state = if out.servers_done.is_empty() {
                "none".to_string()
            } else {
                out.servers_done.join(", ")
            };
            let fix = manual_commands(&program, ctx, env)
                .into_iter()
                .find(|c| c.contains(&format!(" {server} ")))
                .unwrap_or_default();
            out.failure = Some(
                Error::command(format!(
                    "`agy mcp add {server}` failed: {}",
                    stderr_text(&res)
                ))
                .with_state(format!("registered before the failure: {state}"))
                .with_fix(format!("run: {fix}")),
            );
            break;
        }
        out.servers_done.push(server.clone());
        out.registered.push(Registration {
            kind: "mcp".into(),
            harness: "agy".into(),
            server: server.clone(),
            path: None,
        });
    }
    Ok(out)
}

/// Removes the registrations of `servers` from agy.
pub fn unregister(env: &Env, ui: &mut Ui, servers: &[String]) -> Result<Outcome> {
    let mut out = Outcome::default();
    let Some(program) = env.cli_program(Harness::Agy) else {
        return Ok(out);
    };
    if env.which(&program).is_none() {
        out.notes.push(format!(
            "`{program}` was not found on PATH; the servers were not unregistered"
        ));
        out.manual = servers
            .iter()
            .map(|s| format!("{program} mcp remove {s}"))
            .collect();
        return Ok(out);
    }
    let home = home_env(env);
    for s in servers {
        ui.detail(&format!("agy mcp remove {s}"));
        let res = run_cli(env, &program, &remove_args(s), &home, &[])?;
        if res.status.success() {
            out.servers_done.push(s.clone());
        } else {
            out.refused.push((s.clone(), stderr_text(&res)));
        }
    }
    Ok(out)
}

/// The `permissions.allow` entries for the servers registered: dispatch's
/// consultation and read tools, then palette's read-only tools, as
/// `mcp(<server>/<tool>)`.
pub fn permission_entries(dispatch: bool, palette: bool) -> Vec<String> {
    let mut out = Vec::new();
    if dispatch {
        out.extend(
            DISPATCH_PREAPPROVED_TOOLS
                .iter()
                .map(|t| format!("mcp(dispatch/{t})")),
        );
    }
    if palette {
        out.extend(
            PALETTE_PREAPPROVED_TOOLS
                .iter()
                .map(|t| format!("mcp(palette/{t})")),
        );
    }
    out
}

/// Edits `settings.json`: adds the approvals of the registered servers to
/// `permissions.allow`, in one edit, and touches nothing else.
pub fn settings_edits(
    doc: &mut JsonDoc,
    file: &Path,
    dispatch: bool,
    palette: bool,
) -> EditOutcome {
    let mut out = EditOutcome::default();
    let entries = permission_entries(dispatch, palette);
    if entries.is_empty() {
        return out;
    }
    let mut work = doc.clone();
    match list_add(&mut work, &["permissions", "allow"], &entries) {
        Ok(Some(c)) => {
            *doc = work;
            out.changes.push(c);
        }
        Ok(None) => {}
        Err(e) => out.refusals.push(e.with_fix(format!(
            "make `permissions.allow` in {} an array and re-run",
            file.display()
        ))),
    }
    out
}

/// Removes `file`, a settings file the installer created, when it holds an
/// empty JSON object, which is what remains once the installer's entries are
/// restored. Returns whether it was removed; a file with anything else in it stays.
pub fn remove_created_settings(file: &Path) -> Result<bool> {
    let Some(text) = read_text_opt(file)? else {
        return Ok(false);
    };
    let empty = matches!(Json::parse(&text), Ok(Json::Object(m)) if m.is_empty());
    if !empty {
        return Ok(false);
    }
    std::fs::remove_file(file).ctx(|| format!("removing {}", file.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::claude::parse_settings;
    use crate::harness::test_ctx;

    #[test]
    fn command_lines_put_the_flags_before_the_name() {
        assert_eq!(remove_args("palette"), ["mcp", "remove", "palette"]);
        let a = add_args(
            "dispatch",
            Path::new("/b/dispatch"),
            &[
                ("SLATE_HARNESS".into(), "agy".into()),
                ("B".into(), "two words".into()),
            ],
        );
        assert_eq!(
            a,
            [
                "mcp",
                "add",
                "--env",
                "SLATE_HARNESS=agy",
                "--env",
                "B=two words",
                "dispatch",
                "/b/dispatch"
            ]
        );
        assert_eq!(
            add_args("palette", Path::new("/b/palette"), &[]),
            ["mcp", "add", "palette", "/b/palette"]
        );
    }

    #[test]
    fn the_nine_approvals() {
        assert_eq!(
            permission_entries(true, true),
            [
                "mcp(dispatch/dispatch_consult)",
                "mcp(dispatch/dispatch_status)",
                "mcp(dispatch/dispatch_list)",
                "mcp(dispatch/dispatch_logs)",
                "mcp(dispatch/dispatch_backends)",
                "mcp(palette/palette_status)",
                "mcp(palette/palette_lint)",
                "mcp(palette/palette_layout)",
                "mcp(palette/palette_template)",
            ]
        );
        assert_eq!(permission_entries(false, true).len(), 4);
        assert!(permission_entries(false, false).is_empty());
    }

    #[test]
    fn settings_edits_touch_only_the_allow_list() {
        let file = Path::new("settings.json");
        let text = "{\n  \"agentMode\": \"fast\",\n  \"permissions\": {\n    \"allow\": [\"command(ls)\"],\n    \"deny\": []\n  },\n  \"toolPermission\": {}\n}\n";
        let mut doc = parse_settings(Some(text), file).unwrap();
        let out = settings_edits(&mut doc, file, true, true);
        assert!(out.refusals.is_empty());
        assert_eq!(out.changes.len(), 1);
        assert_eq!(out.changes[0].path, ["permissions", "allow"]);
        assert_eq!(out.changes[0].added, permission_entries(true, true));
        let new = doc.render_original();
        assert!(new.starts_with("{\n  \"agentMode\": \"fast\",\n  \"permissions\""));
        assert!(new.contains("\"command(ls)\""));
        assert!(new.contains("\"deny\": []"));
        assert!(new.contains("\"toolPermission\": {}"));
        assert!(
            !new.contains("dispatch_submit"),
            "submitting keeps its prompt"
        );
        // A second run adds nothing.
        let again = settings_edits(&mut doc, file, true, true);
        assert!(again.changes.is_empty());
    }

    #[test]
    fn a_non_array_allow_list_is_refused() {
        let file = Path::new("settings.json");
        let mut doc = parse_settings(Some("{\"permissions\":{\"allow\":\"x\"}}"), file).unwrap();
        let out = settings_edits(&mut doc, file, true, false);
        assert_eq!(out.refusals.len(), 1);
        assert!(doc.render_original().contains("\"allow\": \"x\""));
    }

    #[test]
    fn a_missing_cli_prints_commands_without_running_anything() {
        let mut env = Env::for_home("/h");
        env.set_var("AGY_BIN", "/definitely/not/there");
        let ctx = test_ctx(Harness::Agy, "/h/.gemini", "/h/.gemini");
        let mut ui = Ui::captured(crate::ui::new_sink());
        let out = register(&env, &mut ui, &ctx).unwrap();
        assert!(out.registered.is_empty());
        assert_eq!(out.manual.len(), 2);
        assert!(
            out.manual[0].contains("mcp add --env SLATE_AGENT_STATE_HOME="),
            "{:?}",
            out.manual
        );
        assert!(out.manual[0].contains("--env SLATE_HARNESS=agy dispatch "));
        assert!(out.notes[0].contains("was not found"));
        let out = unregister(&env, &mut ui, &["dispatch".to_string()]).unwrap();
        assert_eq!(out.manual, ["/definitely/not/there mcp remove dispatch"]);
    }

    #[test]
    fn only_an_emptied_settings_file_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("settings.json");
        std::fs::write(&f, "{}\n").unwrap();
        assert!(remove_created_settings(&f).unwrap());
        assert!(!f.exists());
        std::fs::write(&f, "{\"agentMode\": \"x\"}\n").unwrap();
        assert!(!remove_created_settings(&f).unwrap());
        assert!(f.exists());
        assert!(!remove_created_settings(&dir.path().join("absent.json")).unwrap());
    }
}
