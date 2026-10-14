//! The command line and the MCP server, exercised through the built binary.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use common::*;
use palette_server::server::{read_only_tool_names, tool_annotations};
use serde_json::{Value, json};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_palette"))
}

/// A command with no project root: the root variables are removed and the working
/// directory is the file system root, which is never a plausible project.
fn rootless(cmd: &mut Command) -> &mut Command {
    for key in [
        "SLATE_PROJECT_DIR",
        "AGENT_KIT_PROJECT_DIR",
        "CLAUDE_PROJECT_DIR",
        "PALETTE_EXTRA_ROOTS",
    ] {
        cmd.env_remove(key);
    }
    cmd.current_dir("/")
}

#[test]
fn version_and_usage() {
    let out = bin().arg("--version").output().expect("run");
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("palette {}", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(
        bin().arg("--bogus").output().expect("run").status.code(),
        Some(2)
    );
    assert_eq!(
        bin().arg("check").output().expect("run").status.code(),
        Some(2)
    );
}

#[test]
fn read_only_tools_come_from_the_annotations() {
    let out = bin().arg("--read-only-tools").output().expect("run");
    assert!(out.status.success());
    let mut printed: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(String::from)
        .collect();
    printed.sort();
    assert_eq!(
        printed,
        vec![
            "palette_layout",
            "palette_lint",
            "palette_status",
            "palette_template"
        ]
    );
    let mut api = read_only_tool_names();
    api.sort();
    assert_eq!(printed, api);
    let ann = tool_annotations();
    assert_eq!(ann.len(), 20);
    for added in ["palette_generate", "palette_record_refresh"] {
        assert!(ann.iter().any(|(n, _, _)| n == added), "{added}");
    }
    for (name, ro, destructive) in &ann {
        if printed.contains(name) {
            assert_eq!(*ro, Some(true), "{name}");
        } else {
            assert_eq!(*ro, Some(false), "{name}");
            let want = name.as_str() == "palette_layout_set";
            assert_eq!(
                *destructive,
                Some(want),
                "{name}: only layout_set carry the destructive annotation"
            );
        }
    }
}

#[test]
fn check_exit_codes() {
    let p = Proj::valid();
    let ok = bin().arg("check").arg(&p.root).output().expect("run");
    assert_eq!(
        ok.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&ok.stdout)
    );
    assert!(String::from_utf8_lossy(&ok.stdout).contains("0 error(s), 0 warning(s)"));
    p.replace(
        "docs/rfc/rfc-0003-gamma.rst",
        ":Status: Draft",
        ":Status: Done",
    );
    let bad = bin().arg("check").arg(&p.root).output().expect("run");
    assert_eq!(bad.status.code(), Some(1));
    let text = String::from_utf8_lossy(&bad.stdout);
    assert!(
        text.contains("docs/rfc/rfc-0003-gamma.rst:4: error P002"),
        "{text}"
    );
    // A warning alone exits 0.
    let w = Proj::valid();
    w.replace(
        "_palette/backlog.rst",
        "Build the first thing.",
        "one\ntwo\nthree\nfour\nfive",
    );
    assert_eq!(
        bin()
            .arg("check")
            .arg(&w.root)
            .output()
            .expect("run")
            .status
            .code(),
        Some(0)
    );
    // No such project: exit 2.
    assert_eq!(
        bin()
            .arg("check")
            .arg(p.path("nope"))
            .output()
            .expect("run")
            .status
            .code(),
        Some(2)
    );
    // A checkout without `_palette/` (the shared families only, as CI sees them) is
    // checked from the documents themselves: exit 0 when they are clean.
    let shared = common::Proj::valid();
    std::fs::remove_dir_all(shared.path("_palette")).expect("rm");
    let out = bin().arg("check").arg(&shared.root).output().expect("run");
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    // `generate` works there too and needs no lock folder.
    let out = bin()
        .arg("generate")
        .arg(&shared.root)
        .output()
        .expect("run");
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!shared.path("_palette").exists());
}

const LONG_LINE: &str = "This paragraph is far longer than the width of seventy-nine columns, and it is written on one line the way a caller supplies it, far longer than the width of seventy-nine columns.";

