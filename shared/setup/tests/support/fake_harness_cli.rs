//! Stand-in for the `claude`, `codex` and `agy` CLIs and for the palette server's probe.
//!
//! Owns a recording, scriptable imitation of the commands the installer runs:
//! `mcp add` and `mcp remove` for the harness CLIs (Codex also writes its
//! `config.toml`, and agy its `<home>/.gemini/config/mcp_config.json`, like the
//! real ones do) and `palette --read-only-tools`. A copy named `agy` acts as
//! agy, which takes its flags before the server name and follows `HOME`
//! (`USERPROFILE` on Windows). It appends every invocation to the file named by
//! `FAKE_CLI_LOG` and fails when `FAKE_CLI_FAIL` is a substring of the
//! arguments. As the palette server it prints `palette_status` and
//! `palette_read` for `--read-only-tools`, the comma-separated names in
//! `FAKE_PALETTE_TOOLS` instead when that is set, and fails when
//! `FAKE_PALETTE_FAIL` is set. It is test support only.

use slate_setup::config::{ConfigDoc, TomlDoc};
use slate_setup::json::Json;
use std::io::Write;

/// True when this copy runs as `agy` (its file name, without `.exe`).
fn is_agy() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s == "agy"))
        .unwrap_or(false)
}

/// The variable agy reads its home from on this platform.
fn agy_home_var() -> &'static str {
    if cfg!(windows) { "USERPROFILE" } else { "HOME" }
}

fn record(args: &[String]) {
    let Ok(path) = std::env::var("FAKE_CLI_LOG") else {
        return;
    };
    let mut env_members = Vec::new();
    let mut keys = vec!["CODEX_HOME", "CLAUDE_CONFIG_DIR"];
    if is_agy() {
        keys.push(agy_home_var());
    }
    for key in keys {
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

/// `<home>/.gemini/config/mcp_config.json`, from the home agy follows.
fn agy_config_path() -> Option<std::path::PathBuf> {
    std::env::var(agy_home_var()).ok().map(|h| {
        std::path::Path::new(&h)
            .join(".gemini")
            .join("config")
            .join("mcp_config.json")
    })
}

fn agy_read() -> (std::path::PathBuf, Vec<(String, Json)>) {
    let path = agy_config_path().expect("agy runs with a home");
    let servers = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| Json::parse(&t).ok())
        .and_then(|j| match j.get("mcpServers") {
            Some(Json::Object(m)) => Some(m.clone()),
            _ => None,
        })
        .unwrap_or_default();
    (path, servers)
}

fn agy_write(path: &std::path::Path, servers: Vec<(String, Json)>) {
    let j = Json::Object(vec![("mcpServers".into(), Json::Object(servers))]);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, format!("{}\n", j.to_compact())).unwrap();
}

/// `agy mcp add [-e|--env K=V]... <name> <command> [args...]`.
fn agy_add(args: &[String]) {
    let mut env_pairs = Vec::new();
    let mut i = 2;
    while i < args.len() && (args[i] == "--env" || args[i] == "-e") {
        if let Some((k, v)) = args.get(i + 1).and_then(|p| p.split_once('=')) {
            env_pairs.push((k.to_string(), Json::String(v.to_string())));
        }
        i += 2;
    }
    let (Some(name), Some(command)) = (args.get(i), args.get(i + 1)) else {
        eprintln!("fake agy: `mcp add` needs a name and a command");
        std::process::exit(2);
    };
    let rest: Vec<Json> = args[i + 2..]
        .iter()
        .map(|a| Json::String(a.clone()))
        .collect();
    let (path, mut servers) = agy_read();
    servers.retain(|(k, _)| k != name);
    servers.push((
        name.clone(),
        Json::Object(vec![
            ("args".into(), Json::Array(rest)),
            ("command".into(), Json::String(command.clone())),
            ("disabled".into(), Json::Bool(false)),
            ("env".into(), Json::Object(env_pairs)),
        ]),
    ));
    agy_write(&path, servers);
}

/// `agy mcp remove <name>`: fails like a CLI does for a server it does not have.
fn agy_remove(args: &[String]) {
    let name = &args[2];
    let (path, mut servers) = agy_read();
    if !servers.iter().any(|(k, _)| k == name) {
        eprintln!("fake agy: no MCP server named {name}");
        std::process::exit(1);
    }
    servers.retain(|(k, _)| k != name);
    agy_write(&path, servers);
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
        if std::env::var_os("FAKE_PALETTE_FAIL").is_some() {
            eprintln!("fake palette: refusing --read-only-tools");
            std::process::exit(1);
        }
        match std::env::var("FAKE_PALETTE_TOOLS") {
            Ok(list) => list.split(',').for_each(|t| println!("{t}")),
            Err(_) => println!("palette_status\npalette_read"),
        }
        return;
    }
    record(&args);
    if let Ok(needle) = std::env::var("FAKE_CLI_FAIL")
        && args.join(" ").contains(&needle)
    {
        eprintln!("fake cli: refusing `{}`", args.join(" "));
        std::process::exit(1);
    }
    if is_agy() {
        match (
            args.first().map(String::as_str),
            args.get(1).map(String::as_str),
        ) {
            (Some("mcp"), Some("add")) => agy_add(&args),
            (Some("mcp"), Some("remove")) if args.len() == 3 => agy_remove(&args),
            _ => {
                eprintln!("fake agy: unsupported arguments {args:?}");
                std::process::exit(2);
            }
        }
        return;
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
