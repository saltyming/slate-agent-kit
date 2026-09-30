//! Codex: MCP server registration through the `codex` CLI, and the
//! `config.toml` edits that make the servers usable.
//!
//! Owns the `codex mcp add/remove` command lines and the format-preserving
//! edits: aside's `tool_timeout_sec` and approval mode, the code-mode namespace
//! lists, per-tool approvals for palette's read-only tools, and the subagent
//! default model. It does not edit the `[mcp_servers.*]` tables' contents that
//! `codex mcp add` writes.
//!
//! Main entry points: [`register`], [`unregister`], [`config_edits`],
//! [`add_args`] and [`remove_args`].

use super::{
    Ctx, EditOutcome, Outcome, SubagentPrefs, run_cli, server_binary, server_env, shell_quote,
    stderr_text,
};
use crate::config::{ConfigDoc, Desired, TomlDoc, apply_desired, list_add, set_value};
use crate::env::{Env, Harness};
use crate::error::{Error, Result};
use crate::json::Json;
use crate::manifest::{Manifest, Registration};
use crate::ui::Ui;
use std::path::Path;

/// The tool-call timeout aside needs: a consultation blocks for the whole backend run.
pub const ASIDE_TOOL_TIMEOUT_SEC: i64 = 1800;

/// The MCP tool namespace Codex derives from the server name `aside`.
pub const ASIDE_NAMESPACE: &str = "mcp__aside";

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
    let home = shell_quote(&ctx.home.to_string_lossy());
    ctx.servers
        .iter()
        .map(|s| {
            let bin = server_binary(env, &ctx.bin_dir, s);
            let args = add_args(s, &bin, &server_env(ctx, s));
            let quoted: Vec<String> = args.iter().map(|a| shell_quote(a)).collect();
            format!("CODEX_HOME={home} {program} {}", quoted.join(" "))
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
                    "CODEX_HOME={} {program} mcp remove {s}",
                    shell_quote(&home.to_string_lossy())
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

/// Lists an MCP tool namespace in `[features.code_mode]` so Codex hands the
/// tools to the model as plain blocking calls.
///
/// Since Codex 0.144 the model reaches MCP tools from inside a code-mode `exec`
/// cell, which yields after `yield_time_ms` and leaves the model to poll. An
/// aside call blocks for the whole backend run, so the model would see "still
/// running" over and over and give up on an answer that was about to arrive.
/// Both lists are required: `excluded_tool_namespaces` alone drops the tools
/// from the model's view. Other entries are kept.
fn add_code_mode_namespace(
    doc: &mut TomlDoc,
    file: &Path,
    namespace: &str,
) -> Result<Vec<crate::config::Change>> {
    match doc.kind_at(&["features"]) {
        None | Some("table") => {}
        Some(other) => {
            return Err(Error::config(format!(
                "`features` in {} is a {other}, not a table, so [features.code_mode] cannot be added",
                file.display()
            )));
        }
    }
    match doc.kind_at(&["features", "code_mode"]) {
        None | Some("table") => {}
        Some(other) => {
            return Err(Error::config(format!(
                "[features] in {} sets a scalar `code_mode` ({other}), which collides with the [features.code_mode] table aside needs",
                file.display()
            ))
            .with_state("the file was not changed by this step")
            .with_fix(format!("remove the `code_mode = ...` line under [features] in {} and re-run `slate-setup configure`", file.display())));
        }
    }
    let items = vec![namespace.to_string()];
    let mut changes = Vec::new();
    for key in ["excluded_tool_namespaces", "direct_only_tool_namespaces"] {
        let path = ["features", "code_mode", key];
        changes.extend(list_add(doc, &path, &items).map_err(|e| {
            e.with_fix(format!(
                "make `{key}` under [features.code_mode] in {} a list of strings and re-run",
                file.display()
            ))
        })?);
    }
    Ok(changes)
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

    if manages("aside")
        && matches!(
            doc.get(&["mcp_servers", "aside"]),
            Ok(Some(Json::Object(_)))
        )
    {
        let res = atomic(doc, |d| {
            let mut c = Vec::new();
            // Every aside call blocks for the whole backend run, which would trip Codex's per-call timeout.
            c.extend(set_value(
                d,
                &["mcp_servers", "aside", "tool_timeout_sec"],
                Json::Number(ASIDE_TOOL_TIMEOUT_SEC.to_string()),
            )?);
            // A headless `codex exec` has nobody to approve an MCP call; it comes back as "user cancelled MCP tool call".
            // Every aside tool is a read-only advisor, so pre-approving them is safe.
            c.extend(set_value(
                d,
                &["mcp_servers", "aside", "default_tools_approval_mode"],
                Json::str("approve"),
            )?);
            Ok(c)
        });
        match res {
            Ok(c) => out.changes.extend(c),
            Err(e) => out.refusals.push(e),
        }
    }

    if manages("palette")
        && matches!(
            doc.get(&["mcp_servers", "palette"]),
            Ok(Some(Json::Object(_)))
        )
        && let Some(tools) = opts.palette_tools
    {
        let res = atomic(doc, |d| {
            let mut c = Vec::new();
            for t in tools {
                c.extend(set_value(
                    d,
                    &["mcp_servers", "palette", "tools", t, "approval_mode"],
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

    if manages("aside") {
        match atomic(doc, |d| add_code_mode_namespace(d, file, ASIDE_NAMESPACE)) {
            Ok(c) => out.changes.extend(c),
            Err(e) => out.refusals.push(e),
        }
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

    const CONFIG: &str = "# my codex config\nmodel = \"gpt-6\"  # main model\n\n[mcp_servers.aside]\ncommand = \"/b/aside\"\nargs = []\n\n[mcp_servers.aside.env]\nASIDE_HARNESS = \"codex\"\n\n[mcp_servers.palette]\ncommand = \"/b/palette\"\n\n[features.code_mode]\nexcluded_tool_namespaces = [\"mcp__other\"]\n\n[tui]\ntheme = \"dark\" # keep\n";

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
        vec!["aside".into(), "dispatch".into(), "palette".into()]
    }

    #[test]
    fn add_and_remove_command_lines() {
        assert_eq!(remove_args("aside"), ["mcp", "remove", "aside"]);
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
    fn edits_set_aside_keys_namespaces_and_palette_approvals_and_keep_the_rest() {
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
        let text = doc.render();
        assert!(text.starts_with("# my codex config\nmodel = \"gpt-6\"  # main model\n"));
        assert!(text.contains("tool_timeout_sec = 1800"));
        assert!(text.contains("default_tools_approval_mode = \"approve\""));
        assert!(text.contains("excluded_tool_namespaces = [\"mcp__other\", \"mcp__aside\"]"));
        assert!(text.contains("direct_only_tool_namespaces = [\"mcp__aside\"]"));
        assert!(
            text.contains("[mcp_servers.palette.tools.palette_read]")
                || text.contains("palette_read")
        );
        assert!(text.contains("approval_mode = \"approve\""));
        assert!(text.contains("theme = \"dark\" # keep"));
        // The [mcp_servers.aside.env] table stays intact and after the direct keys.
        let timeout = text.find("tool_timeout_sec").unwrap();
        let env_table = text.find("[mcp_servers.aside.env]").unwrap();
        assert!(timeout < env_table);
        // Re-parse to prove the result is valid TOML with the right values.
        let parsed: toml::Table = toml::from_str(&text).unwrap();
        assert_eq!(
            parsed["mcp_servers"]["aside"]["tool_timeout_sec"].as_integer(),
            Some(1800)
        );
        assert_eq!(
            parsed["mcp_servers"]["palette"]["tools"]["palette_read"]["approval_mode"].as_str(),
            Some("approve")
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
    fn a_missing_code_mode_table_is_created_at_the_end() {
        let file = Path::new("config.toml");
        let mut doc =
            TomlDoc::parse("model = \"x\"\n\n[mcp_servers.aside]\ncommand = \"a\"\n").unwrap();
        let s = servers();
        config_edits(
            &mut doc,
            file,
            &opts(&s, None, SubagentPrefs::default()),
            &Manifest::new("k"),
        );
        let text = doc.render();
        assert!(text.contains("[features.code_mode]\nexcluded_tool_namespaces = [\"mcp__aside\"]\ndirect_only_tool_namespaces = [\"mcp__aside\"]\n"), "{text}");
        assert!(
            !text.contains("[features]\n"),
            "the implicit parent table has no header: {text}"
        );
    }

    #[test]
    fn multiline_namespace_arrays_are_extended_not_refused() {
        let file = Path::new("config.toml");
        let src = "[features.code_mode]\nexcluded_tool_namespaces = [\n  \"mcp__other\",  # why\n  \"mcp__two\",\n]\n";
        let mut doc = TomlDoc::parse(src).unwrap();
        let s = servers();
        let out = config_edits(
            &mut doc,
            file,
            &opts(&s, None, SubagentPrefs::default()),
            &Manifest::new("k"),
        );
        assert!(out.refusals.is_empty());
        let text = doc.render();
        assert!(text.contains("\"mcp__other\",  # why"), "{text}");
        assert!(text.contains("mcp__aside"));
        let parsed: toml::Table = toml::from_str(&text).unwrap();
        let list = parsed["features"]["code_mode"]["excluded_tool_namespaces"]
            .as_array()
            .unwrap();
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn a_scalar_code_mode_is_refused_and_the_other_edits_still_apply() {
        let file = Path::new("/h/config.toml");
        let src = "[mcp_servers.aside]\ncommand = \"a\"\n\n[features]\ncode_mode = true\n";
        let mut doc = TomlDoc::parse(src).unwrap();
        let s = servers();
        let out = config_edits(
            &mut doc,
            file,
            &opts(
                &s,
                None,
                SubagentPrefs {
                    model: "gpt-6-astra".into(),
                    effort: "high".into(),
                },
            ),
            &Manifest::new("k"),
        );
        assert_eq!(out.refusals.len(), 1);
        let msg = out.refusals[0].to_string();
        assert!(msg.contains("scalar `code_mode`"), "{msg}");
        assert!(
            out.refusals[0]
                .fix
                .as_deref()
                .unwrap()
                .contains("remove the `code_mode = ...` line")
        );
        let text = doc.render();
        assert!(
            text.contains("code_mode = true"),
            "the scalar is never dropped: {text}"
        );
        assert!(!text.contains("excluded_tool_namespaces"));
        assert!(text.contains("tool_timeout_sec = 1800"));
        assert!(text.contains("default_subagent_model = \"gpt-6-astra\""));
        assert!(text.contains("default_subagent_reasoning_effort = \"high\""));
    }

    #[test]
    fn inline_code_mode_tables_are_edited_in_place() {
        let file = Path::new("config.toml");
        let mut doc = TomlDoc::parse("[features]\ncode_mode = { yield_time_ms = 5000 }\n").unwrap();
        let s = servers();
        let out = config_edits(
            &mut doc,
            file,
            &opts(&s, None, SubagentPrefs::default()),
            &Manifest::new("k"),
        );
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
        let parsed: toml::Table = toml::from_str(&doc.render()).unwrap();
        assert_eq!(
            parsed["features"]["code_mode"]["yield_time_ms"].as_integer(),
            Some(5000)
        );
        assert_eq!(
            parsed["features"]["code_mode"]["excluded_tool_namespaces"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn aside_keys_are_not_created_without_a_registered_table() {
        let file = Path::new("config.toml");
        let mut doc = TomlDoc::parse("model = \"x\"\n").unwrap();
        let s = servers();
        config_edits(
            &mut doc,
            file,
            &opts(&s, None, SubagentPrefs::default()),
            &Manifest::new("k"),
        );
        assert!(!doc.render().contains("tool_timeout_sec"));
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
        let out = register(&env, &mut ui, &ctx).unwrap();
        assert!(out.registered.is_empty());
        assert!(out.manual[0].starts_with("CODEX_HOME=/h/.codex /definitely/not/there mcp add aside --env ASIDE_HARNESS=codex -- "), "{:?}", out.manual);
    }
}