#[test]
fn reflow_rewraps_prints_the_files_it_wrote_and_then_has_nothing_to_do() {
    let p = Proj::valid();
    p.replace(
        "docs/spec/thing.rst",
        "The scope and authority of the thing.",
        LONG_LINE,
    );
    p.replace(
        "docs/rfc/rfc-0002-beta.rst",
        "Beta rules apply to every operation of the thing.",
        LONG_LINE,
    );
    let out = bin().arg("reflow").arg(&p.root).output().expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let printed: Vec<&str> = stdout.lines().collect();
    assert!(
        printed.contains(&"modified docs/spec/thing.rst"),
        "{stdout}"
    );
    assert!(
        printed.contains(&"modified docs/rfc/rfc-0002-beta.rst"),
        "{stdout}"
    );
    // The staging mirror of the spec is regenerated from the rewrapped text.
    assert!(
        printed.contains(&"modified docs/staging/spec/thing.rst"),
        "{stdout}"
    );
    assert!(
        p.read("docs/spec/thing.rst")
            .lines()
            .all(|l| l.chars().count() <= 79)
    );

    let check = bin().arg("check").arg(&p.root).output().expect("run");
    assert_eq!(
        check.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&check.stdout)
    );
    let generate = bin().arg("generate").arg(&p.root).output().expect("run");
    assert!(
        String::from_utf8_lossy(&generate.stdout).contains("nothing to generate"),
        "{}",
        String::from_utf8_lossy(&generate.stdout)
    );

    let again = bin().arg("reflow").arg(&p.root).output().expect("run");
    assert_eq!(again.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&again.stdout).trim(),
        "nothing to reflow: every document is already wrapped"
    );
}

#[test]
fn reflow_takes_paths_and_refuses_a_bad_one_without_writing() {
    let p = Proj::valid();
    p.replace(
        "docs/spec/thing.rst",
        "The thing has one operation.",
        LONG_LINE,
    );
    p.replace(
        "docs/rfc/rfc-0002-beta.rst",
        "Beta rules apply to every operation of the thing.",
        LONG_LINE,
    );
    let before = p.snapshot();
    for bad in ["docs/missing.rst", "docs/rfc/index.rst", "../x.rst"] {
        let out = bin()
            .arg("reflow")
            .arg(&p.root)
            .arg("docs/spec")
            .arg(bad)
            .output()
            .expect("run");
        assert_eq!(out.status.code(), Some(1), "{bad}");
        assert!(
            String::from_utf8_lossy(&out.stderr).starts_with("palette reflow: "),
            "{bad}"
        );
        assert_eq!(p.snapshot(), before, "{bad}: nothing written");
    }
    let out = bin()
        .arg("reflow")
        .arg(&p.root)
        .arg("docs/spec")
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("modified docs/spec/thing.rst"), "{stdout}");
    assert!(!stdout.contains("rfc-0002-beta"), "{stdout}");
    assert!(p.read("docs/rfc/rfc-0002-beta.rst").contains(LONG_LINE));
}

#[test]
fn reflow_usage_and_projects_that_do_not_exist() {
    assert_eq!(
        bin().arg("reflow").output().expect("run").status.code(),
        Some(2)
    );
    let p = Proj::valid();
    let out = bin()
        .arg("reflow")
        .arg(p.path("nope"))
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(1));
    // A checkout without `_palette/` is rewrapped from the documents it identifies, as
    // `generate` and `check` read it, and no lock folder is made.
    let shared = Proj::valid();
    std::fs::remove_dir_all(shared.path("_palette")).expect("rm");
    shared.replace(
        "docs/spec/thing.rst",
        "The thing has one operation.",
        LONG_LINE,
    );
    let out = bin().arg("reflow").arg(&shared.root).output().expect("run");
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("modified docs/spec/thing.rst"));
    assert!(!shared.path("_palette").exists());
}

#[test]
fn check_resolves_the_project_without_the_root_rules() {
    // The working directory is elsewhere and no root variable is set: check still works.
    let p = Proj::valid();
    let out = rootless(&mut bin())
        .arg("check")
        .arg(&p.root)
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(0));
}

struct Client {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    next: u64,
}

