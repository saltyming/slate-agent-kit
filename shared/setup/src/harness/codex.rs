//! Codex: MCP server registration through the `codex` CLI, and the
//! `config.toml` edits that make the servers usable.
//!
//! Owns the `codex mcp add/remove` command lines and the format-preserving
//! edits: per-tool approvals for dispatch's consultation and read tools and for
//! palette's read-only tools, and the subagent default model. It does not edit
//! the `[mcp_servers.*]` tables' contents that `codex mcp add` writes.
//!
//! Main entry points: [`register`], [`unregister`], [`config_edits`],
//! [`add_args`] and [`remove_args`].

use super::{
    Ctx, DISPATCH_PREAPPROVED_TOOLS, EditOutcome, Outcome, SubagentPrefs, arg_quote, env_prefix,
    run_cli, server_binary, server_env, stderr_text,
};
use crate::config::{ConfigDoc, Desired, TomlDoc, apply_desired, set_value};
use crate::env::{Env, Harness};
use crate::error::{Error, Result};
use crate::json::Json;
use crate::manifest::{Manifest, Registration};
use crate::ui::Ui;
use std::path::Path;

/// `codex mcp remove <server>`.
pub fn remove_args(server: &str) -> Vec<String> {
    ["mcp", "remove", server]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// `codex mcp add <server> --env K=V ... -- <binary>`.
pub fn add_args(server: &str, binary: &Path, env: &[(String, String)]) -> Vec<String> {
    let mut a: Vec<String> = ["mcp", "add", server]
        .iter()
        .map(|s| s.to_string())
        .collect();
    for (k, v) in env {
        a.push("--env".into());
        a.push(format!("{k}={v}"));
    }
    a.push("--".into());
    a.push(binary.to_string_lossy().into_owned());
    a
}

fn manual_commands(program: &str, ctx: &Ctx, env: &Env) -> Vec<String> {
    let platform = env.platform();
    let prefix = env_prefix(&platform, "CODEX_HOME", &ctx.home.to_string_lossy());
    ctx.servers
        .iter()
        .map(|s| {
            let bin = server_binary(env, &ctx.bin_dir, s);
            let args = add_args(s, &bin, &server_env(ctx, s));
            let quoted: Vec<String> = args.iter().map(|a| arg_quote(&platform, a)).collect();
            format!("{prefix}{program} {}", quoted.join(" "))
        })
        .collect()
}

/// Registers the servers of `ctx` with Codex.
pub fn register(env: &Env, ui: &mut Ui, ctx: &Ctx) -> Result<Outcome> {
    let mut out = Outcome::default();
    let Some(program) = env.cli_program(Harness::Codex) else {
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
        .map_err(|e| Error::io(format!("creating {}", ctx.state_home().display()), e))?;
    let home_env = vec![(
        "CODEX_HOME".to_string(),
        ctx.home.to_string_lossy().into_owned(),
    )];
    for server in &ctx.servers {
        let bin = server_binary(env, &ctx.bin_dir, server);
        ui.detail(&format!("codex mcp add {server}"));
        run_cli(env, &program, &remove_args(server), &home_env, &[])?;
        let args = add_args(server, &bin, &server_env(ctx, server));
        let res = run_cli(env, &program, &args, &home_env, &[])?;
        if !res.status.success() {
            let state = if out.servers_done.is_empty() {
                "none".to_string()
            } else {
                out.servers_done.join(", ")
            };
            out.failure = Some(
                Error::command(format!(
                    "`codex mcp add {server}` failed: {}",
                    stderr_text(&res)
                ))
                .with_state(format!("registered before the failure: {state}"))
                .with_fix(format!(
                    "check `{program} mcp add --help` and re-run `slate-setup configure`"
                )),
            );
            break;
        }
        out.servers_done.push(server.clone());
        out.registered.push(Registration {
            kind: "mcp".into(),
            harness: "codex".into(),
            server: server.clone(),
            path: None,
        });
    }
    if ctx.roots.is_empty() {
        out.notes.push(
            "no workspace roots were given: if Codex starts MCP servers outside your project, dispatch and palette reject every working directory until you run `slate-setup configure --roots <workspace root>`"
                .to_string(),
        );
    }
    Ok(out)
}

/// Removes the registrations of `servers` from Codex.
pub fn unregister(env: &Env, ui: &mut Ui, home: &Path, servers: &[String]) -> Result<Outcome> {
    let mut out = Outcome::default();
    let Some(program) = env.cli_program(Harness::Codex) else {
        return Ok(out);
    };
    if env.which(&program).is_none() {
        out.notes.push(format!(
            "`{program}` was not found on PATH; the servers were not unregistered"
        ));
        out.manual = servers
            .iter()
            .map(|s| {
                format!(
                    "{}{program} mcp remove {s}",
                    env_prefix(&env.platform(), "CODEX_HOME", &home.to_string_lossy())
                )
            })
            .collect();
        return Ok(out);
    }
    let home_env = vec![(
        "CODEX_HOME".to_string(),
        home.to_string_lossy().into_owned(),
    )];
    for s in servers {
        ui.detail(&format!("codex mcp remove {s}"));
        let res = run_cli(env, &program, &remove_args(s), &home_env, &[])?;
        if res.status.success() {
            out.servers_done.push(s.clone());
        } else {
            out.refused.push((s.clone(), stderr_text(&res)));
        }
    }
    Ok(out)
}

/// What the Codex edits need to know about this run.
#[derive(Debug, Clone, Default)]
pub struct Opts<'a> {
    /// Servers managed in this run (registered, or already present).
    pub servers: &'a [String],
    /// Palette read-only tool names; `None` when they could not be read.
    pub palette_tools: Option<&'a [String]>,
    /// The subagent default from `subagent-prefs.md`; `None` leaves the native keys alone.
    pub subagent: Option<SubagentPrefs>,
}

/// Runs `edit` on a copy of `doc` and keeps the result only if it succeeds.
fn atomic<T>(doc: &mut TomlDoc, edit: impl FnOnce(&mut TomlDoc) -> Result<T>) -> Result<T> {
    let mut work = doc.clone();
    let value = edit(&mut work)?;
    *doc = work;
    Ok(value)
}

/// Pre-approves `tools` of `server` as `[mcp_servers.<server>.tools.<tool>] approval_mode = "approve"`.
///
/// Only when `codex mcp add` wrote the server's table: a `tools` table on its
/// own would be a server Codex cannot start. A headless `codex exec` has nobody
/// to approve an MCP call, so an unapproved one comes back as "user cancelled
/// MCP tool call"; the server-level approval stays at Codex's default.
fn approve_tools(doc: &mut TomlDoc, server: &str, tools: &[&str], out: &mut EditOutcome) {
    if !matches!(doc.get(&["mcp_servers", server]), Ok(Some(Json::Object(_)))) {
        return;
    }
    let res = atomic(doc, |d| {
        let mut c = Vec::new();
        for t in tools {
            c.extend(set_value(
                d,
                &["mcp_servers", server, "tools", t, "approval_mode"],
                Json::str("approve"),
            )?);
        }
        Ok(c)
    });
    match res {
        Ok(c) => out.changes.extend(c),
        Err(e) => out.refusals.push(e),
    }
}

/// Edits `config.toml`. Each group is applied on its own; a refused group leaves the file as it was for that group.
pub fn config_edits(
    doc: &mut TomlDoc,
    file: &Path,
    opts: &Opts<'_>,
    manifest: &Manifest,
) -> EditOutcome {
    let mut out = EditOutcome::default();
    let manages = |s: &str| opts.servers.iter().any(|x| x == s);

    if manages("dispatch") {
        approve_tools(doc, "dispatch", &DISPATCH_PREAPPROVED_TOOLS, &mut out);
    }

    if manages("palette")
        && let Some(tools) = opts.palette_tools
    {
        let tools: Vec<&str> = tools.iter().map(String::as_str).collect();
        approve_tools(doc, "palette", &tools, &mut out);
    }

    let Some(sub) = &opts.subagent else {
        return out;
    };
    for (key, value) in [
        ("default_subagent_model", &sub.model),
        ("default_subagent_reasoning_effort", &sub.effort),
    ] {
        let path = ["agents", key];
        let desired = if value.is_empty() {
            Desired::Unset
        } else {
            Desired::Set(Json::str(value.clone()))
        };
        let res = atomic(doc, |d| {
            apply_desired(d, &path, &desired, manifest.config_record(file, &path))
        });
        match res {
            Ok((change, undone)) => {
                out.changes.extend(change);
                if undone.is_some() {
                    out.undone
                        .push(path.iter().map(|s| s.to_string()).collect());
                }
            }
            Err(e) => out.refusals.push(e.with_fix(format!(
                "make [agents] in {} a table and re-run",
                file.display()
            ))),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::test_ctx;

    const CONFIG: &str = "# my codex config\nmodel = \"gpt-6\"  # main model\n\n[mcp_servers.dispatch]\ncommand = \"/b/dispatch\"\nargs = []\n\n[mcp_servers.dispatch.env]\nSLATE_HARNESS = \"codex\"\n\n[mcp_servers.palette]\ncommand = \"/b/palette\"\n\n[features.code_mode]\nexcluded_tool_namespaces = [\"mcp__other\"]\n\n[tui]\ntheme = \"dark\" # keep\n";

    fn opts<'a>(
        servers: &'a [String],
        tools: Option<&'a [String]>,
        sub: SubagentPrefs,
    ) -> Opts<'a> {
        Opts {
            servers,
            palette_tools: tools,
            subagent: Some(sub),
        }
    }

    fn servers() -> Vec<String> {
        vec!["dispatch".into(), "palette".into()]
    }

    #[test]
    fn add_and_remove_command_lines() {
        assert_eq!(remove_args("dispatch"), ["mcp", "remove", "dispatch"]);
        let a = add_args(
            "dispatch",
            Path::new("/b/dispatch"),
            &[("A".into(), "1".into())],
        );
        assert_eq!(
            a,
            [
                "mcp",
                "add",
                "dispatch",
                "--env",
                "A=1",
                "--",
                "/b/dispatch"
            ]
        );
    }

    #[test]
    fn edits_approve_dispatch_and_palette_tools_and_keep_the_rest() {
        let file = Path::new("config.toml");
        let mut doc = TomlDoc::parse(CONFIG).unwrap();
        let tools = vec!["palette_read".to_string()];
        let s = servers();
        let out = config_edits(
            &mut doc,
            file,
            &opts(&s, Some(&tools), SubagentPrefs::default()),
            &Manifest::new("k"),
        );
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
        assert_eq!(out.changes.len(), DISPATCH_PREAPPROVED_TOOLS.len() + 1);
        let text = doc.render();
        assert!(text.starts_with("# my codex config\nmodel = \"gpt-6\"  # main model\n"));
        assert!(text.contains("theme = \"dark\" # keep"));
        for t in DISPATCH_PREAPPROVED_TOOLS {
            assert!(
                text.contains(&format!(
                    "[mcp_servers.dispatch.tools.{t}]\napproval_mode = \"approve\"\n"
                )),
                "{t}: {text}"
            );
        }
        assert!(
            text.contains(
                "[mcp_servers.palette.tools.palette_read]\napproval_mode = \"approve\"\n"
            ),
            "{text}"
        );
        // Nothing else is written: no timeout, no server-level approval, no namespaces.
        assert!(!text.contains("tool_timeout_sec"), "{text}");
        assert!(!text.contains("default_tools_approval_mode"), "{text}");
        assert!(!text.contains("direct_only_tool_namespaces"), "{text}");
        let parsed: toml::Table = toml::from_str(&text).unwrap();
        let dispatch = parsed["mcp_servers"]["dispatch"].as_table().unwrap();
        assert_eq!(dispatch["env"]["SLATE_HARNESS"].as_str(), Some("codex"));
        let mut approved: Vec<&str> = dispatch["tools"]
            .as_table()
            .unwrap()
            .iter()
            .filter(|(_, v)| v["approval_mode"].as_str() == Some("approve"))
            .map(|(k, _)| k.as_str())
            .collect();
        approved.sort_unstable();
        let mut want = DISPATCH_PREAPPROVED_TOOLS;
        want.sort_unstable();
        assert_eq!(approved, want);
        assert_eq!(
            parsed["features"]["code_mode"]["excluded_tool_namespaces"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn a_second_run_changes_nothing() {
        let file = Path::new("config.toml");
        let mut doc = TomlDoc::parse(CONFIG).unwrap();
        let s = servers();
        let tools = vec!["t".to_string()];
        config_edits(
            &mut doc,
            file,
            &opts(&s, Some(&tools), SubagentPrefs::default()),
            &Manifest::new("k"),
        );
        let once = doc.render();
        let out = config_edits(
            &mut doc,
            file,
            &opts(&s, Some(&tools), SubagentPrefs::default()),
            &Manifest::new("k"),
        );
        assert!(out.changes.is_empty());
        assert_eq!(doc.render(), once);
    }

    #[test]
    fn approvals_are_not_written_without_a_registered_table() {
        let file = Path::new("config.toml");
        let mut doc = TomlDoc::parse("model = \"x\"\n").unwrap();
        let s = servers();
        let tools = vec!["t".to_string()];
        let out = config_edits(
            &mut doc,
            file,
            &opts(&s, Some(&tools), SubagentPrefs::default()),
            &Manifest::new("k"),
        );
        assert!(out.changes.is_empty());
        assert_eq!(doc.render(), "model = \"x\"\n");
    }

    #[test]
    fn a_table_that_is_not_a_table_is_refused_and_the_other_edits_still_apply() {
        let file = Path::new("/h/config.toml");
        let src = "[mcp_servers.dispatch]\ncommand = \"d\"\ntools = 3\n\n[mcp_servers.palette]\ncommand = \"p\"\n";
        let mut doc = TomlDoc::parse(src).unwrap();
        let s = servers();
        let tools = vec!["palette_read".to_string()];
        let out = config_edits(
            &mut doc,
            file,
            &opts(
                &s,
                Some(&tools),
                SubagentPrefs {
                    model: "gpt-6-astra".into(),
                    effort: "high".into(),
                },
            ),
            &Manifest::new("k"),
        );
        assert_eq!(out.refusals.len(), 1, "{:?}", out.refusals);
        let text = doc.render();
        assert!(
            text.contains("tools = 3"),
            "the dispatch group is left as it was: {text}"
        );
        assert!(text.contains("[mcp_servers.palette.tools.palette_read]"));
        assert!(text.contains("default_subagent_model = \"gpt-6-astra\""));
    }

    #[test]
    fn subagent_keys_are_written_and_restored_when_unset() {
        let file = Path::new("config.toml");
        let mut doc = TomlDoc::parse("[agents]\nmax_threads = 4\n").unwrap();
        let mut manifest = Manifest::new("k");
        let none: Vec<String> = Vec::new();
        let on = SubagentPrefs {
            model: "gpt-6-astra".into(),
            effort: "xhigh".into(),
        };
        let out = config_edits(&mut doc, file, &opts(&none, None, on), &manifest);
        assert_eq!(out.changes.len(), 2);
        for c in &out.changes {
            manifest.record_config(c.record(file));
        }
        assert!(
            doc.render()
                .contains("default_subagent_model = \"gpt-6-astra\"")
        );
        let out = config_edits(
            &mut doc,
            file,
            &opts(&none, None, SubagentPrefs::default()),
            &manifest,
        );
        assert_eq!(out.undone.len(), 2);
        assert_eq!(doc.render(), "[agents]\nmax_threads = 4\n");
    }

    #[test]
    fn missing_cli_prints_commands_with_codex_home() {
        let mut env = Env::for_home("/h");
        env.set_var("CODEX_BIN", "/definitely/not/there");
        let ctx = test_ctx(Harness::Codex, "/h/.codex", "/h/.codex");
        let mut ui = Ui::captured(crate::ui::new_sink());
        env.set_var("SLATE_PLATFORM", "x86_64-unknown-linux-gnu");
        let out = register(&env, &mut ui, &ctx).unwrap();
        assert!(out.registered.is_empty());
        let state = Path::new("/h/.codex").join("slate-agent-kit");
        assert!(
            out.manual[0].starts_with(&format!(
                "CODEX_HOME=/h/.codex /definitely/not/there mcp add dispatch --env SLATE_AGENT_STATE_HOME={} --env SLATE_HARNESS=codex -- ",
                state.display()
            )),
            "{:?}",
            out.manual
        );
        env.set_var("SLATE_PLATFORM", "x86_64-pc-windows-msvc");
        let ps = register(&env, &mut ui, &ctx).unwrap();
        assert!(
            ps.manual[0].starts_with("$env:CODEX_HOME='/h/.codex'; "),
            "{:?}",
            ps.manual
        );
    }
}
