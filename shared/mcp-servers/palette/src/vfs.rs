//! File access for lint and the write tools: a read trait, an in-memory overlay of
//! pending changes, the per-project lock, and the all-or-nothing commit.
//!
//! Owns reading through a [`FileSource`], recording what the overlay read (so a change
//! made between read and write is a `conflict`), the advisory lock, temp-file writes,
//! renames and rollback, and unified diffs. Does not know palette documents.
//! Entry points: [`DiskSource`], [`Overlay`], [`ProjectLock`], [`commit`], [`unified_diff`].

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::errors::{ErrCode, PalError, Res};
use crate::project::check_write_path;

/// One directory entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    /// File or folder name.
    pub name: String,
    /// Whether it is a folder.
    pub is_dir: bool,
}

/// Read access to a tree of files.
pub trait FileSource {
    /// The bytes of `path`, or `None` when it does not exist.
    fn read(&self, path: &Path) -> io::Result<Option<Vec<u8>>>;
    /// The entries of `dir`, sorted by name; empty when `dir` does not exist.
    fn list(&self, dir: &Path) -> io::Result<Vec<DirEntry>>;
    /// Whether `path` is an existing folder.
    fn is_dir(&self, path: &Path) -> bool;
}

/// The real file system.
pub struct DiskSource;

impl FileSource for DiskSource {
    fn read(&self, path: &Path) -> io::Result<Option<Vec<u8>>> {
        match fs::read(path) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) if path.is_dir() => {
                let _ = e;
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    fn list(&self, dir: &Path) -> io::Result<Vec<DirEntry>> {
        let rd = match fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(e)
                if e.kind() == io::ErrorKind::NotFound
                    || e.kind() == io::ErrorKind::NotADirectory =>
            {
                return Ok(Vec::new());
            }
            Err(e) => return Err(e),
        };
        let mut out = Vec::new();
        for entry in rd {
            let entry = entry?;
            let is_dir = entry.path().is_dir();
            out.push(DirEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir,
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }
}

/// One file whose content will differ: path, content when first read, content to write
/// (`None` for a missing file on either side).
pub type FileChange = (PathBuf, Option<Vec<u8>>, Option<Vec<u8>>);

/// A pending change to one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// Create or replace the file with these bytes.
    Write(Vec<u8>),
    /// Delete the file.
    Delete,
}

/// A view of a base source with pending changes applied. Every read of the base is
/// remembered, so the commit can tell whether the disk moved underneath the operation.
pub struct Overlay<'a> {
    base: &'a dyn FileSource,
    changes: BTreeMap<PathBuf, Change>,
    baseline: RefCell<BTreeMap<PathBuf, Option<Vec<u8>>>>,
}

impl<'a> Overlay<'a> {
    /// An overlay with no changes.
    pub fn new(base: &'a dyn FileSource) -> Overlay<'a> {
        Overlay {
            base,
            changes: BTreeMap::new(),
            baseline: RefCell::new(BTreeMap::new()),
        }
    }

    fn remember(&self, path: &Path) -> io::Result<()> {
        if !self.baseline.borrow().contains_key(path) {
            let bytes = self.base.read(path)?;
            self.baseline.borrow_mut().insert(path.to_path_buf(), bytes);
        }
        Ok(())
    }

    /// Sets the content of `path`.
    pub fn write(&mut self, path: &Path, bytes: Vec<u8>) -> io::Result<()> {
        self.remember(path)?;
        self.changes
            .insert(path.to_path_buf(), Change::Write(bytes));
        Ok(())
    }

    /// Deletes `path`.
    pub fn delete(&mut self, path: &Path) -> io::Result<()> {
        self.remember(path)?;
        self.changes.insert(path.to_path_buf(), Change::Delete);
        Ok(())
    }

    /// The pending changes, keyed by absolute path.
    pub fn changes(&self) -> &BTreeMap<PathBuf, Change> {
        &self.changes
    }

    /// Files whose overlay content differs from what the base held when first read.
    pub fn effective_changes(&self) -> Vec<FileChange> {
        let mut out = Vec::new();
        for (p, c) in &self.changes {
            let old = self.baseline.borrow().get(p).cloned().unwrap_or(None);
            let new = match c {
                Change::Write(b) => Some(b.clone()),
                Change::Delete => None,
            };
            if old != new {
                out.push((p.clone(), old, new));
            }
        }
        out
    }

    /// What the base held for `path` when this overlay first read it.
    pub fn baseline_of(&self, path: &Path) -> Option<Option<Vec<u8>>> {
        self.baseline.borrow().get(path).cloned()
    }

    /// Whether `path` was touched by a change.
    pub fn is_changed(&self, path: &Path) -> bool {
        self.changes.contains_key(path)
    }
}

impl FileSource for Overlay<'_> {
    fn read(&self, path: &Path) -> io::Result<Option<Vec<u8>>> {
        match self.changes.get(path) {
            Some(Change::Write(b)) => Ok(Some(b.clone())),
            Some(Change::Delete) => Ok(None),
            None => {
                self.remember(path)?;
                Ok(self.baseline.borrow().get(path).cloned().unwrap_or(None))
            }
        }
    }

