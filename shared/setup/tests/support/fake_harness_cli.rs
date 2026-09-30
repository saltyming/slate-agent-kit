//! Stand-in for the `claude` and `codex` CLIs and for the palette server's probe.
//!
//! Owns a recording, scriptable imitation of the commands the installer runs:
//! `mcp add` and `mcp remove` for both harness CLIs (Codex also writes its
//! `config.toml` like the real one does) and `palette --read-only-tools`. It
//! appends every invocation to the file named by `FAKE_CLI_LOG` and fails when
//! `FAKE_CLI_FAIL` is a substring of the arguments. It is test support only.

use slate_setup::config::{ConfigDoc, TomlDoc};
use slate_setup::json::Json;
use std::io::Write;

fn record(args: &[String]) {
    let Ok(path) = std::env::var("FAKE_CLI_LOG") else {
        return;
    };
    let mut env_members = Vec::new();
    for key in ["CODEX_HOME", "CLAUDE_CONFIG_DIR"] {
        if let Ok(v) = std::env::var(key) {
            env_members.push((key.to_string(), Json::String(v)));
        }
    }
    let line = Json::Object(vec![
        (
            "args".to_string(),
            Json::Array(args.iter().map(|a| Json::String(a.clone())).collect()),
        ),
        ("env".to_string(), Json::Object(env_members)),
    ]);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{}", line.to_compact());
    }
}

fn codex_config_path() -> Option<std::path::PathBuf> {
    std::env::var("CODEX_HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join("config.toml"))
}

fn codex_add(args: &[String]) {
    let name = &args[2];
    let mut env_pairs = Vec::new();
    let mut i = 3;
    while i < args.len() && args[i] != "--" {
        if args[i] == "--env" {
            if let Some((k, v)) = args.get(i + 1).and_then(|p| p.split_once('=')) {
                env_pairs.push((k.to_string(), v.to_string()));
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    let command = args.get(i + 1).cloned().unwrap_or_default();
    let Some(path) = codex_config_path() else {
        return;
    };
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc = TomlDoc::parse(&text).expect("the fake codex reads a valid config.toml");
    doc.set(&["mcp_servers", name, "command"], &Json::String(command))
        .unwrap();
    doc.set(&["mcp_servers", name, "args"], &Json::Array(vec![]))
        .unwrap();
    for (k, v) in env_pairs {
        doc.set(&["mcp_servers", name, "env", &k], &Json::String(v))
            .unwrap();
    }
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, doc.render()).unwrap();
}

fn codex_remove(args: &[String]) {
    let Some(path) = codex_config_path() else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let mut doc = TomlDoc::parse(&text).expect("the fake codex reads a valid config.toml");
    doc.remove(&["mcp_servers", &args[2]], 0);
    std::fs::write(&path, doc.render()).unwrap();
}

/// `cargo build --release -p <name>... [--bin <bin>]`: writes each package's binary under
/// `target/release`; `--bin` names the one binary of the selected package instead.
fn cargo_build(args: &[String]) {
    let exe = std::env::current_exe().expect("the fake knows its own path");
    let target = std::env::var("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("target"));
    let release = target.join("release");
    std::fs::create_dir_all(&release).unwrap();
    let by_bin = args.iter().any(|a| a == "--bin");
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-p" && by_bin {
            i += 1;
        } else if args[i] == "-p" || args[i] == "--bin" {
            let name = &args[i + 1];
            let file = if cfg!(windows) {
                format!("{name}.exe")
            } else {
                name.clone()
            };
            std::fs::copy(&exe, release.join(file)).unwrap();
            i += 1;
        }
        i += 1;
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("build") {
        record(&args);
        cargo_build(&args);
        return;
    }
    if args.first().map(String::as_str) == Some("--read-only-tools") {
        println!("palette_status\npalette_read");
        return;
    }
    record(&args);
    if let Ok(needle) = std::env::var("FAKE_CLI_FAIL")
        && args.join(" ").contains(&needle)
    {
        eprintln!("fake cli: refusing `{}`", args.join(" "));
        std::process::exit(1);
    }
    let is_claude = args.iter().any(|a| a == "-s");
    match (
        args.first().map(String::as_str),
        args.get(1).map(String::as_str),
    ) {
        (Some("mcp"), Some("add")) if !is_claude => codex_add(&args),
        (Some("mcp"), Some("remove")) if !is_claude => codex_remove(&args),
        (Some("mcp"), Some("add" | "remove")) => {}
        _ => {
            eprintln!("fake cli: unsupported arguments {args:?}");
            std::process::exit(2);
        }
    }
}
