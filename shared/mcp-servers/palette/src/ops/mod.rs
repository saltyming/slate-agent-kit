//! Write tools: the transaction wrapper and the tool implementations.
//!
//! Owns running an operation as one transaction: resolve the project, take the lock,
//! load the project through an overlay, let the operation edit files in memory,
//! regenerate the indexes and staging it affects, lint the files it touched, and
//! commit all files or none. Each tool's edit logic is in a submodule.
//! Entry points: [`run_write`], [`run_init`], [`Ctx`].

pub mod backlog;
pub mod init;
pub mod records;
pub mod state;

use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::docs::{DocFile, Snapshot};
use crate::errors::{PalError, Res};
use crate::generated;
use crate::lint::{self, Analysis, Severity};
use crate::project::Roots;
use crate::text::Source;
use crate::util::{now_utc, rel_slash, today_utc};
use crate::vfs::{self, DiskSource, FileSource, Overlay, ProjectLock};

/// Callback run before each commit step; a test seam that is a no-op in production.
pub type CommitHook = Arc<dyn Fn(usize) -> io::Result<()> + Send + Sync>;

/// Settings shared by every tool call.
#[derive(Clone)]
pub struct Ctx {
    /// The containment boundary.
    pub roots: Roots,
    /// How long a write waits for the project lock.
    pub lock_timeout: Duration,
    /// Today's date as `YYYY-MM-DD`.
    pub clock: Arc<dyn Fn() -> String + Send + Sync>,
    /// The current UTC time as `YYYY-MM-DDTHH:MMZ` (written into `Accepted`).
    pub clock_now: Arc<dyn Fn() -> String + Send + Sync>,
    /// Test seam: called before each rename or delete of a commit.
    pub commit_hook: Option<CommitHook>,
}

impl Ctx {
    /// Production settings: 10 second lock wait, UTC date.
    pub fn new(roots: Roots) -> Ctx {
        Ctx {
            roots,
            lock_timeout: Duration::from_secs(10),
            clock: Arc::new(today_utc),
            clock_now: Arc::new(now_utc),
            commit_hook: None,
        }
    }
}

/// What a write tool reports.
#[derive(Clone, Debug)]
pub struct WriteResult {
    /// Whether nothing was written.
    pub dry_run: bool,
    /// Identifiers the operation allocated.
    pub allocated: Vec<String>,
    /// Changed files: project-relative path and `created`, `modified` or `deleted`.
    pub files: Vec<(String, &'static str)>,
    /// Unified diff of every changed file.
    pub diff: String,
}

impl WriteResult {
    /// The JSON text returned to the caller.
    pub fn to_json(&self) -> String {
        let files: Vec<serde_json::Value> = self
            .files
            .iter()
            .map(|(f, a)| serde_json::json!({"file": f, "action": a}))
            .collect();
        serde_json::to_string_pretty(&serde_json::json!({
            "dry_run": self.dry_run,
            "allocated": self.allocated,
            "files": files,
            "diff": self.diff,
        }))
        .unwrap_or_else(|_| "{}".to_string())
    }
}

/// The view an operation edits: the project as it is now plus an overlay for changes.
pub struct Session<'o, 'a> {
    /// Pending changes.
    pub ov: &'o mut Overlay<'a>,
    /// The project before the operation.
    pub snap: Snapshot,
    /// Facts derived from `snap`.
    pub an: Analysis,
    /// Today's date.
    pub today: String,
    /// The current UTC time to the minute.
    pub now: String,
}

impl Session<'_, '_> {
    /// Project-relative text of `path`.
    pub fn rel(&self, path: &Path) -> String {
        rel_slash(&self.snap.root, path)
    }

    /// The loaded document at `path`, or `not_found`/`parse_error`.
    pub fn file(&self, path: &Path) -> Res<&DocFile> {
        let f = self
            .snap
            .file(path)
            .ok_or_else(|| PalError::not_found(format!("{} does not exist", self.rel(path))))?;
        if let Some(line) = f.bad_utf8 {
            return Err(PalError::parse(&f.rel, line, "the file is not valid UTF-8"));
        }
        Ok(f)
    }

    /// A copy of the lines of the document at `path`.
    pub fn source(&self, path: &Path) -> Res<Source> {
        self.file(path)?
            .doc
            .as_ref()
            .map(|d| d.src.clone())
            .ok_or_else(|| PalError::not_found(format!("{} cannot be read", self.rel(path))))
    }

