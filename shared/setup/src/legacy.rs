//! Cleanup of what earlier releases installed and this one no longer ships.
//!
//! Owns the `workslate` cleanup for Claude Code (settings hooks, binary, MCP
//! registration and per-project databases) and the removal of binaries the
//! earlier Windows Codex installer left under the Codex home. It does not run
//! unless the kit descriptor lists the cleanup or an earlier location holds files.
//!
//! Main entry points: [`workslate_findings`], [`workslate_cleanup`],
//! [`earlier_codex_binaries`] and [`remove_earlier_codex_binaries`].

use crate::binaries::exe_name;
use crate::env::{Env, Harness};
use crate::error::{Error, IoContext, Result};
use crate::harness::claude::{CliScope, cli_scope, remove_args};
use crate::json::Json;
use crate::manifest::Manifest;
use crate::ui::Ui;
use crate::util::{backup_file, read_text_opt, write_atomic};
use std::fs;
use std::path::{Path, PathBuf};

fn is_workslate_handler(h: &Json) -> bool {
    let command = h.get("command").and_then(Json::as_str).unwrap_or("");
    let prompt = h.get("prompt").and_then(Json::as_str).unwrap_or("");
    (command.contains("workslate") && command.contains("--hook="))
        || command.contains("[workslate-task-verify]")
        || prompt.contains("[workslate-task-verify]")
}

/// Removes workslate hook handlers from a parsed `settings.json`. Returns how many were removed.
fn strip_hooks(root: &mut Json) -> usize {
    let mut removed = 0;
    let Some(hooks) = root.get_mut("hooks") else {
        return 0;
    };
    let Json::Object(events) = hooks else {
        return 0;
    };
    for (_, groups) in events.iter_mut() {
        let Some(list) = groups.as_array_mut() else {
            continue;
        };
        for group in list.iter_mut() {
            if let Some(handlers) = group.get_mut("hooks").and_then(Json::as_array_mut) {
                let before = handlers.len();
                handlers.retain(|h| !is_workslate_handler(h));
                removed += before - handlers.len();
            }
        }
        list.retain(|g| !matches!(g.get("hooks"), Some(Json::Array(h)) if h.is_empty()));
    }
    events.retain(|(_, v)| !matches!(v, Json::Array(a) if a.is_empty()));
    removed
}

fn workslate_db_dirs(home: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(home.join("projects")) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path().join("workslate"))
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs
}

/// Describes what the workslate cleanup would remove; empty when there is nothing.
pub fn workslate_findings(env: &Env, home: &Path, bin_dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let settings = home.join("settings.json");
    if let Ok(Some(text)) = read_text_opt(&settings)
        && let Ok(mut json) = Json::parse(&text)
    {
        let n = strip_hooks(&mut json);
        if n > 0 {
            out.push(format!(
                "{n} workslate hook entries in {}",
                settings.display()
            ));
        }
    }
    let binary = bin_dir.join(exe_name(&env.platform(), "workslate"));
    if binary.exists() {
        out.push(format!("the binary {}", binary.display()));
    }
    for d in workslate_db_dirs(home) {
        out.push(format!("the database folder {}", d.display()));
    }
    out
}

