//! Shared setup for the end-to-end tests.
//!
//! Owns an isolated sandbox (a temporary user home, a copy of a fixture payload,
//! the stand-in harness CLI wired in through `CLAUDE_BIN` and `CODEX_BIN`), a
//! local HTTP mirror of a slate release, and helpers to run the installer and to
//! read what the stand-in CLI recorded. Nothing here touches the real home.

#![allow(dead_code)]

use flate2::Compression;
use flate2::write::GzEncoder;
use slate_setup::binaries::{exe_name, sha256_hex};
use slate_setup::env::{Env, Harness};
use slate_setup::json::Json;
use slate_setup::ui::{Ui, new_sink, sink_text};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use tempfile::TempDir;

/// Path of a file under `tests/fixtures`.
pub fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(rel)
}

/// Path of the stand-in harness CLI built by cargo for the tests.
pub fn fake_cli() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fake-harness-cli"))
}

/// Text with Windows line endings normalised, so assertions hold with `core.autocrlf`.
pub fn norm(s: &str) -> String {
    s.replace("\r\n", "\n")
}

/// Reads a file and normalises its line endings.
pub fn read(path: &Path) -> String {
    norm(
        &std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display())),
    )
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).unwrap();
        }
    }
}

/// Result of one installer run.
pub struct Outcome {
    /// Process exit code.
    pub code: i32,
    /// Everything the installer printed.
    pub out: String,
}

/// An isolated installer environment for one harness.
pub struct Sandbox {
    /// The temporary folder that holds everything.
    pub dir: TempDir,
    /// The installer's environment.
    pub env: Env,
    /// Harness under test.
    pub harness: Harness,
    /// The user's home folder.
    pub user_home: PathBuf,
    /// The payload copy.
    pub payload: PathBuf,
    /// The binary folder.
    pub bin_dir: PathBuf,
    /// Log written by the stand-in CLI.
    pub log: PathBuf,
}

impl Sandbox {
    /// Creates a sandbox with a copy of the fixture payload of `harness`.
    pub fn new(harness: Harness) -> Sandbox {
        let dir = tempfile::tempdir().unwrap();
        let user_home = dir.path().join("home");
        std::fs::create_dir_all(&user_home).unwrap();
        let payload = dir.path().join("dist");
        copy_dir(&fixture(&format!("payload-{}", harness.name())), &payload);
        let bin_dir = dir.path().join("bin");
        let log = dir.path().join("cli.log");
        let mut env = Env::for_home(&user_home);
        env.set_var("CLAUDE_BIN", fake_cli().to_string_lossy());
        env.set_var("CODEX_BIN", fake_cli().to_string_lossy());
        env.child_env
            .insert("FAKE_CLI_LOG".into(), log.to_string_lossy().into_owned());
        Sandbox {
            dir,
            env,
            harness,
            user_home,
            payload,
            bin_dir,
            log,
        }
    }

    /// The harness home under the sandbox's user home.
    pub fn home(&self) -> PathBuf {
        self.env.default_home(self.harness)
    }

    /// The kit name of the fixture.
    pub fn kit(&self) -> String {
        format!("{}-agent-kit", self.harness.name())
    }

    /// The path of an installed prefs file.
    pub fn prefs_path(&self, name: &str) -> PathBuf {
        self.home()
            .join("rules")
            .join(format!("{}--{name}-prefs.md", self.kit()))
    }

    /// Points the installer at a release mirror.
    pub fn use_release(&mut self, server: &ReleaseServer) {
        self.env.set_var("SLATE_RELEASE_BASE_URL", server.url());
    }

    fn base_args(&self, cmd: &str) -> Vec<String> {
        vec![
            "slate-setup".into(),
            cmd.into(),
            "--payload".into(),
            self.payload.to_string_lossy().into_owned(),
            "--bin-dir".into(),
            self.bin_dir.to_string_lossy().into_owned(),
        ]
    }

    /// Runs `slate-setup <cmd> <extra...> --yes` without a terminal.
    pub fn run(&self, cmd: &str, extra: &[&str]) -> Outcome {
        let mut args = self.base_args(cmd);
        args.extend(extra.iter().map(|s| s.to_string()));
        args.push("--yes".into());
        let sink = new_sink();
        let code = slate_setup::run(
            args.into_iter().map(Into::into),
            &self.env,
            Some(Ui::captured(sink.clone())),
        );
        Outcome {
            code,
            out: sink_text(&sink),
        }
    }

    /// Runs `slate-setup <cmd> <extra...>` with `answers` typed at a terminal.
    pub fn run_scripted(&self, cmd: &str, extra: &[&str], answers: &str) -> Outcome {
        let mut args = self.base_args(cmd);
        args.extend(extra.iter().map(|s| s.to_string()));
        let sink = new_sink();
        let code = slate_setup::run(
            args.into_iter().map(Into::into),
            &self.env,
            Some(Ui::scripted(answers, sink.clone())),
        );
        Outcome {
            code,
            out: sink_text(&sink),
        }
    }

