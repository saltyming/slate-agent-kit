//! Command-line entry points that do not need the MCP server.
//!
//! Owns `palette check <project>` (read-only lint), `palette generate <project>`
//! (regenerates the indexes and staging documents) and `palette reflow <project>
//! [paths]` (rewraps the prose of the project's documents). All resolve the project
//! without the root rules. Entry points: [`check`], [`generate`], [`reflow`].

use std::path::PathBuf;

use crate::docs::Snapshot;
use crate::lint::{self, Analysis, Severity};
use crate::ops::{self, Ctx};
use crate::project::{Roots, canonical_dir};
use crate::vfs::DiskSource;

/// Lints `project` and prints the findings; returns the process exit code (1 on error).
pub fn check(project: &str) -> i32 {
    // The command line accepts a relative path; the tools require an absolute one.
    let abs: PathBuf = std::env::current_dir().unwrap_or_default().join(project);
    let root: PathBuf = match canonical_dir(&abs.to_string_lossy()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("palette check: {e}");
            return 2;
        }
    };
    let snap = match Snapshot::load(&DiskSource, &root) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("palette check: {e}");
            return 2;
        }
    };
    let an = Analysis::build(&snap);
    let findings = lint::run(&DiskSource, &snap, &an);
    print!("{}", lint::to_text(&findings));
    i32::from(findings.iter().any(|f| f.severity == Severity::Error))
}

/// Regenerates every index and staging document of `project` under the same lock and
/// all-or-nothing transaction as the write tools, prints the paths it wrote, and returns
/// the process exit code (0 on success, 1 on error).
pub fn generate(project: &str) -> i32 {
    let abs: PathBuf = std::env::current_dir().unwrap_or_default().join(project);
    let root: PathBuf = match canonical_dir(&abs.to_string_lossy()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("palette generate: {e}");
            return 1;
        }
    };
    // The command line resolves the project without the root rules: the project itself
    // is the only root.
    let ctx = Ctx::new(Roots {
        project_root: Some(root.clone()),
        extra_roots: Vec::new(),
    });
    match ops::generate(&ctx, &root.to_string_lossy()) {
        Ok(result) => {
            if let Some(m) = &result.message {
                println!("{m}");
            }
            for (file, action) in &result.files {
                println!("{action} {file}");
            }
            0
        }
        Err(e) => {
            eprintln!("palette generate: {e}");
            1
        }
    }
}

/// Rewraps the prose of the documents of `project` at 79 columns, or of those under
/// `paths` (project-relative files or folders), under the same lock and all-or-nothing
/// transaction as the write tools (indexes and staging regenerated from the rewrapped
/// text, the result linted, only changed files written). Prints the paths it wrote and
/// returns the process exit code (0 on success, 1 on error).
pub fn reflow(project: &str, paths: &[String]) -> i32 {
    let abs: PathBuf = std::env::current_dir().unwrap_or_default().join(project);
    let root: PathBuf = match canonical_dir(&abs.to_string_lossy()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("palette reflow: {e}");
            return 1;
        }
    };
    let ctx = Ctx::new(Roots {
        project_root: Some(root.clone()),
        extra_roots: Vec::new(),
    });
    match ops::reflow::reflow(&ctx, &root.to_string_lossy(), paths) {
        Ok(result) => {
            if let Some(m) = &result.message {
                println!("{m}");
            }
            for (file, action) in &result.files {
                println!("{action} {file}");
            }
            0
        }
        Err(e) => {
            eprintln!("palette reflow: {e}");
            1
        }
    }
}