impl Client {
    fn start(configure: impl FnOnce(&mut Command)) -> Client {
        let mut cmd = bin();
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        configure(&mut cmd);
        let mut child = cmd.spawn().expect("spawn");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut c = Client {
            child,
            stdin,
            lines: rx,
            next: 1,
        };
        let init = c.request("initialize", json!({"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}));
        assert!(
            init["result"]["instructions"]
                .as_str()
                .expect("instructions")
                .contains("palette_lint")
        );
        c.notify("notifications/initialized");
        c
    }

    fn send(&mut self, v: &Value) {
        writeln!(self.stdin, "{v}").expect("write");
        self.stdin.flush().expect("flush");
    }

    fn notify(&mut self, method: &str) {
        self.send(&json!({"jsonrpc": "2.0", "method": method}));
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let line = self
                .lines
                .recv_timeout(Duration::from_secs(20))
                .expect("response in time");
            let v: Value = serde_json::from_str(&line).expect("json line");
            if v["id"] == json!(id) {
                return v;
            }
        }
    }

    fn call(&mut self, tool: &str, args: Value) -> (bool, String) {
        let r = self.request("tools/call", json!({"name": tool, "arguments": args}));
        if let Some(e) = r.get("error") {
            // Invalid arguments are refused by the protocol layer before the tool runs.
            return (true, e["message"].as_str().unwrap_or("").to_string());
        }
        let is_error = r["result"]["isError"].as_bool().unwrap_or(false);
        let text = r["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string();
        (is_error, text)
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn mcp_lists_tools_and_serves_calls() {
    let p = Proj::valid();
    let mut c = Client::start(|cmd| {
        cmd.env_remove("PALETTE_EXTRA_ROOTS")
            .env("SLATE_PROJECT_DIR", &p.root);
    });
    let list = c.request("tools/list", json!({}));
    let tools = list["result"]["tools"].as_array().expect("tools");
    assert_eq!(tools.len(), 20);
    for t in tools {
        let name = t["name"].as_str().expect("name");
        assert!(
            t["description"].as_str().expect("description").len() > 100,
            "{name} needs a real description"
        );
        let props = &t["inputSchema"]["properties"];
        if name != "palette_template" {
            assert!(props.get("project").is_some(), "{name} takes project");
        }
    }
    let write_tools: Vec<&Value> = tools
        .iter()
        .filter(|t| t["annotations"]["readOnlyHint"] == json!(false))
        .collect();
    assert_eq!(write_tools.len(), 16);
    for t in write_tools.iter().filter(|t| t["name"] != "palette_init") {
        assert!(
            t["inputSchema"]["properties"].get("dry_run").is_some(),
            "{} takes dry_run",
            t["name"]
        );
    }

    let (err, text) = c.call("palette_lint", json!({"project": p.arg()}));
    assert!(!err);
    let v: Value = serde_json::from_str(&text).expect("json");
    assert_eq!(v["errors"], 0);

    // A write with dry_run, and array arguments sent as a JSON-encoded string.
    let (err, text) = c.call("palette_backlog_add", json!({
        "project": p.arg(), "title": "Via MCP", "type": "bug", "source": "user", "depends": "[\"B-2\"]", "dry_run": "true"
    }));
    assert!(!err, "{text}");
    let v: Value = serde_json::from_str(&text).expect("json");
    assert_eq!(v["dry_run"], true);
    assert_eq!(v["allocated"], json!(["B-5"]));
    assert!(v["diff"].as_str().expect("diff").contains("+B-5 Via MCP"));
    assert!(!p.read("_palette/backlog.rst").contains("Via MCP"));

    // A real write, and a structured error.
    let (err, _) = c.call(
        "palette_backlog_add",
        json!({"project": p.arg(), "title": "Via MCP", "type": "bug", "source": "user"}),
    );
    assert!(!err);
    assert!(p.read("_palette/backlog.rst").contains("B-5 Via MCP"));
    let other = tempfile::tempdir().expect("other");
    let (err, text) = c.call(
        "palette_lint",
        json!({"project": other.path().to_string_lossy()}),
    );
    assert!(err);
    let v: Value = serde_json::from_str(&text).expect("json");
    assert_eq!(v["error"]["code"], "outside_roots");
    assert!(
        v["error"]["message"]
            .as_str()
            .expect("message")
            .contains("PALETTE_EXTRA_ROOTS")
    );
    let (err, text) = c.call("palette_template", json!({"family": "adr"}));
    assert!(!err && text.contains("ADR-<NNNN>"));
    let (err, text) = c.call(
        "palette_backlog_add",
        json!({"project": p.arg(), "title": "x"}),
    );
    assert!(err, "missing required fields are an error: {text}");
}

#[test]
fn without_a_root_every_project_tool_says_how_to_set_one() {
    let p = Proj::valid();
    let mut c = Client::start(|cmd| {
        rootless(cmd);
    });
    let (err, text) = c.call("palette_status", json!({"project": p.arg()}));
    assert!(err);
    let v: Value = serde_json::from_str(&text).expect("json");
    assert_eq!(v["error"]["code"], "no_project_root");
    assert!(
        v["error"]["message"]
            .as_str()
            .expect("m")
            .contains("SLATE_PROJECT_DIR")
    );
    // The template tool needs no project.
    let (err, _) = c.call("palette_template", json!({"family": "rfc"}));
    assert!(!err);
}

#[test]
fn extra_roots_extend_the_boundary() {
    let p = Proj::valid();
    let elsewhere = tempfile::tempdir().expect("else");
    let mut c = Client::start(|cmd| {
        rootless(cmd)
            .env("SLATE_PROJECT_DIR", elsewhere.path())
            .env("PALETTE_EXTRA_ROOTS", p.root.parent().expect("parent"));
    });
    let (err, text) = c.call("palette_layout", json!({"project": p.arg()}));
    assert!(!err, "{text}");
    let v: Value = serde_json::from_str(&text).expect("json");
    assert_eq!(v["families"]["rfc"]["placement"], "docs/rfc");
}

fn generated_files() -> [&'static str; 4] {
    [
        "docs/rfc/index.rst",
        "docs/adr/index.rst",
        "docs/changeset/index.rst",
        "docs/staging/spec/thing.rst",
    ]
}

#[test]
fn generate_writes_the_missing_generated_files_and_check_then_passes() {
    let p = Proj::valid();
    for f in generated_files() {
        p.remove(f);
    }
    p.write("docs/staging/spec/orphan.rst", "Orphan\n======\n");
    assert_eq!(
        bin()
            .arg("check")
            .arg(&p.root)
            .output()
            .expect("run")
            .status
            .code(),
        Some(1)
    );
    let before = p.snapshot();
    let out = rootless(&mut bin())
        .arg("generate")
        .arg(&p.root)
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    for f in generated_files() {
        assert!(stdout.contains(&format!("created {f}")), "{stdout}");
        assert!(p.exists(f));
    }
    assert!(
        stdout.contains("deleted docs/staging/spec/orphan.rst")
            && !p.exists("docs/staging/spec/orphan.rst"),
        "{stdout}"
    );
    // Only the generated files changed.
    let after = p.snapshot();
    let changed: Vec<&String> = before
        .keys()
        .chain(after.keys())
        .filter(|k| before.get(*k) != after.get(*k))
        .collect();
    assert_eq!(changed.len(), 5, "{changed:?}");
    assert_eq!(
        bin()
            .arg("check")
            .arg(&p.root)
            .output()
            .expect("run")
            .status
            .code(),
        Some(0)
    );
    // Nothing left to do the second time.
    let again = bin().arg("generate").arg(&p.root).output().expect("run");
    assert_eq!(again.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&again.stdout).contains("nothing to generate"));
    assert_eq!(p.snapshot(), after);
}

#[test]
fn generate_is_all_or_nothing_and_reports_errors_with_exit_1() {
    let p = Proj::valid();
    for f in generated_files() {
        p.remove(f);
    }
    // A table in a changeset body ends up in the staging document: P001 blocks the write.
    p.replace(
        "docs/changeset/rfc-0002.rst",
        "The thing has two operations: ``open`` and ``close``.",
        "The thing has two operations.\n\n+---+---+\n| a | b |\n+---+---+",
    );
    let before = p.snapshot();
    let out = bin().arg("generate").arg(&p.root).output().expect("run");
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("invariant_violation"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(p.snapshot(), before, "nothing may be written");
    // No project, no layout, and wrong usage.
    assert_eq!(
        bin()
            .arg("generate")
            .arg(p.path("nope"))
            .output()
            .expect("run")
            .status
            .code(),
        Some(1)
    );
    assert_eq!(
        bin()
            .arg("generate")
            .arg(p.path("docs"))
            .output()
            .expect("run")
            .status
            .code(),
        Some(1)
    );
    assert_eq!(
        bin().arg("generate").output().expect("run").status.code(),
        Some(2)
    );
}

#[test]
fn check_stays_read_only() {
    let p = Proj::valid();
    p.remove("docs/rfc/index.rst");
    let before = p.snapshot();
    assert_eq!(
        bin()
            .arg("check")
            .arg(&p.root)
            .output()
            .expect("run")
            .status
            .code(),
        Some(1)
    );
    assert_eq!(p.snapshot(), before);
}