    /// The invocations the stand-in CLI recorded, as argument lists.
    pub fn cli_calls(&self) -> Vec<Vec<String>> {
        let Ok(text) = std::fs::read_to_string(&self.log) else {
            return Vec::new();
        };
        text.lines()
            .map(|l| {
                let j = Json::parse(l).unwrap();
                j.get("args")
                    .unwrap()
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|a| a.as_str().unwrap().to_string())
                    .collect()
            })
            .collect()
    }

    /// The value of environment variable `key` seen by each recorded invocation that had it.
    pub fn cli_env(&self, key: &str) -> Vec<String> {
        let Ok(text) = std::fs::read_to_string(&self.log) else {
            return Vec::new();
        };
        text.lines()
            .filter_map(|l| {
                let j = Json::parse(l).ok()?;
                j.get("env")?.get(key)?.as_str().map(str::to_string)
            })
            .collect()
    }

    /// The recorded `mcp add` calls as `(server, "K=V" pairs)`.
    pub fn added_servers(&self) -> Vec<(String, Vec<String>)> {
        self.cli_calls()
            .into_iter()
            .filter(|c| c.get(1).map(String::as_str) == Some("add"))
            .map(|c| {
                let pairs = c
                    .windows(2)
                    .filter(|w| w[0] == "-e" || w[0] == "--env")
                    .map(|w| w[1].clone())
                    .collect();
                (c[2].clone(), pairs)
            })
            .collect()
    }

    /// The recorded `mcp remove` calls, as server names.
    pub fn removed_servers(&self) -> Vec<String> {
        self.cli_calls()
            .into_iter()
            .filter(|c| c.get(1).map(String::as_str) == Some("remove"))
            .map(|c| c[2].clone())
            .collect()
    }

    /// Clears the CLI log.
    pub fn clear_log(&self) {
        let _ = std::fs::remove_file(&self.log);
    }

    /// Every file under the harness home, relative and sorted, for before/after comparisons.
    pub fn tree(&self, root: &Path) -> Vec<String> {
        let mut out = Vec::new();
        fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(base, &p, out);
                } else {
                    out.push(
                        p.strip_prefix(base)
                            .unwrap()
                            .to_string_lossy()
                            .replace('\\', "/"),
                    );
                }
            }
        }
        walk(root, root, &mut out);
        out.sort();
        out
    }
}

/// Builds the release archive of one binary for the current platform.
fn archive(platform: &str, name: &str, content: &[u8]) -> (String, Vec<u8>) {
    let exe = exe_name(platform, name);
    if platform.contains("windows") {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        w.start_file(&exe, zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(content).unwrap();
        (
            format!("{name}-{platform}.zip"),
            w.finish().unwrap().into_inner(),
        )
    } else {
        let gz = GzEncoder::new(Vec::new(), Compression::default());
        let mut tar = tar::Builder::new(gz);
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, &exe, content).unwrap();
        (
            format!("{name}-{platform}.tar.gz"),
            tar.into_inner().unwrap().finish().unwrap(),
        )
    }
}

/// A local HTTP server that mirrors a slate release.
pub struct ReleaseServer {
    addr: String,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    /// Requests served, as paths.
    pub requests: Arc<Mutex<Vec<String>>>,
}

impl ReleaseServer {
    /// Serves archives whose binaries are the stand-in CLI (so `palette --read-only-tools` works).
    ///
    /// `tamper` names a binary whose archive is replaced after its checksum was recorded.
    /// With `version` set to `None` no release exists under `v0.7.0`; only `latest` is served.
    ///
    /// The release also ships `agent-guard` for platforms that use it; see
    /// [`ReleaseServer::start_without_guard`] for an older release.
    pub fn start(
        platform: &str,
        tamper: Option<&str>,
        versioned: bool,
        with_checksums: bool,
    ) -> ReleaseServer {
        ReleaseServer::start_release(platform, tamper, versioned, with_checksums, true)
    }

    /// Like [`ReleaseServer::start`] for a release from before `agent-guard` was shipped.
    pub fn start_without_guard(platform: &str, versioned: bool) -> ReleaseServer {
        ReleaseServer::start_release(platform, None, versioned, true, false)
    }

    fn start_release(
        platform: &str,
        tamper: Option<&str>,
        versioned: bool,
        with_checksums: bool,
        with_guard: bool,
    ) -> ReleaseServer {
        let cli = std::fs::read(fake_cli()).unwrap();
        let mut files: HashMap<String, Vec<u8>> = HashMap::new();
        let mut sums = String::new();
        let prefix = "/saltyming/slate-agent-kit/releases";
        let dir = if versioned {
            format!("{prefix}/download/v0.7.0")
        } else {
            format!("{prefix}/latest/download")
        };
        let mut names = vec!["aside", "dispatch", "palette"];
        if with_guard && !platform.contains("windows") {
            names.push("agent-guard");
        }
        for name in names {
            let (file, bytes) = archive(platform, name, &cli);
            sums.push_str(&format!("{}  {file}\n", sha256_hex(&bytes)));
            let served = if tamper == Some(name) {
                archive(platform, name, b"tampered").1
            } else {
                bytes
            };
            files.insert(format!("{dir}/{file}"), served);
        }
        if with_checksums {
            files.insert(format!("{dir}/checksums.txt"), sums.into_bytes());
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (stop2, requests2) = (stop.clone(), requests.clone());
        let handle = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stop2.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                serve_one(stream, &files, &requests2);
            }
        });
        ReleaseServer {
            addr,
            stop,
            handle: Some(handle),
            requests,
        }
    }

    /// The base URL to give the installer as `SLATE_RELEASE_BASE_URL`.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

impl Drop for ReleaseServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(&self.addr);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn serve_one(
    mut stream: TcpStream,
    files: &HashMap<String, Vec<u8>>,
    requests: &Arc<Mutex<Vec<String>>>,
) {
    let mut buf = [0u8; 4096];
    let mut request = Vec::new();
    loop {
        let Ok(n) = stream.read(&mut buf) else { return };
        if n == 0 {
            return;
        }
        request.extend_from_slice(&buf[..n]);
        if request.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&request);
    let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
    if let Ok(mut r) = requests.lock() {
        r.push(path.clone());
    }
    let (status, body): (&str, &[u8]) = match files.get(&path) {
        Some(b) => ("200 OK", b),
        None => ("404 Not Found", b"not found"),
    };
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}
