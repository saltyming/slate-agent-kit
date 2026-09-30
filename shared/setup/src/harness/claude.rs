//! Claude Code: MCP server registration through the `claude` CLI, and the
//! `settings.json` edits (palette read-only permissions, subagent model).
//!
//! Owns the `claude mcp add/remove` command lines, the rule for which config
//! folder the CLI acts on, and the JSON edits. It does not edit `.claude.json`
//! directly; only the CLI does.
//!
//! Main entry points: [`register`], [`unregister`], [`settings_edits`],
//! [`cli_scope`] and [`CliScope`].

use super::{
    Ctx, EditOutcome, Outcome, SubagentPrefs, env_prefix, run_cli, server_binary, server_env,
    shell_quote, stderr_text,
};
use crate::config::{ConfigDoc, Desired, JsonDoc, apply_desired, list_add};
use crate::env::{Env, Harness};
use crate::error::{Error, Result};
use crate::json::Json;
use crate::manifest::{Manifest, Registration};
use crate::ui::Ui;
use std::path::Path;

/// Which configuration folder a `claude` invocation would act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliScope {
    /// The CLI already acts on the target home (its own `CLAUDE_CONFIG_DIR` matches).
    Inherit,
    /// The target home is the default; the invocation must not inherit `CLAUDE_CONFIG_DIR`.
    Default,
    /// The target home differs from what the CLI would use. Whether Claude Code
    /// keeps user-scope MCP servers under `CLAUDE_CONFIG_DIR` is not documented,
    /// so the installer does not register and prints the commands instead.
    Unsupported,
}

/// Decides which configuration folder the CLI acts on for `home`.
pub fn cli_scope(env: &Env, home: &Path) -> CliScope {
    let default_home = env.default_home(Harness::Claude);
    let inherited = env.get("CLAUDE_CONFIG_DIR").map(std::path::PathBuf::from);
    match inherited {
        Some(dir) if crate::util::same_path(&dir, home) => CliScope::Inherit,
        _ if crate::util::same_path(&default_home, home) => CliScope::Default,
        _ => CliScope::Unsupported,
    }
}

fn scope_env(scope: CliScope) -> Vec<&'static str> {
    if scope == CliScope::Default {
        vec!["CLAUDE_CONFIG_DIR"]
    } else {
        Vec::new()
    }
}

