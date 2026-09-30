//! Shared helpers for the integration tests: fixture projects in temporary folders,
//! file access by project-relative path, lint shortcuts and tool calls from JSON.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use palette_server::docs::Snapshot;
use palette_server::errors::PalError;
use palette_server::lint::{self, Analysis, Finding, Severity};
use palette_server::ops::{Ctx, WriteResult};
use palette_server::project::Roots;
use palette_server::vfs::DiskSource;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

pub const TODAY: &str = "2026-09-30";

/// A project in a temporary folder.
pub struct Proj {
    _tmp: tempfile::TempDir,
    pub root: PathBuf,
    pub ctx: Ctx,
}

fn copy_dir(a: &Path, b: &Path) {
    std::fs::create_dir_all(b).expect("mkdir");
    for e in std::fs::read_dir(a).expect("read_dir") {
        let e = e.expect("entry");
        let t = b.join(e.file_name());
        if e.path().is_dir() {
            copy_dir(&e.path(), &t);
        } else {
            std::fs::copy(e.path(), &t).expect("copy");
        }
    }
}

impl Proj {
    /// An empty project folder.
    pub fn empty() -> Proj {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().canonicalize().expect("canonicalize");
        let mut ctx = Ctx::new(Roots {
            project_root: Some(root.clone()),
            extra_roots: Vec::new(),
        });
        ctx.clock = Arc::new(|| TODAY.to_string());
        ctx.lock_timeout = std::time::Duration::from_millis(300);
        Proj {
            _tmp: tmp,
            root,
            ctx,
        }
    }

    /// A copy of `tests/fixtures/<name>`.
    pub fn fixture(name: &str) -> Proj {
        let p = Proj::empty();
        let src = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name);
        copy_dir(&src, &p.root);
        p
    }

    /// A copy of the valid fixture.
    pub fn valid() -> Proj {
        Proj::fixture("valid")
    }

    /// The valid fixture with every file converted to CRLF line endings.
    pub fn valid_crlf() -> Proj {
        let p = Proj::valid();
        for rel in p.files() {
            let text = p.read(&rel);
            p.write(&rel, &text.replace("\r\n", "\n").replace('\n', "\r\n"));
        }
        p
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        rel.split('/').fold(self.root.clone(), |a, c| a.join(c))
    }

    pub fn arg(&self) -> String {
        self.root.to_string_lossy().into_owned()
    }

    pub fn read(&self, rel: &str) -> String {
        String::from_utf8(
            std::fs::read(self.path(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}")),
        )
        .expect("utf8")
    }

    pub fn write(&self, rel: &str, text: &str) {
        let p = self.path(rel);
        std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        std::fs::write(p, text).expect("write");
    }

    pub fn mutate(&self, rel: &str, f: impl FnOnce(&str) -> String) {
        let before = self.read(rel);
        let after = f(&before);
        assert_ne!(before, after, "mutation of {rel} changed nothing");
        self.write(rel, &after);
    }

    /// Replaces `from` with `to` in `rel`, requiring that `from` occurs.
    pub fn replace(&self, rel: &str, from: &str, to: &str) {
        self.mutate(rel, |s| {
            assert!(s.contains(from), "{rel} does not contain {from:?}");
            s.replacen(from, to, 1)
        });
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.path(rel).exists()
    }

    pub fn remove(&self, rel: &str) {
        std::fs::remove_file(self.path(rel)).expect("remove");
    }

    /// Every file, project-relative with `/`.
    pub fn files(&self) -> Vec<String> {
        fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).expect("read_dir") {
                let p = e.expect("entry").path();
                if p.is_dir() {
                    walk(&p, root, out);
                } else {
                    let rel: Vec<String> = p
                        .strip_prefix(root)
                        .expect("prefix")
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect();
                    out.push(rel.join("/"));
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &self.root, &mut out);
        out.sort();
        out
    }

    /// All file contents by relative path (lock and temp files excluded).
    pub fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
        self.files()
            .into_iter()
            .filter(|f| !f.ends_with(".palette.lock"))
            .map(|f| {
                let bytes = std::fs::read(self.path(&f)).expect("read");
                (f, bytes)
            })
            .collect()
    }

    pub fn lint(&self) -> Vec<Finding> {
        let snap = Snapshot::load(&DiskSource, &self.root).expect("load");
        let an = Analysis::build(&snap);
        lint::run(&DiskSource, &snap, &an)
    }

    pub fn errors(&self) -> Vec<Finding> {
        self.lint()
            .into_iter()
            .filter(|f| f.severity == Severity::Error)
            .collect()
    }
}

/// Whether some finding has `rule`, a file containing `file` and a message containing `needle`.
pub fn has(findings: &[Finding], rule: &str, file: &str, needle: &str) -> bool {
    findings
        .iter()
        .any(|f| f.rule == rule && f.file.contains(file) && f.message.contains(needle))
}

pub fn show(findings: &[Finding]) -> String {
    findings
        .iter()
        .map(|f| {
            format!(
                "{} {} {}:{} {}",
                f.severity.as_str(),
                f.rule,
                f.file,
                f.line,
                f.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Builds tool parameters from JSON, with `project` filled in.
pub fn params<T: DeserializeOwned>(p: &Proj, mut v: Value) -> T {
    v["project"] = json!(p.arg());
    serde_json::from_value(v).expect("parameters")
}

pub type ToolResult = Result<WriteResult, PalError>;