    fn list(&self, dir: &Path) -> io::Result<Vec<DirEntry>> {
        let mut map: BTreeMap<String, bool> = BTreeMap::new();
        for e in self.base.list(dir)? {
            let full = dir.join(&e.name);
            if !e.is_dir && matches!(self.changes.get(&full), Some(Change::Delete)) {
                continue;
            }
            map.insert(e.name, e.is_dir);
        }
        for (p, c) in &self.changes {
            if let Change::Write(_) = c
                && let Ok(rest) = p.strip_prefix(dir)
            {
                let mut comps = rest.components();
                if let Some(first) = comps.next() {
                    let name = first.as_os_str().to_string_lossy().into_owned();
                    let is_dir = comps.next().is_some();
                    map.entry(name)
                        .and_modify(|d| *d = *d || is_dir)
                        .or_insert(is_dir);
                }
            }
        }
        Ok(map
            .into_iter()
            .map(|(name, is_dir)| DirEntry { name, is_dir })
            .collect())
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.base.is_dir(path)
            || self
                .changes
                .iter()
                .any(|(p, c)| matches!(c, Change::Write(_)) && p.starts_with(path) && p != path)
    }
}

/// The per-project advisory lock (`_palette/.palette.lock`), released on drop or when
/// the process dies.
pub struct ProjectLock {
    _file: fs::File,
}

