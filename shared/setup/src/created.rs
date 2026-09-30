//! Folders and files the installer creates, recorded so that uninstall can remove them again.
//!
//! Owns the snapshot taken before a run (which candidate folders and files do
//! not exist yet), the recording of those that exist afterwards, and the removal
//! of recorded folders that are empty. It never removes a folder that holds
//! anything, and never one that already existed before the install.
//!
//! Main entry points: [`Watch`] and [`remove_empty`].

use crate::env::Harness;
use crate::harness::kimi;
use crate::manifest::Manifest;
use crate::ui::Ui;
use std::fs;
use std::path::{Path, PathBuf};

/// Candidate folders and files that did not exist when the snapshot was taken.
#[derive(Debug, Default)]
pub struct Watch {
    dirs: Vec<PathBuf>,
    files: Vec<PathBuf>,
}

impl Watch {
    /// Snapshots which of `dirs` (and their missing ancestors) and `files` do not exist yet.
    pub fn new(
        dirs: impl IntoIterator<Item = PathBuf>,
        files: impl IntoIterator<Item = PathBuf>,
    ) -> Watch {
        let mut missing_dirs: Vec<PathBuf> = Vec::new();
        for dir in dirs {
            let mut cur: &Path = &dir;
            while !cur.exists() {
                if !missing_dirs.iter().any(|d| d == cur) {
                    missing_dirs.push(cur.to_path_buf());
                }
                match cur.parent() {
                    Some(p) if !p.as_os_str().is_empty() => cur = p,
                    _ => break,
                }
            }
        }
        Watch {
            dirs: missing_dirs,
            files: files.into_iter().filter(|f| !f.exists()).collect(),
        }
    }

    /// Records into `manifest` the candidates that exist now.
    pub fn record(&self, manifest: &mut Manifest) {
        for d in self.dirs.iter().filter(|d| d.is_dir()) {
            manifest.record_created_dir(d);
        }
        for f in self.files.iter().filter(|f| f.is_file()) {
            manifest.record_created_file(f);
        }
    }
}

/// What an install may create, for [`watch_install`].
#[derive(Debug, Clone, Copy)]
pub struct Targets<'a> {
    /// The harness home.
    pub home: &'a Path,
    /// The install writes into `<home>/rules`.
    pub rules: bool,
    /// The install writes into `<home>/skills`.
    pub skills: bool,
    /// The binary folder, when binaries are installed or servers registered.
    pub bin_dir: Option<&'a Path>,
    /// Servers are registered (state folder, Kimi plugin and registry).
    pub servers: bool,
}

/// Takes the snapshot for an install into `harness`.
pub fn watch_install(harness: Harness, t: &Targets<'_>) -> Watch {
    let mut dirs = vec![t.home.to_path_buf()];
    let mut files = Vec::new();
    if t.rules {
        dirs.push(t.home.join("rules"));
    }
    if t.skills {
        dirs.push(t.home.join("skills"));
    }
    if let Some(b) = t.bin_dir {
        dirs.push(b.to_path_buf());
    }
    if t.servers {
        if harness != Harness::Claude {
            dirs.push(t.home.join("slate-agent-kit"));
        }
        if harness == Harness::Kimi {
            dirs.push(kimi::plugin_root(t.home));
            files.push(kimi::registry_path(t.home));
        }
    }
    Watch::new(dirs, files)
}

/// Removes each of `dirs` that exists and is empty, deepest first. Returns the ones removed.
pub fn remove_empty(ui: &mut Ui, dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut sorted: Vec<&PathBuf> = dirs.iter().collect();
    sorted.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    let mut removed = Vec::new();
    for d in sorted {
        // `remove_dir` fails on a folder that has anything in it, which is what we want.
        if fs::remove_dir(d).is_ok() {
            ui.ok(&format!("removed the empty folder {}", d.display()));
            removed.push(d.clone());
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::new_sink;

    #[test]
    fn only_paths_missing_at_snapshot_time_are_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("existing");
        fs::create_dir(&existing).unwrap();
        let new_chain = dir.path().join("a").join("b");
        let watch = Watch::new(
            [existing.clone(), new_chain.clone(), existing.join("skills")],
            [dir.path().join("f.json")],
        );
        fs::create_dir_all(&new_chain).unwrap();
        fs::write(dir.path().join("f.json"), "{}").unwrap();
        let mut m = Manifest::new("k");
        watch.record(&mut m);
        assert!(!m.created_dirs.contains(&existing));
        assert!(m.created_dirs.contains(&dir.path().join("a")));
        assert!(m.created_dirs.contains(&new_chain));
        assert!(
            !m.created_dirs.contains(&existing.join("skills")),
            "never created, so not recorded"
        );
        assert_eq!(m.created_files, vec![dir.path().join("f.json")]);
    }

    #[test]
    fn removal_takes_empty_folders_deepest_first_and_spares_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = a.join("b");
        let c = dir.path().join("c");
        fs::create_dir_all(&b).unwrap();
        fs::create_dir_all(&c).unwrap();
        fs::write(c.join("keep.txt"), "x").unwrap();
        let mut ui = Ui::captured(new_sink());
        let removed = remove_empty(
            &mut ui,
            &[a.clone(), b.clone(), c.clone(), dir.path().join("gone")],
        );
        assert_eq!(removed, vec![b, a.clone()]);
        assert!(!a.exists());
        assert!(c.exists());
    }
}