    /// The error for a record that is not among the parsed records: `parse_error` when
    /// its file exists but is not valid UTF-8, else `not_found`.
    pub fn missing_record(&self, id: crate::records::RecId) -> PalError {
        let prefix = format!("{}-{:04}", id.kind.lower(), id.number);
        for f in &self.snap.files {
            if matches!(f.role, crate::docs::Role::Rfc | crate::docs::Role::Adr)
                && f.name.starts_with(&prefix)
                && let Some(line) = f.bad_utf8
            {
                return PalError::parse(&f.rel, line, "the file is not valid UTF-8");
            }
        }
        PalError::not_found(format!("{id} does not exist"))
    }

    /// The next free number for `kind`, counting record files that do not parse too.
    pub fn next_record_number(&self, kind: crate::records::RecordKind) -> u32 {
        let name_re = crate::util::re(r"^(rfc|adr)-(\d{4})");
        let parsed = self
            .an
            .records
            .list
            .iter()
            .filter_map(crate::records::Records::id_of)
            .filter(|i| i.kind == kind)
            .map(|i| i.number);
        let by_name = self.snap.files.iter().filter_map(|f| {
            let c = name_re.captures(&f.name)?;
            (c[1] == *kind.lower())
                .then(|| c[2].parse::<u32>().ok())
                .flatten()
        });
        parsed.chain(by_name).max().unwrap_or(0) + 1
    }

    /// The line ending new files use: the one `layout.rst` uses.
    pub fn project_eol(&self) -> crate::text::Eol {
        project_eol(&self.snap)
    }

    /// Stores new content for `path`. A file that does not exist yet gets the project's
    /// line ending; an existing file keeps the endings of the lines it already had.
    pub fn put(&mut self, path: &Path, src: &Source) -> Res<()> {
        if self.snap.file(path).is_none() {
            let converted = generated::with_eol(src.clone(), self.project_eol());
            return self.put_raw(path, &converted);
        }
        self.put_raw(path, src)
    }

    /// Stores `src` exactly as given.
    pub fn put_raw(&mut self, path: &Path, src: &Source) -> Res<()> {
        self.ov
            .write(path, src.to_bytes())
            .map_err(|e| PalError::io(&format!("reading {}", path.display()), &e))
    }