impl ProjectLock {
    /// Takes the lock, retrying until `timeout` has passed. The lock file is created
    /// when missing. Returns `locked` on timeout.
    pub fn acquire(path: &Path, timeout: Duration) -> Res<ProjectLock> {
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| PalError::io(&format!("opening lock file {}", path.display()), &e))?;
        let start = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(ProjectLock { _file: file }),
                Err(fs::TryLockError::WouldBlock) => {
                    if start.elapsed() >= timeout {
                        return Err(PalError::new(
                            ErrCode::Locked,
                            format!(
                                "another palette operation holds {} (waited {} s). Retry after it \
                                 finishes; if no palette process is running, the lock is stale \
                                 only if the file is held by a process you can find and stop.",
                                path.display(),
                                timeout.as_secs_f32()
                            ),
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(fs::TryLockError::Error(e)) => {
                    return Err(PalError::io(&format!("locking {}", path.display()), &e));
                }
            }
        }
    }
}

/// One step of the commit, reported to the observer before it runs.
pub type Observer<'a> = &'a mut dyn FnMut(usize) -> io::Result<()>;

enum Undo {
    /// The file was created; remove it.
    Created(PathBuf),
    /// The file was replaced or deleted; restore these bytes.
    Restore(PathBuf, Vec<u8>),
}

fn tmp_sibling(path: &Path, n: usize) -> PathBuf {
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    path.with_file_name(format!(".{name}.palette-tmp-{}-{n}", std::process::id()))
}

fn write_file(path: &Path, bytes: &[u8], n: usize) -> io::Result<()> {
    let tmp = tmp_sibling(path, n);
    let res = (|| {
        let mut f = fs::File::create(&tmp)?;
        io::Write::write_all(&mut f, bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res
}

/// Writes every pending change of `overlay` to disk as one transaction.
///
/// Order: check write paths and detect conflicts, create parent folders, write each new
/// content to a temporary sibling, rename them into place, delete files, remove folders
/// that became empty. `observer(i)` runs before step `i` of the rename/delete phase; an
/// error from it (or any failed step) restores every file to its previous content.
pub fn commit(root: &Path, overlay: &Overlay<'_>, observer: Observer<'_>) -> Res<()> {
    let changes = overlay.effective_changes();
    if changes.is_empty() {
        return Ok(());
    }
    for (path, _, _) in &changes {
        check_write_path(root, path)?;
    }
    // Conflict detection: the disk must still hold what the operation read.
    for (path, old, _) in &changes {
        let now = fs::read(path).ok();
        if &now != old {
            return Err(PalError::new(
                ErrCode::Conflict,
                format!(
                    "{} changed after it was read. Nothing was written; re-run the operation.",
                    path.display()
                ),
            ));
        }
    }

    let mut created_dirs: Vec<PathBuf> = Vec::new();
    let mut tmp_files: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut undo: Vec<Undo> = Vec::new();
    let mut removed_dirs: Vec<PathBuf> = Vec::new();

    let result = (|| -> Res<()> {
        let mut n = 0usize;
        for (path, _, new) in &changes {
            let Some(bytes) = new else { continue };
            if let Some(parent) = path.parent() {
                let mut missing: Vec<PathBuf> = Vec::new();
                let mut cur = parent.to_path_buf();
                while !cur.exists() && cur != root {
                    missing.push(cur.clone());
                    if !cur.pop() {
                        break;
                    }
                }
                fs::create_dir_all(parent)
                    .map_err(|e| PalError::io(&format!("creating {}", parent.display()), &e))?;
                missing.reverse();
                created_dirs.extend(missing);
            }
            let tmp = tmp_sibling(path, n);
            n += 1;
            let mut f = fs::File::create(&tmp)
                .map_err(|e| PalError::io(&format!("writing {}", tmp.display()), &e))?;
            io::Write::write_all(&mut f, bytes)
                .and_then(|_| f.sync_all())
                .map_err(|e| PalError::io(&format!("writing {}", tmp.display()), &e))?;
            tmp_files.push((tmp, path.clone()));
        }
        let mut step = 0usize;
        for (tmp, path) in &tmp_files {
            observer(step).map_err(|e| PalError::io("committing", &e))?;
            step += 1;
            let previous = fs::read(path).ok();
            fs::rename(tmp, path)
                .map_err(|e| PalError::io(&format!("replacing {}", path.display()), &e))?;
            undo.push(match previous {
                Some(b) => Undo::Restore(path.clone(), b),
                None => Undo::Created(path.clone()),
            });
        }
        tmp_files.clear();
        for (path, old, new) in &changes {
            if new.is_some() {
                continue;
            }
            observer(step).map_err(|e| PalError::io("committing", &e))?;
            step += 1;
            fs::remove_file(path)
                .map_err(|e| PalError::io(&format!("deleting {}", path.display()), &e))?;
            if let Some(b) = old {
                undo.push(Undo::Restore(path.clone(), b.clone()));
            }
        }
        // Folders left empty by deletions go too.
        let mut parents: BTreeSet<PathBuf> = BTreeSet::new();
        for (path, _, new) in &changes {
            if new.is_none() {
                let mut cur = path.parent().map(Path::to_path_buf);
                while let Some(d) = cur {
                    if d == root
                        || d.file_name()
                            .is_some_and(|n| n == crate::layout::PALETTE_DIR)
                    {
                        break;
                    }
                    parents.insert(d.clone());
                    cur = d.parent().map(Path::to_path_buf);
                }
            }
        }
        let mut ordered: Vec<PathBuf> = parents.into_iter().collect();
        ordered.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
        for d in ordered {
            if fs::remove_dir(&d).is_ok() {
                removed_dirs.push(d);
            }
        }
        Ok(())
    })();

    if let Err(e) = result {
        let mut problems: Vec<String> = Vec::new();
        for (tmp, _) in &tmp_files {
            let _ = fs::remove_file(tmp);
        }
        for d in removed_dirs.iter().rev() {
            if let Err(err) = fs::create_dir_all(d) {
                problems.push(format!("recreating {}: {err}", d.display()));
            }
        }
        for u in undo.iter().rev() {
            let r = match u {
                Undo::Created(p) => fs::remove_file(p),
                Undo::Restore(p, b) => {
                    if let Some(parent) = p.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    write_file(p, b, 9_999)
                }
            };
            if let Err(err) = r {
                problems.push(format!("{err}"));
            }
        }
        for d in created_dirs.iter().rev() {
            let _ = fs::remove_dir(d);
        }
        if problems.is_empty() {
            return Err(e);
        }
        return Err(PalError::new(
            ErrCode::IoError,
            format!(
                "{}. Rollback also failed for: {}. Check these files by hand.",
                e.message,
                problems.join("; ")
            ),
        ));
    }
    Ok(())
}

/// Unified diff of one file. `old`/`new` are `None` for a missing file. Line endings
/// are shown as `\n` so a CRLF file reads normally.
pub fn unified_diff(rel: &str, old: Option<&[u8]>, new: Option<&[u8]>) -> String {
    let text = |b: Option<&[u8]>| {
        b.map(|b| String::from_utf8_lossy(b).replace("\r\n", "\n"))
            .unwrap_or_default()
    };
    let (a, b) = (text(old), text(new));
    let from = if old.is_some() {
        format!("a/{rel}")
    } else {
        "/dev/null".to_string()
    };
    let to = if new.is_some() {
        format!("b/{rel}")
    } else {
        "/dev/null".to_string()
    };
    let diff = similar::TextDiff::from_lines(&a, &b);
    diff.unified_diff()
        .context_radius(3)
        .header(&from, &to)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> (tempfile::TempDir, PathBuf) {
        let t = tempfile::tempdir().expect("tempdir");
        let r = t.path().canonicalize().expect("canon");
        (t, r)
    }

    #[test]
    fn overlay_reads_writes_and_lists() {
        let (_t, r) = root();
        fs::write(r.join("a.rst"), "a").expect("write");
        fs::write(r.join("b.rst"), "b").expect("write");
        let disk = DiskSource;
        let mut o = Overlay::new(&disk);
        o.write(&r.join("c.rst"), b"c".to_vec()).expect("w");
        o.delete(&r.join("a.rst")).expect("d");
        o.write(&r.join("sub").join("d.rst"), b"d".to_vec())
            .expect("w");
        assert_eq!(o.read(&r.join("a.rst")).expect("read"), None);
        assert_eq!(o.read(&r.join("c.rst")).expect("read"), Some(b"c".to_vec()));
        let names: Vec<String> = o
            .list(&r)
            .expect("list")
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, vec!["b.rst", "c.rst", "sub"]);
        assert!(o.is_dir(&r.join("sub")));
        assert_eq!(o.effective_changes().len(), 3);
    }

    #[test]
    fn commit_writes_creates_deletes_and_prunes_folders() {
        let (_t, r) = root();
        fs::create_dir_all(r.join("old")).expect("mkdir");
        fs::write(r.join("old").join("x.rst"), "x").expect("write");
        fs::write(r.join("keep.rst"), "one").expect("write");
        let disk = DiskSource;
        let mut o = Overlay::new(&disk);
        o.read(&r.join("keep.rst")).expect("read");
        o.write(&r.join("keep.rst"), b"two".to_vec()).expect("w");
        o.write(&r.join("new").join("n.rst"), b"n".to_vec())
            .expect("w");
        o.delete(&r.join("old").join("x.rst")).expect("d");
        commit(&r, &o, &mut |_| Ok(())).expect("commit");
        assert_eq!(fs::read_to_string(r.join("keep.rst")).expect("read"), "two");
        assert_eq!(
            fs::read_to_string(r.join("new").join("n.rst")).expect("read"),
            "n"
        );
        assert!(!r.join("old").exists());
    }

    #[test]
    fn failure_after_first_rename_restores_everything() {
        let (_t, r) = root();
        fs::create_dir_all(r.join("gone")).expect("mkdir");
        fs::write(r.join("gone").join("g.rst"), "g").expect("write");
        fs::write(r.join("a.rst"), "a1").expect("write");
        fs::write(r.join("b.rst"), "b1").expect("write");
        let disk = DiskSource;
        let mut o = Overlay::new(&disk);
        o.read(&r.join("a.rst")).expect("r");
        o.read(&r.join("b.rst")).expect("r");
        o.write(&r.join("a.rst"), b"a2".to_vec()).expect("w");
        o.write(&r.join("b.rst"), b"b2".to_vec()).expect("w");
        o.write(&r.join("fresh").join("f.rst"), b"f".to_vec())
            .expect("w");
        o.delete(&r.join("gone").join("g.rst")).expect("d");
        let err = commit(&r, &o, &mut |i| {
            if i == 1 {
                Err(io::Error::other("injected"))
            } else {
                Ok(())
            }
        })
        .expect_err("must fail");
        assert_eq!(err.code, ErrCode::IoError);
        assert_eq!(fs::read_to_string(r.join("a.rst")).expect("a"), "a1");
        assert_eq!(fs::read_to_string(r.join("b.rst")).expect("b"), "b1");
        assert_eq!(
            fs::read_to_string(r.join("gone").join("g.rst")).expect("g"),
            "g"
        );
        assert!(!r.join("fresh").exists());
        let stray: Vec<_> = fs::read_dir(&r)
            .expect("dir")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("palette-tmp"))
            .collect();
        assert!(stray.is_empty());
    }

    #[test]
    fn failure_during_delete_phase_restores_writes_and_deletes() {
        let (_t, r) = root();
        fs::write(r.join("a.rst"), "a1").expect("write");
        fs::write(r.join("d.rst"), "d1").expect("write");
        let disk = DiskSource;
        let mut o = Overlay::new(&disk);
        o.read(&r.join("a.rst")).expect("r");
        o.write(&r.join("a.rst"), b"a2".to_vec()).expect("w");
        o.delete(&r.join("d.rst")).expect("d");
        commit(&r, &o, &mut |i| {
            if i == 1 {
                Err(io::Error::other("x"))
            } else {
                Ok(())
            }
        })
        .expect_err("fail");
        assert_eq!(fs::read_to_string(r.join("a.rst")).expect("a"), "a1");
        assert_eq!(fs::read_to_string(r.join("d.rst")).expect("d"), "d1");
    }

    #[test]
    fn conflict_when_disk_changed_after_read() {
        let (_t, r) = root();
        fs::write(r.join("a.rst"), "one").expect("write");
        let disk = DiskSource;
        let mut o = Overlay::new(&disk);
        o.read(&r.join("a.rst")).expect("r");
        o.write(&r.join("a.rst"), b"two".to_vec()).expect("w");
        fs::write(r.join("a.rst"), "hand edit").expect("hand edit");
        let err = commit(&r, &o, &mut |_| Ok(())).expect_err("conflict");
        assert_eq!(err.code, ErrCode::Conflict);
        assert_eq!(fs::read_to_string(r.join("a.rst")).expect("a"), "hand edit");
    }

    #[test]
    fn lock_times_out_while_held() {
        let (_t, r) = root();
        let p = r.join("lock");
        let held = ProjectLock::acquire(&p, Duration::from_secs(1)).expect("first");
        let err = ProjectLock::acquire(&p, Duration::from_millis(150))
            .err()
            .expect("locked");
        assert_eq!(err.code, ErrCode::Locked);
        drop(held);
        assert!(ProjectLock::acquire(&p, Duration::from_millis(150)).is_ok());
    }

    #[test]
    fn diff_shows_new_and_changed_files() {
        let d = unified_diff("a.rst", Some(b"x\ny\n"), Some(b"x\nz\n"));
        assert!(d.contains("--- a/a.rst") && d.contains("+++ b/a.rst"));
        assert!(d.contains("-y") && d.contains("+z"));
        let d = unified_diff("n.rst", None, Some(b"hi\n"));
        assert!(d.contains("--- /dev/null") && d.contains("+hi"));
    }
}