/// Runs the workslate cleanup. Returns notes for the report.
///
/// Every other hook, server and file is left alone. `claude mcp remove` runs
/// only when the CLI acts on `home`.
pub fn workslate_cleanup(
    env: &Env,
    ui: &mut Ui,
    home: &Path,
    bin_dir: &Path,
    manifest: &mut Manifest,
) -> Result<Vec<String>> {
    let mut notes = Vec::new();
    let settings = home.join("settings.json");
    if let Some(text) = read_text_opt(&settings)? {
        match Json::parse(&text) {
            Ok(mut json) => {
                let indent = crate::json::detect_indent(&text);
                if strip_hooks(&mut json) > 0 {
                    let backup = backup_file(&settings)?;
                    manifest.record_backup(&backup, &settings);
                    write_atomic(&settings, json.to_pretty(&indent).as_bytes())?;
                    ui.ok(&format!("removed workslate hooks from {} (backup {})", settings.display(), backup.display()));
                }
            }
            Err(_) if text.contains("workslate") => notes.push(format!(
                "{} is not valid JSON, so workslate hooks in it could not be checked; delete every hook whose command contains `workslate` and `--hook=` by hand, or each tool call reports a hook error",
                settings.display()
            )),
            Err(_) => {}
        }
    }
    let binary = bin_dir.join(exe_name(&env.platform(), "workslate"));
    if binary.exists() {
        fs::remove_file(&binary).ctx(|| format!("removing {}", binary.display()))?;
        ui.ok(&format!("removed {}", binary.display()));
    }
    if let Some(program) = env.cli_program(Harness::Claude) {
        let scope = cli_scope(env, home);
        if scope != CliScope::Unsupported && env.which(&program).is_some() {
            let remove_env: &[&str] = if scope == CliScope::Default {
                &["CLAUDE_CONFIG_DIR"]
            } else {
                &[]
            };
            let out =
                crate::harness::run_cli(env, &program, &remove_args("workslate"), &[], remove_env)?;
            if out.status.success() {
                ui.ok("unregistered the workslate MCP server");
            }
        }
    }
    for dir in workslate_db_dirs(home) {
        for f in ["workslate.db", "workslate.db-wal", "workslate.db-shm"] {
            let p = dir.join(f);
            if p.exists() {
                fs::remove_file(&p).ctx(|| format!("removing {}", p.display()))?;
            }
        }
        // The folder goes only when nothing else is in it.
        let _ = fs::remove_dir(&dir);
    }
    Ok(notes)
}

/// Binaries the earlier Windows Codex installer put under `<home>/slate-agent-kit/bin`.
///
/// Nothing is listed when that folder is also the binary folder in use.
pub fn earlier_codex_binaries(home: &Path, bin_dir: &Path) -> Vec<PathBuf> {
    let dir = home.join("slate-agent-kit").join("bin");
    if crate::util::same_path(&dir, bin_dir) {
        return Vec::new();
    }
    ["aside", "dispatch"]
        .iter()
        .flat_map(|n| [dir.join(exe_name("x86_64-pc-windows-msvc", n)), dir.join(n)])
        .filter(|p| p.is_file())
        .collect()
}