    /// Deletes `path`.
    pub fn remove(&mut self, path: &Path) -> Res<()> {
        self.ov
            .delete(path)
            .map_err(|e| PalError::io(&format!("reading {}", path.display()), &e))
    }
}

/// Runs `op` as one transaction on the initialized project `project`.
pub fn run_write<F>(ctx: &Ctx, project: &str, dry_run: bool, op: F) -> Res<WriteResult>
where
    F: FnOnce(&mut Session<'_, '_>) -> Res<Vec<String>>,
{
    run_transaction(ctx, project, dry_run, true, op)
}

/// Runs `op` as one transaction. With `need_layout`, a project without
/// `_palette/layout.rst` is refused; without it (regeneration), the families are
/// inferred from the documents, and the lock is skipped when `_palette/` does not
/// exist, since the lock lives there and nothing else writes such a checkout.
fn run_transaction<F>(
    ctx: &Ctx,
    project: &str,
    dry_run: bool,
    need_layout: bool,
    op: F,
) -> Res<WriteResult>
where
    F: FnOnce(&mut Session<'_, '_>) -> Res<Vec<String>>,
{
    let root = ctx.roots.resolve_project(project)?;
    let palette_dir = root.join(crate::layout::PALETTE_DIR);
    if need_layout && !palette_dir.join("layout.rst").is_file() {
        return Err(PalError::new(
            crate::errors::ErrCode::NoLayout,
            format!(
                "{} has no _palette/layout.rst. Run palette_init to set the project up.",
                root.display()
            ),
        ));
    }
    let _lock = if palette_dir.is_dir() {
        Some(ProjectLock::acquire(
            &palette_dir.join(".palette.lock"),
            ctx.lock_timeout,
        )?)
    } else {
        None
    };
    let disk = DiskSource;
    let mut ov = Overlay::new(&disk);
    let snap = Snapshot::load(&ov, &root)?;
    let an = Analysis::build(&snap);
    let allocated = {
        let mut sess = Session {
            ov: &mut ov,
            snap,
            an,
            today: (ctx.clock)(),
            now: (ctx.clock_now)(),
        };
        op(&mut sess)?
    };
    finish(ctx, &root, &mut ov, dry_run, allocated)
}

/// Runs `op` on a project that has no layout yet (`palette_init`).
pub fn run_init<F>(ctx: &Ctx, project: &str, dry_run: bool, op: F) -> Res<WriteResult>
where
    F: FnOnce(&mut Overlay<'_>, &Path, &str) -> Res<Vec<String>>,
{
    let root = ctx.roots.resolve_project(project)?;
    let disk = DiskSource;
    let mut ov = Overlay::new(&disk);
    let allocated = op(&mut ov, &root, &(ctx.clock)())?;
    finish(ctx, &root, &mut ov, dry_run, allocated)
}

/// The line ending of `layout.rst`, which new files follow.
pub fn project_eol(snap: &Snapshot) -> crate::text::Eol {
    snap.file(&snap.loc.layout_file())
        .and_then(|f| f.doc.as_ref())
        .map(|d| d.src.native_eol())
        .unwrap_or(crate::text::Eol::Lf)
}

/// Brings every generated file in line with what the server would generate now: writes
/// the ones that are missing or differ, and deletes staging documents no changeset
/// produces. Generated files are written only by the server, so refreshing all of them
/// (not only the ones this operation affects) also repairs drift left by hand edits.
/// Regenerates every index and staging document that is missing or differs from what the
/// server would generate (the generation every write tool runs), and nothing else.
pub fn generate(ctx: &Ctx, project: &str) -> Res<WriteResult> {
    run_transaction(ctx, project, false, false, |_| Ok(Vec::new()))
}

fn regenerate(ov: &mut Overlay<'_>, root: &Path) -> Res<()> {
    let snap = Snapshot::load(&*ov, root)?;
    let an = Analysis::build(&snap);
    let wanted = generated::generate(&snap, &an.records, &an.sets, &an.plan);
    let io = |p: &Path, e: io::Error| PalError::io(&format!("reading {}", p.display()), &e);
    for (path, src) in &wanted {
        let existing = snap.file(path).and_then(|f| f.doc.as_ref());
        if existing.is_some_and(|d| generated::same_text(&d.src, src)) {
            continue;
        }
        let eol = existing
            .map(|d| d.src.native_eol())
            .unwrap_or_else(|| project_eol(&snap));
        let out = generated::with_eol(src.clone(), eol);
        ov.write(path, out.to_bytes()).map_err(|e| io(path, e))?;
    }
    for f in &snap.files {
        if matches!(f.role, crate::docs::Role::Staging(_)) && !wanted.contains_key(&f.path) {
            ov.delete(&f.path).map_err(|e| io(&f.path, e))?;
        }
    }
    Ok(())
}

fn finish(
    ctx: &Ctx,
    root: &Path,
    ov: &mut Overlay<'_>,
    dry_run: bool,
    allocated: Vec<String>,
) -> Res<WriteResult> {
    regenerate(ov, root)?;
    let changes = ov.effective_changes();
    let touched: Vec<String> = changes.iter().map(|(p, _, _)| rel_slash(root, p)).collect();
    let snap = Snapshot::load(&*ov, root)?;
    let an = Analysis::build(&snap);
    let findings = lint::run(&*ov, &snap, &an);
    let blocking: Vec<&lint::Finding> = findings
        .iter()
        .filter(|f| f.severity == Severity::Error && touched.contains(&f.file))
        .collect();
    if !blocking.is_empty() {
        let mut msg = format!(
            "the result would contain {} lint error(s) in the files it changes, so nothing was written. Fix the operation's inputs, or fix these by hand first if they were already in the file:",
            blocking.len()
        );
        for f in blocking.iter().take(10) {
            msg.push_str(&format!(
                "\n  {} {}:{} {}",
                f.rule, f.file, f.line, f.message
            ));
        }
        if blocking.len() > 10 {
            msg.push_str(&format!(
                "\n  ... and {} more (run palette_lint)",
                blocking.len() - 10
            ));
        }
        return Err(PalError::invariant(msg));
    }
    let mut files = Vec::new();
    let mut diff = String::new();
    let mut sorted = changes.clone();
    sorted.sort_by_key(|(p, _, _)| rel_slash(root, p));
    for (p, old, new) in &sorted {
        let rel = rel_slash(root, p);
        let action = match (old, new) {
            (None, Some(_)) => "created",
            (Some(_), None) => "deleted",
            _ => "modified",
        };
        files.push((rel.clone(), action));
        diff.push_str(&vfs::unified_diff(&rel, old.as_deref(), new.as_deref()));
    }
    if !dry_run {
        let hook = ctx.commit_hook.clone();
        let mut call = |i: usize| match &hook {
            Some(h) => h(i),
            None => Ok(()),
        };
        vfs::commit(root, ov, &mut call)?;
    }
    Ok(WriteResult {
        dry_run,
        allocated,
        files,
        diff,
    })
}

/// Reads `path` through `src`, returning `None` when it is absent.
pub fn read_optional(src: &dyn FileSource, path: &Path) -> Res<Option<Vec<u8>>> {
    src.read(path)
        .map_err(|e| PalError::io(&format!("reading {}", path.display()), &e))
}