/// `claude mcp remove <server> -s user`.
pub fn remove_args(server: &str) -> Vec<String> {
    ["mcp", "remove", server, "-s", "user"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// `claude mcp add <server> -s user --transport stdio -e K=V ... -- <binary>`.
pub fn add_args(server: &str, binary: &Path, env: &[(String, String)]) -> Vec<String> {
    let mut a: Vec<String> = ["mcp", "add", server, "-s", "user", "--transport", "stdio"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    for (k, v) in env {
        a.push("-e".into());
        a.push(format!("{k}={v}"));
    }
    a.push("--".into());
    a.push(binary.to_string_lossy().into_owned());
    a
}

fn manual_commands(program: &str, ctx: &Ctx, env: &Env, with_config_dir: bool) -> Vec<String> {
    let prefix = if with_config_dir {
        env_prefix(
            &env.platform(),
            "CLAUDE_CONFIG_DIR",
            &ctx.home.to_string_lossy(),
        )
    } else {
        String::new()
    };
    ctx.servers
        .iter()
        .map(|s| {
            let bin = server_binary(env, &ctx.bin_dir, s);
            let args = add_args(s, &bin, &server_env(ctx, s));
            let quoted: Vec<String> = args.iter().map(|a| shell_quote(a)).collect();
            format!("{prefix}{program} {}", quoted.join(" "))
        })
        .collect()
}

/// Registers the servers of `ctx` with Claude Code.
pub fn register(env: &Env, ui: &mut Ui, ctx: &Ctx) -> Result<Outcome> {
    let mut out = Outcome::default();
    let Some(program) = env.cli_program(Harness::Claude) else {
        return Ok(out);
    };
    let scope = cli_scope(env, &ctx.home);
    if scope == CliScope::Unsupported {
        out.notes.push(format!(
            "{} is not the folder `claude` uses, and Claude Code does not document that user-scope MCP servers follow CLAUDE_CONFIG_DIR; nothing was registered",
            ctx.home.display()
        ));
        out.manual = manual_commands(&program, ctx, env, true);
        return Ok(out);
    }
    if env.which(&program).is_none() {
        out.notes.push(format!(
            "`{program}` was not found on PATH; nothing was registered"
        ));
        out.manual = manual_commands(&program, ctx, env, false);
        return Ok(out);
    }
    let remove_env = scope_env(scope);
    for server in &ctx.servers {
        let bin = server_binary(env, &ctx.bin_dir, server);
        ui.detail(&format!("claude mcp add {server}"));
        // A missing registration makes `remove` fail; that is expected.
        run_cli(env, &program, &remove_args(server), &[], &remove_env)?;
        let args = add_args(server, &bin, &server_env(ctx, server));
        let res = run_cli(env, &program, &args, &[], &remove_env)?;
        if !res.status.success() {
            let state = if out.servers_done.is_empty() {
                "none".to_string()
            } else {
                out.servers_done.join(", ")
            };
            let fix = manual_commands(&program, ctx, env, false)
                .into_iter()
                .find(|c| c.contains(&format!(" add {server} ")))
                .unwrap_or_default();
            out.failure = Some(
                Error::command(format!(
                    "`claude mcp add {server}` failed: {}",
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
            harness: "claude".into(),
            server: server.clone(),
            path: None,
        });
    }
    Ok(out)
}

/// Removes the registrations of `servers`. Returns commands to run by hand when the CLI cannot act.
pub fn unregister(env: &Env, ui: &mut Ui, home: &Path, servers: &[String]) -> Result<Outcome> {
    let mut out = Outcome::default();
    let Some(program) = env.cli_program(Harness::Claude) else {
        return Ok(out);
    };
    let scope = cli_scope(env, home);
    let manual = |with_dir: bool| -> Vec<String> {
        let prefix = if with_dir {
            env_prefix(
                &env.platform(),
                "CLAUDE_CONFIG_DIR",
                &home.to_string_lossy(),
            )
        } else {
            String::new()
        };
        servers
            .iter()
            .map(|s| format!("{prefix}{program} mcp remove {s} -s user"))
            .collect()
    };
    if scope == CliScope::Unsupported {
        out.notes.push(format!(
            "{} is not the folder `claude` uses; the servers were not unregistered",
            home.display()
        ));
        out.manual = manual(true);
        return Ok(out);
    }
    if env.which(&program).is_none() {
        out.notes.push(format!(
            "`{program}` was not found on PATH; the servers were not unregistered"
        ));
        out.manual = manual(false);
        return Ok(out);
    }
    for s in servers {
        ui.detail(&format!("claude mcp remove {s}"));
        let res = run_cli(env, &program, &remove_args(s), &[], &scope_env(scope))?;
        if res.status.success() {
            out.servers_done.push(s.clone());
        }
    }
    Ok(out)
}

/// `permissions.allow` entries for the palette server's read-only tools.
pub fn palette_permission_entries(tools: &[String]) -> Vec<String> {
    tools.iter().map(|t| format!("mcp__palette__{t}")).collect()
}

/// Edits `settings.json`: palette read-only permissions and the subagent default model.
///
/// `palette_tools` is `None` when the palette server is not registered.
pub fn settings_edits(
    doc: &mut JsonDoc,
    file: &Path,
    palette_tools: Option<&[String]>,
    subagent: Option<&SubagentPrefs>,
    manifest: &Manifest,
) -> EditOutcome {
    let mut out = EditOutcome::default();
    if let Some(tools) = palette_tools {
        let entries = palette_permission_entries(tools);
        if !entries.is_empty() {
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
        }
    }
    let Some(subagent) = subagent else {
        return out;
    };
    let desired = if subagent.model.is_empty() {
        Desired::Unset
    } else {
        Desired::Set(Json::str(subagent.model.clone()))
    };
    let path = ["env", "CLAUDE_CODE_SUBAGENT_MODEL"];
    let mut work = doc.clone();
    match apply_desired(
        &mut work,
        &path,
        &desired,
        manifest.config_record(file, &path),
    ) {
        Ok((change, undone)) => {
            *doc = work;
            out.changes.extend(change);
            if undone.is_some() {
                out.undone
                    .push(path.iter().map(|s| s.to_string()).collect());
            }
        }
        Err(e) => out.refusals.push(e.with_fix(format!(
            "make `env` in {} a JSON object and re-run",
            file.display()
        ))),
    }
    out
}

/// Reads `settings.json` text into a document; a missing file is an empty object.
pub fn parse_settings(text: Option<&str>, file: &Path) -> Result<JsonDoc> {
    JsonDoc::parse(text.unwrap_or("{}")).map_err(|e| {
        Error::config(format!("{} cannot be edited: {e}", file.display()))
            .with_state("the file was not changed")
            .with_fix(format!(
                "repair the JSON in {} and re-run `slate-setup configure`",
                file.display()
            ))
    })
}

/// True when `settings.json` contains the palette permission entries of `tools`.
pub fn has_permissions(doc: &JsonDoc, tools: &[String]) -> bool {
    let Ok(Some(Json::Array(list))) = doc.get(&["permissions", "allow"]) else {
        return false;
    };
    palette_permission_entries(tools)
        .iter()
        .all(|e| list.iter().any(|v| v.as_str() == Some(e.as_str())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::test_ctx;

    #[test]
    fn add_and_remove_command_lines() {
        assert_eq!(
            remove_args("aside"),
            ["mcp", "remove", "aside", "-s", "user"]
        );
        let a = add_args(
            "dispatch",
            Path::new("/b/dispatch"),
            &[("A".into(), "1".into()), ("B".into(), "two words".into())],
        );
        assert_eq!(
            a,
            [
                "mcp",
                "add",
                "dispatch",
                "-s",
                "user",
                "--transport",
                "stdio",
                "-e",
                "A=1",
                "-e",
                "B=two words",
                "--",
                "/b/dispatch"
            ]
        );
    }

    #[test]
    fn scope_rules() {
        let mut env = Env::for_home("/h");
        let default = Path::new("/h").join(".claude");
        assert_eq!(cli_scope(&env, &default), CliScope::Default);
        assert_eq!(
            cli_scope(&env, Path::new("/elsewhere")),
            CliScope::Unsupported
        );
        env.set_var("CLAUDE_CONFIG_DIR", "/elsewhere");
        assert_eq!(cli_scope(&env, Path::new("/elsewhere")), CliScope::Inherit);
        // The user's own variable points elsewhere but the target is the default home.
        assert_eq!(cli_scope(&env, &default), CliScope::Default);
        assert_eq!(cli_scope(&env, Path::new("/third")), CliScope::Unsupported);
    }

    #[test]
    fn unsupported_homes_get_printed_commands_and_nothing_is_run() {
        let mut env = Env::for_home("/h");
        env.set_var("CLAUDE_BIN", "/definitely/not/there");
        let ctx = test_ctx(Harness::Claude, "/elsewhere", "/h/.claude");
        let mut ui = Ui::captured(crate::ui::new_sink());
        let out = register(&env, &mut ui, &ctx).unwrap();
        assert!(out.registered.is_empty());
        assert_eq!(out.manual.len(), 3);
        // The prefix takes the form of the platform's shell.
        env.set_var("SLATE_PLATFORM", "x86_64-unknown-linux-gnu");
        let posix = register(&env, &mut ui, &ctx).unwrap();
        assert!(posix.manual[0].starts_with("CLAUDE_CONFIG_DIR=/elsewhere "));
        env.set_var("SLATE_PLATFORM", "x86_64-pc-windows-msvc");
        let ps = register(&env, &mut ui, &ctx).unwrap();
        assert!(ps.manual[0].starts_with("$env:CLAUDE_CONFIG_DIR='/elsewhere'; "));
        assert!(
            out.manual[0]
                .contains("mcp add aside -s user --transport stdio -e ASIDE_HARNESS=claude -- ")
        );
        assert!(out.notes[0].contains("does not document"));
    }

    #[test]
    fn a_missing_cli_prints_commands() {
        let mut env = Env::for_home("/h");
        env.set_var("CLAUDE_BIN", "/definitely/not/there");
        let ctx = test_ctx(Harness::Claude, "/h/.claude", "/h/.claude");
        let mut ui = Ui::captured(crate::ui::new_sink());
        let out = register(&env, &mut ui, &ctx).unwrap();
        assert!(out.registered.is_empty());
        assert!(out.notes[0].contains("was not found"));
        assert!(!out.manual[0].starts_with("CLAUDE_CONFIG_DIR"));
    }

    const SETTINGS: &str = "{\n    \"model\": \"opus\",\n    \"permissions\": {\n        \"allow\": [\"Bash(ls)\"],\n        \"deny\": []\n    },\n    \"env\": {\"FOO\": \"1\"},\n    \"hooks\": {}\n}\n";

    #[test]
    fn settings_edits_add_permissions_and_model_and_keep_everything_else() {
        let file = Path::new("settings.json");
        let mut doc = parse_settings(Some(SETTINGS), file).unwrap();
        let tools = vec!["palette_status".to_string(), "palette_read".to_string()];
        let out = settings_edits(
            &mut doc,
            file,
            Some(&tools),
            Some(&SubagentPrefs {
                model: "sonnet".into(),
                effort: "high".into(),
            }),
            &Manifest::new("k"),
        );
        assert!(out.refusals.is_empty());
        assert_eq!(out.changes.len(), 2);
        let text = doc.render_original();
        assert!(text.contains("\"mcp__palette__palette_status\""));
        assert!(text.contains("\"CLAUDE_CODE_SUBAGENT_MODEL\": \"sonnet\""));
        assert!(text.contains("\"FOO\": \"1\""));
        assert!(text.contains("\"hooks\": {}"));
        // Four-space indentation of the original file is kept, and key order too.
        assert!(text.starts_with("{\n    \"model\": \"opus\",\n    \"permissions\""));
        let model_pos = text.find("\"model\"").unwrap();
        let hooks_pos = text.find("\"hooks\"").unwrap();
        assert!(model_pos < hooks_pos);
        assert!(has_permissions(&doc, &tools));
    }

    #[test]
    fn blank_model_removes_only_what_the_installer_wrote() {
        let file = Path::new("settings.json");
        let mut doc = parse_settings(Some("{}"), file).unwrap();
        let mut manifest = Manifest::new("k");
        let on = SubagentPrefs {
            model: "haiku".into(),
            effort: String::new(),
        };
        let out = settings_edits(&mut doc, file, None, Some(&on), &manifest);
        for c in &out.changes {
            manifest.record_config(c.record(file));
        }
        assert!(doc.render_original().contains("haiku"));
        let off = SubagentPrefs::default();
        let out = settings_edits(&mut doc, file, None, Some(&off), &manifest);
        assert_eq!(out.undone.len(), 1);
        assert_eq!(doc.render_original(), "{}\n");
    }

    #[test]
    fn a_settings_file_that_is_not_json_is_refused_with_a_fix() {
        let err = parse_settings(Some("{ nope"), Path::new("/h/settings.json")).unwrap_err();
        assert!(err.state.as_deref().unwrap().contains("not changed"));
        assert!(
            err.fix
                .as_deref()
                .unwrap()
                .contains("slate-setup configure")
        );
        assert!(
            parse_settings(None, Path::new("s"))
                .unwrap()
                .root
                .is_object()
        );
    }

    #[test]
    fn a_non_array_allow_list_is_refused_but_the_model_still_applies() {
        let file = Path::new("settings.json");
        let mut doc = parse_settings(Some("{\"permissions\":{\"allow\":\"x\"}}"), file).unwrap();
        let tools = vec!["t".to_string()];
        let out = settings_edits(
            &mut doc,
            file,
            Some(&tools),
            Some(&SubagentPrefs {
                model: "opus".into(),
                effort: String::new(),
            }),
            &Manifest::new("k"),
        );
        assert_eq!(out.refusals.len(), 1);
        assert_eq!(out.changes.len(), 1);
        assert!(doc.render_original().contains("\"allow\": \"x\""));
    }
}