/// Removes the earlier Codex binaries and the emptied `bin` folder.
pub fn remove_earlier_codex_binaries(ui: &mut Ui, home: &Path, bin_dir: &Path) -> Result<()> {
    for p in earlier_codex_binaries(home, bin_dir) {
        fs::remove_file(&p).map_err(|e| Error::io(format!("removing {}", p.display()), e))?;
        ui.ok(&format!(
            "removed the earlier installer's binary {}",
            p.display()
        ));
    }
    let _ = fs::remove_dir(home.join("slate-agent-kit").join("bin"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{Ui, new_sink};

    const SETTINGS: &str = r#"{
  "theme": "dark",
  "hooks": {
    "PreToolUse": [
      {"matcher": "", "hooks": [
        {"type": "command", "command": "/home/u/.local/bin/workslate --hook=pre-tool-use"},
        {"type": "command", "command": "/usr/bin/my-own-hook"}
      ]}
    ],
    "Stop": [
      {"hooks": [
        {"type": "agent", "prompt": "[workslate-task-verify] check the task list"},
        {"type": "command", "command": "echo [workslate-task-verify]"}
      ]}
    ],
    "SessionStart": [
      {"hooks": [{"type": "command", "command": "/x/workslate --hook=session-start"}]}
    ]
  }
}
"#;

    #[test]
    fn only_workslate_handlers_are_removed_and_empty_containers_go() {
        let mut json = Json::parse(SETTINGS).unwrap();
        assert_eq!(strip_hooks(&mut json), 4);
        let hooks = json.get("hooks").unwrap();
        assert!(hooks.get("Stop").is_none());
        assert!(hooks.get("SessionStart").is_none());
        let pre = hooks.get("PreToolUse").unwrap().as_array().unwrap();
        assert_eq!(pre.len(), 1);
        let handlers = pre[0].get("hooks").unwrap().as_array().unwrap();
        assert_eq!(handlers.len(), 1);
        assert_eq!(
            handlers[0].get("command").and_then(Json::as_str),
            Some("/usr/bin/my-own-hook")
        );
        assert_eq!(json.get("theme").and_then(Json::as_str), Some("dark"));
    }

    #[test]
    fn a_workslate_word_without_hook_flag_is_not_a_hook() {
        let mut json = Json::parse(r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"cat workslate-notes.txt"}]}]}}"#).unwrap();
        assert_eq!(strip_hooks(&mut json), 0);
    }

    #[test]
    fn cleanup_backs_up_settings_and_removes_binary_and_databases() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join(".claude");
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(home.join("projects").join("-w-proj").join("workslate")).unwrap();
        fs::create_dir_all(home.join("projects").join("-w-other").join("workslate")).unwrap();
        fs::write(home.join("settings.json"), SETTINGS).unwrap();
        fs::write(bin.join("workslate"), "x").unwrap();
        for f in ["workslate.db", "workslate.db-wal", "workslate.db-shm"] {
            fs::write(home.join("projects/-w-proj/workslate").join(f), "db").unwrap();
        }
        fs::write(home.join("projects/-w-other/workslate/keep.txt"), "keep").unwrap();
        let mut env = Env::for_home(dir.path());
        env.set_var("CLAUDE_BIN", "/definitely/not/there");
        let findings = workslate_findings(&env, &home, &bin);
        assert_eq!(findings.len(), 4, "{findings:?}");
        let mut manifest = Manifest::new("k");
        let mut ui = Ui::captured(new_sink());
        workslate_cleanup(&env, &mut ui, &home, &bin, &mut manifest).unwrap();
        assert!(!bin.join("workslate").exists());
        assert!(
            !home.join("projects/-w-proj/workslate").exists(),
            "emptied folder is removed"
        );
        assert!(
            home.join("projects/-w-other/workslate/keep.txt").exists(),
            "other files stay"
        );
        let settings = fs::read_to_string(home.join("settings.json")).unwrap();
        assert!(!settings.contains("workslate"));
        assert!(settings.contains("my-own-hook"));
        assert_eq!(manifest.backups.len(), 1);
        assert!(
            fs::read_to_string(&manifest.backups[0].path)
                .unwrap()
                .contains("workslate")
        );
        // Only the unrelated file in the other project's folder is left over.
        assert_eq!(workslate_findings(&env, &home, &bin).len(), 1);
    }

    #[test]
    fn an_unparsable_settings_file_is_reported_not_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join(".claude");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("settings.json"), "{ workslate --hook= broken").unwrap();
        let mut env = Env::for_home(dir.path());
        env.set_var("CLAUDE_BIN", "/definitely/not/there");
        let mut manifest = Manifest::new("k");
        let mut ui = Ui::captured(new_sink());
        let notes = workslate_cleanup(&env, &mut ui, &home, &dir.path().join("bin"), &mut manifest)
            .unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(
            fs::read_to_string(home.join("settings.json")).unwrap(),
            "{ workslate --hook= broken"
        );
    }

    #[test]
    fn earlier_codex_binaries_are_found_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join(".codex");
        let old = home.join("slate-agent-kit").join("bin");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("aside.exe"), "x").unwrap();
        fs::write(old.join("dispatch.exe"), "x").unwrap();
        let bin = dir.path().join("bin");
        assert_eq!(earlier_codex_binaries(&home, &bin).len(), 2);
        assert!(
            earlier_codex_binaries(&home, &old).is_empty(),
            "the folder in use is never cleaned"
        );
        let mut ui = Ui::captured(new_sink());
        remove_earlier_codex_binaries(&mut ui, &home, &bin).unwrap();
        assert!(!old.exists());
        assert!(earlier_codex_binaries(&home, &bin).is_empty());
    }
}
