//! `layout.rst`: the thirteen document families and where each one lives.
//!
//! Owns parsing the layout document, inferring a layout from the documents
//! themselves when the project has none, validating placements and resolving every
//! family (and the documents inside it) to an absolute path, reading which kind of
//! document a record's `Changes` entry names, and the location rules of a changeset
//! target (which document a spelling names, whether a changeset may edit it, and where
//! its staging mirror lies). Does not read files and does not report findings; `lint`
//! turns [`LayoutProblem`]s into P012, and the snapshot adds the checks that need the
//! files to [`Locations::resolve_target`].
//! Entry points: [`Layout::parse`], [`Layout::infer`], [`Locations`], [`changes_doc`],
//! [`Locations::resolve_target`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::rst::Doc;
use crate::text::Source;
use crate::util::{is_kebab_rst, rel_slash, same_path_nocase, starts_with_nocase};
use crate::vfs::FileSource;

/// Folders the inference walk never enters.
const SKIPPED_DIRS: [&str; 5] = [PALETTE_DIR, ".git", "target", "node_modules", "fixtures"];

/// The name of the folder that holds internal documents.
pub const PALETTE_DIR: &str = "_palette";

/// A document family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Family {
    /// The list of items and phases.
    Backlog,
    /// A phase brief.
    Phase,
    /// One approved unit inside a phase.
    Deliverable,
    /// Decisions, questions and discrepancies.
    State,
    /// Requests for comments.
    Rfc,
    /// Architecture decision records.
    Adr,
    /// Edits to maintained documents that source does not implement yet.
    Changeset,
    /// Generated maintained documents with accepted edits applied.
    Staging,
    /// Design documents.
    Design,
    /// Specifications.
    Spec,
    /// Principles.
    Principles,
    /// Glossary.
    Glossary,
    /// How to contribute: branches, commits, pull requests, verification.
    Contributing,
}

impl Family {
    /// Every family in layout order.
    pub const ALL: [Family; 13] = [
        Family::Backlog,
        Family::Phase,
        Family::Deliverable,
        Family::State,
        Family::Rfc,
        Family::Adr,
        Family::Changeset,
        Family::Staging,
        Family::Design,
        Family::Spec,
        Family::Principles,
        Family::Glossary,
        Family::Contributing,
    ];

    /// The field name used in `layout.rst`.
    pub fn key(self) -> &'static str {
        match self {
            Family::Backlog => "backlog",
            Family::Phase => "phase",
            Family::Deliverable => "deliverable",
            Family::State => "state",
            Family::Rfc => "rfc",
            Family::Adr => "adr",
            Family::Changeset => "changeset",
            Family::Staging => "staging",
            Family::Design => "design",
            Family::Spec => "spec",
            Family::Principles => "principles",
            Family::Glossary => "glossary",
            Family::Contributing => "contributing",
        }
    }

    /// The family named `key`.
    pub fn from_key(key: &str) -> Option<Family> {
        Family::ALL.into_iter().find(|f| f.key() == key)
    }

    /// Whether the family is one file rather than a folder of files.
    pub fn is_single_file(self) -> bool {
        matches!(
            self,
            Family::Backlog
                | Family::State
                | Family::Principles
                | Family::Glossary
                | Family::Contributing
        )
    }
}

/// Where a family lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Under `_palette/`.
    Internal,
    /// A project-relative path with `/` separators, no leading or trailing slash.
    Path(String),
}

impl Placement {
    /// The text written in `layout.rst`.
    pub fn text(&self) -> String {
        match self {
            Placement::Internal => "internal".to_string(),
            Placement::Path(p) => p.clone(),
        }
    }
}

/// A placed family and the layout line that placed it.
#[derive(Clone, Debug)]
pub struct Placed {
    /// The placement.
    pub placement: Placement,
    /// 0-based line of the field.
    pub line: usize,
}

/// A problem found while reading `layout.rst` or inferring a layout; reported as P012.
#[derive(Clone, Debug)]
pub struct LayoutProblem {
    /// 0-based line (in `layout.rst`, or in the document named by `rel`).
    pub line: usize,
    /// Message.
    pub message: String,
    /// The project-relative document an inferred problem is reported on; `None` for
    /// a problem in `layout.rst`.
    pub rel: Option<String>,
    /// The family the document looked like, for an inferred problem.
    pub family: Option<Family>,
}

/// The parsed layout.
#[derive(Clone, Debug)]
pub struct Layout {
    /// Family placements found (valid ones only).
    pub placements: BTreeMap<Family, Placed>,
    /// The `checker` setting: a project command or `none`.
    pub checker: Option<(String, usize)>,
    /// Problems for P012.
    pub problems: Vec<LayoutProblem>,
}

/// Validates a project-relative placement path and returns it normalized.
pub fn normalize_path(value: &str, family: Family) -> Result<String, String> {
    let v = value.trim();
    if v.is_empty() {
        return Err("the placement is empty".to_string());
    }
    let bytes = v.as_bytes();
    let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if v.starts_with('/') || v.starts_with('\\') || drive {
        return Err(format!(
            "`{v}` is an absolute path; a placement is relative to the project"
        ));
    }
    if v.contains('\\') {
        return Err(format!("`{v}` uses a backslash; write paths with `/`"));
    }
    let mut parts: Vec<&str> = Vec::new();
    for c in v.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(format!("`{v}` leaves the project"));
                }
            }
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        return Err(format!(
            "`{v}` is the project folder itself; name a folder or file inside it"
        ));
    }
    if parts[0] == PALETTE_DIR {
        return Err(format!(
            "`{v}` is inside `{PALETTE_DIR}/`; use `internal` for internal documents"
        ));
    }
    let joined = parts.join("/");
    let is_rst = joined.ends_with(".rst");
    if family.is_single_file() && !is_rst {
        return Err(format!(
            "`{v}` must name a single `.rst` file for {}",
            family.key()
        ));
    }
    if !family.is_single_file() && is_rst {
        return Err(format!("`{v}` must name a folder for {}", family.key()));
    }
    Ok(joined)
}

impl Layout {
    /// Reads the layout document.
    pub fn parse(src: &Source) -> Layout {
        let doc = Doc::parse(src.clone());
        let mut placements: BTreeMap<Family, Placed> = BTreeMap::new();
        let mut checker = None;
        let mut problems = Vec::new();
        let mut seen_paths: BTreeMap<String, Family> = BTreeMap::new();
        for f in doc.fields(0, doc.src.len()) {
            let name = f.name.trim();
            let value = f.value.trim().trim_matches('`').trim();
            if name == "checker" {
                checker = Some((value.to_string(), f.start));
                continue;
            }
            let Some(fam) = Family::from_key(name) else {
                problems.push(LayoutProblem {
                    line: f.start,
                    message: format!(
                        "`{name}` is not a document family; families are: {}",
                        Family::ALL.map(Family::key).join(", ")
                    ),
                    rel: None,
                    family: None,
                });
                continue;
            };
            if placements.contains_key(&fam) {
                problems.push(LayoutProblem {
                    line: f.start,
                    message: format!("family `{name}` is placed more than once"),
                    rel: None,
                    family: None,
                });
                continue;
            }
            let placement = if value == "internal" {
                Placement::Internal
            } else {
                match normalize_path(value, fam) {
                    Ok(p) => {
                        let clash = seen_paths.get(&p).copied().filter(|other| {
                            !matches!(
                                (fam, *other),
                                (Family::Phase, Family::Deliverable)
                                    | (Family::Deliverable, Family::Phase)
                            )
                        });
                        if let Some(other) = clash {
                            problems.push(LayoutProblem {
                                line: f.start,
                                message: format!(
                                    "`{name}` and `{}` are both placed at `{p}`; each family needs its own location",
                                    other.key()
                                ),
                                rel: None,
                                family: None,
                            });
                            // Staging is the placement that yields: every document
                            // under it would be read, and removed, as a mirror.
                            if other != Family::Staging {
                                continue;
                            }
                            placements.remove(&Family::Staging);
                        }
                        seen_paths.insert(p.clone(), fam);
                        Placement::Path(p)
                    }
                    Err(msg) => {
                        problems.push(LayoutProblem {
                            line: f.start,
                            message: format!("family `{name}`: {msg}"),
                            rel: None,
                            family: None,
                        });
                        continue;
                    }
                }
            };
            placements.insert(
                fam,
                Placed {
                    placement,
                    line: f.start,
                },
            );
        }
        for fam in Family::ALL {
            if !placements.contains_key(&fam)
                && !problems
                    .iter()
                    .any(|p| p.message.contains(&format!("`{}`", fam.key())))
            {
                problems.push(LayoutProblem {
                    line: 0,
                    message: format!(
                        "family `{}` is missing; add `:{}: internal` or a project path",
                        fam.key(),
                        fam.key()
                    ),
                    rel: None,
                    family: None,
                });
            }
        }
        if checker.is_none() {
            problems.push(LayoutProblem {
                line: 0,
                message: "setting `checker` is missing; add `:checker: none` or the project command that validates its documents".to_string(),
                rel: None,
                family: None,
            });
        }
        // Every document under the staging folder is a generated mirror, so a family
        // placed inside it would be read, and removed, as staging. Such a staging
        // placement is refused and reads as internal.
        if let Some(Placed {
            placement: Placement::Path(st),
            line,
        }) = placements.get(&Family::Staging).cloned()
        {
            let prefix = format!("{}/", st.to_ascii_lowercase());
            let inside: Vec<(Family, String)> = placements
                .iter()
                .filter_map(|(fam, placed)| match &placed.placement {
                    Placement::Path(p)
                        if *fam != Family::Staging
                            && p.to_ascii_lowercase().starts_with(&prefix) =>
                    {
                        Some((*fam, p.clone()))
                    }
                    _ => None,
                })
                .collect();
            for (fam, p) in &inside {
                problems.push(LayoutProblem {
                    line,
                    message: format!(
                        "`staging` at `{st}` contains the placement of `{}` (`{p}`); the staging folder holds only generated mirrors, so no family is placed inside it",
                        fam.key()
                    ),
                    rel: None,
                    family: None,
                });
            }
            if !inside.is_empty() {
                placements.remove(&Family::Staging);
            }
        }
        Layout {
            placements,
            checker,
            problems,
        }
    }

    /// Infers a layout from the documents under `root` when the project has no
    /// `layout.rst`: every `.rst` file outside `_palette/`, `.git`, `target` and
    /// `node_modules` is classified by its title and `:Status:` field, and each family
    /// is placed where its documents were found. The staging family is the folder
    /// named `staging` that holds a `design` or `spec` subfolder, or that lies beside
    /// a folder where records, design or spec documents were found; nothing inside it
    /// is classified. A family found in two places keeps the first and reports the
    /// second as a problem; a family with no documents is internal. The checker is
    /// `none`. A template (a document whose title still carries a `<...>`
    /// placeholder) and anything under a `fixtures` folder are not project documents
    /// and are skipped.
    pub fn infer(src: &dyn FileSource, root: &Path) -> std::io::Result<Layout> {
        let mut found: BTreeMap<Family, (String, String)> = BTreeMap::new();
        let mut problems = Vec::new();
        let mut files: Vec<PathBuf> = Vec::new();
        let mut staging_dirs: Vec<PathBuf> = Vec::new();
        walk(src, root, &mut files, &mut staging_dirs)?;
        let under = |p: &Path, dirs: &[PathBuf]| dirs.iter().any(|d| p.starts_with(d));
        // Everything outside a folder named `staging` first: whether such a folder is
        // the staging family can depend on where the records were found.
        for path in files.iter().filter(|p| !under(p, &staging_dirs)) {
            infer_one(src, root, path, &mut found, &mut problems)?;
        }
        let beside: Vec<PathBuf> = [Family::Rfc, Family::Adr, Family::Design, Family::Spec]
            .iter()
            .filter_map(|f| found.get(f))
            .filter_map(|(placed, _)| {
                let dir = placed.split('/').fold(root.to_path_buf(), |a, c| a.join(c));
                dir.parent().map(Path::to_path_buf)
            })
            .collect();
        let mut staging: Vec<PathBuf> = Vec::new();
        for d in &staging_dirs {
            if under(d, &staging) {
                continue;
            }
            let mirrors = src.is_dir(&d.join("design")) || src.is_dir(&d.join("spec"));
            let next_to_records = d.parent().is_some_and(|p| beside.iter().any(|b| b == p));
            if mirrors || next_to_records {
                staging.push(d.clone());
            }
        }
        for path in files
            .iter()
            .filter(|p| under(p, &staging_dirs) && !under(p, &staging))
        {
            infer_one(src, root, path, &mut found, &mut problems)?;
        }
        if let Some((first, rest)) = staging.split_first() {
            let first_rel = rel_slash(root, first);
            for d in rest {
                for path in files.iter().filter(|p| p.starts_with(d)) {
                    let rel = rel_slash(root, path);
                    problems.push(LayoutProblem {
                        line: 0,
                        message: format!(
                            "{rel} lies in a second staging folder `{}`, but the staging family was already found at `{first_rel}`; with no layout.rst each family lives in one place",
                            rel_slash(root, d)
                        ),
                        rel: Some(rel.clone()),
                        family: Some(Family::Staging),
                    });
                }
            }
            found.insert(Family::Staging, (first_rel.clone(), first_rel));
        }
        let placements = Family::ALL
            .into_iter()
            .map(|f| {
                let placement = found
                    .get(&f)
                    .map(|(p, _)| Placement::Path(p.clone()))
                    .unwrap_or(Placement::Internal);
                (f, Placed { placement, line: 0 })
            })
            .collect();
        Ok(Layout {
            placements,
            checker: None,
            problems,
        })
    }

    /// The placement of `fam`; a family missing from the layout reads as internal.
    pub fn placement(&self, fam: Family) -> Placement {
        self.placements
            .get(&fam)
            .map(|p| p.placement.clone())
            .unwrap_or(Placement::Internal)
    }

    /// A copy of the layout with `fam` placed at `placement`.
    pub fn with_placement(&self, fam: Family, placement: Placement) -> Layout {
        let mut l = self.clone();
        let line = l.placements.get(&fam).map(|p| p.line).unwrap_or(0);
        l.placements.insert(fam, Placed { placement, line });
        l
    }

    /// Resolves the layout against the project root.
    pub fn locations(&self, root: &Path) -> Locations {
        Locations {
            root: root.to_path_buf(),
            placements: Family::ALL
                .into_iter()
                .map(|f| (f, self.placement(f)))
                .collect(),
        }
    }
}

/// Every family resolved to absolute paths.
#[derive(Clone, Debug)]
pub struct Locations {
    /// The canonical project root.
    pub root: PathBuf,
    placements: BTreeMap<Family, Placement>,
}

impl Locations {
    /// `<root>/_palette`.
    pub fn palette_dir(&self) -> PathBuf {
        self.root.join(PALETTE_DIR)
    }

    /// `<root>/_palette/layout.rst`.
    pub fn layout_file(&self) -> PathBuf {
        self.palette_dir().join("layout.rst")
    }

    /// `<root>/_palette/.palette.lock`.
    pub fn lock_file(&self) -> PathBuf {
        self.palette_dir().join(".palette.lock")
    }

    /// The placement of `fam`.
    pub fn placement(&self, fam: Family) -> &Placement {
        &self.placements[&fam]
    }

    fn base(&self, fam: Family) -> PathBuf {
        match &self.placements[&fam] {
            Placement::Internal => self.palette_dir().join(fam.key()),
            Placement::Path(p) => p.split('/').fold(self.root.clone(), |a, c| a.join(c)),
        }
    }

    /// The file of a single-file family (backlog, state, principles, glossary).
    pub fn file_of(&self, fam: Family) -> PathBuf {
        match &self.placements[&fam] {
            Placement::Internal => self.palette_dir().join(format!("{}.rst", fam.key())),
            Placement::Path(_) => self.base(fam),
        }
    }

    /// The folder of a folder family (rfc, adr, changeset, design, spec, staging).
    pub fn dir_of(&self, fam: Family) -> PathBuf {
        self.base(fam)
    }

    /// The folder that holds the `phase-<N>` folders of phase files.
    pub fn phase_root(&self) -> PathBuf {
        match &self.placements[&Family::Phase] {
            Placement::Internal => self.palette_dir(),
            Placement::Path(_) => self.base(Family::Phase),
        }
    }

    /// The folder that holds the `phase-<N>` folders of deliverables.
    pub fn deliverable_root(&self) -> PathBuf {
        match &self.placements[&Family::Deliverable] {
            Placement::Internal => self.palette_dir(),
            Placement::Path(_) => self.base(Family::Deliverable),
        }
    }

    /// `<phase root>/phase-<n>/phase.rst`.
    pub fn phase_file(&self, n: u32) -> PathBuf {
        self.phase_root()
            .join(format!("phase-{n}"))
            .join("phase.rst")
    }

    /// `<deliverable root>/phase-<n>/deliverables`.
    pub fn deliverables_dir(&self, phase: u32) -> PathBuf {
        self.deliverable_root()
            .join(format!("phase-{phase}"))
            .join("deliverables")
    }

    /// The index file of a record or changeset folder.
    pub fn index_file(&self, fam: Family) -> PathBuf {
        self.dir_of(fam).join("index.rst")
    }

    /// Whether `path` lies inside the `_palette` folder.
    pub fn is_internal(&self, path: &Path) -> bool {
        path.starts_with(self.palette_dir())
    }

    /// The folder whose documents a changeset may edit besides the family documents:
    /// the parent of the staging folder (`_palette/` when staging is internal).
    pub fn document_root(&self) -> PathBuf {
        let staging = self.dir_of(Family::Staging);
        staging
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.root.clone())
    }

    /// The file a changeset target edits.
    pub fn target_file(&self, t: &Target) -> PathBuf {
        match t {
            Target::Family(fam, _) if fam.is_single_file() => self.file_of(*fam),
            Target::Family(fam, name) => self.dir_of(*fam).join(name),
            Target::Document(parts) => parts.iter().fold(self.root.clone(), |a, c| a.join(c)),
        }
    }

    /// The staging document that mirrors a changeset target: `<staging>/<family
    /// key>/<name>` for a design or spec document, `<staging>/<family key>.rst` for a
    /// single-file family, and `<staging>/<path relative to the document root>` for
    /// any other document.
    pub fn mirror_file(&self, t: &Target) -> PathBuf {
        let staging = self.dir_of(Family::Staging);
        match t {
            Target::Family(fam, _) if fam.is_single_file() => {
                staging.join(format!("{}.rst", fam.key()))
            }
            Target::Family(fam, name) => staging.join(fam.key()).join(name),
            Target::Document(_) => self
                .root_relative(&self.target_file(t))
                .iter()
                .fold(staging, |a, c| a.join(c)),
        }
    }

    /// The components of `path` below the document root; `path` lies inside it (ASCII
    /// case ignored).
    fn root_relative(&self, path: &Path) -> Vec<String> {
        let skip = self.document_root().components().count();
        path.components()
            .skip(skip)
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect()
    }

    /// Reads the spelling of a changeset target (one of the three forms of a `Changes`
    /// entry) and applies the location rules: a family spelling names that family's
    /// document wherever the family is placed; a project-relative spelling must name an
    /// `.rst` document under the document root, outside `_palette/`, the staging folder
    /// and every family folder, whose mirror does not take a family mirror's place.
    /// Does not look at the file system: the snapshot adds that the document exists
    /// and is no palette document ([`crate::docs::Snapshot::target`]). The error says
    /// which rule the spelling fails.
    pub fn resolve_target(&self, spelling: &str) -> Result<Target, String> {
        let refuse = |why: String| format!("`{spelling}` cannot be edited by a changeset: {why}");
        if let Some((fam, name)) = split_logical(spelling) {
            if !is_kebab_rst(name) {
                return Err(refuse(format!(
                    "a {} document is named in lowercase kebab-case ending in `.rst`; {CHANGESET_TARGETS}",
                    fam.key()
                )));
            }
            return Ok(Target::Family(fam, name.to_string()));
        }
        for fam in [Family::Principles, Family::Glossary, Family::Contributing] {
            if spelling.strip_suffix(".rst") == Some(fam.key()) {
                return Ok(Target::Family(fam, spelling.to_string()));
            }
        }
        let parts = project_parts(spelling).map_err(refuse)?;
        let name = parts
            .last()
            .copied()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if name.ends_with(".md") {
            return Err(refuse(
                "`.md` documents are named in a record's Changes and edited directly when the change lands; a changeset edits `.rst` documents".to_string(),
            ));
        }
        if !name.ends_with(".rst") {
            return Err(refuse(format!(
                "it is not an `.rst` document; {CHANGESET_TARGETS}"
            )));
        }
        let path = parts.iter().fold(self.root.clone(), |a, c| a.join(c));
        let rel = |p: &Path| match rel_slash(&self.root, p) {
            r if r.is_empty() => "the project folder".to_string(),
            r => format!("`{r}`"),
        };
        let frozen = "records, indexes, changesets, staging and work documents are generated or frozen and are never changeset targets";
        if starts_with_nocase(&path, &self.palette_dir()) {
            return Err(refuse(format!(
                "it is inside `{PALETTE_DIR}/`, which holds work documents; {frozen}"
            )));
        }
        let doc_root = self.document_root();
        if !starts_with_nocase(&path, &doc_root) {
            return Err(refuse(format!(
                "it lies outside the document root {} (the staging folder's parent), so the project's documents do not maintain it",
                rel(&doc_root)
            )));
        }
        for fam in [
            Family::Staging,
            Family::Design,
            Family::Spec,
            Family::Rfc,
            Family::Adr,
            Family::Changeset,
            Family::Phase,
            Family::Deliverable,
        ] {
            if *self.placement(fam) == Placement::Internal {
                continue;
            }
            let dir = match fam {
                Family::Phase => self.phase_root(),
                Family::Deliverable => self.deliverable_root(),
                _ => self.dir_of(fam),
            };
            if !starts_with_nocase(&path, &dir) {
                continue;
            }
            let file = parts.last().copied().unwrap_or_default();
            return Err(refuse(match fam {
                Family::Design | Family::Spec
                    if path.components().count() == dir.components().count() + 1 =>
                {
                    format!(
                        "it is a {key} document written by its real path; write its logical name `{key}/{file}`",
                        key = fam.key()
                    )
                }
                Family::Design | Family::Spec => format!(
                    "it lies inside the {key} folder {}, where a changeset edits only the documents named `{key}/<topic>.rst`",
                    rel(&dir),
                    key = fam.key()
                ),
                Family::Staging => {
                    format!("it is a staging document in {}; {frozen}", rel(&dir))
                }
                Family::Rfc | Family::Adr => {
                    format!("it is a record or index in {}; {frozen}", rel(&dir))
                }
                Family::Changeset => {
                    format!("it is a changeset or index in {}; {frozen}", rel(&dir))
                }
                _ => format!("it is a work document in {}; {frozen}", rel(&dir)),
            }));
        }
        for fam in [
            Family::Principles,
            Family::Glossary,
            Family::Contributing,
            Family::Backlog,
            Family::State,
        ] {
            if *self.placement(fam) == Placement::Internal
                || !same_path_nocase(&path, &self.file_of(fam))
            {
                continue;
            }
            return Err(refuse(match fam {
                Family::Backlog | Family::State => {
                    format!("it is the {} work document; {frozen}", fam.key())
                }
                _ => format!(
                    "it is the {key} document written by its real path; write its logical name `{key}.rst`",
                    key = fam.key()
                ),
            }));
        }
        let mirror = self.root_relative(&path);
        let staging = rel(&self.dir_of(Family::Staging));
        if let [first, _, ..] = mirror.as_slice()
            && let Some(fam) = [Family::Design, Family::Spec]
                .into_iter()
                .find(|f| first.eq_ignore_ascii_case(f.key()))
        {
            return Err(refuse(format!(
                "its staging mirror would lie in the `{key}` folder of {staging}, which mirrors the {key} family",
                key = fam.key()
            )));
        }
        if let [only] = mirror.as_slice()
            && let Some(fam) = [Family::Principles, Family::Glossary, Family::Contributing]
                .into_iter()
                .find(|f| only.eq_ignore_ascii_case(&format!("{}.rst", f.key())))
        {
            return Err(refuse(format!(
                "its staging mirror would be `{only}` in {staging}, the mirror of the {} document",
                fam.key()
            )));
        }
        Ok(Target::Document(
            parts.into_iter().map(str::to_string).collect(),
        ))
    }
}

/// A document a changeset edits, resolved from its spelling by
/// [`Locations::resolve_target`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A maintained family document: design or spec with its file name, or the
    /// principles, glossary or contributing document with its logical name.
    Family(Family, String),
    /// Another `.rst` document under the document root: the components of its
    /// project-relative path.
    Document(Vec<String>),
}

impl Target {
    /// Whether a `Create:` edit may make this document: design and spec documents only.
    pub fn can_create(&self) -> bool {
        matches!(self, Target::Family(Family::Design | Family::Spec, _))
    }
}

/// Splits `design/<topic>.rst` or `spec/<topic>.rst` into its family and file name.
pub fn split_logical(logical: &str) -> Option<(Family, &str)> {
    let (head, name) = logical.split_once('/')?;
    let fam = match head {
        "design" => Family::Design,
        "spec" => Family::Spec,
        _ => return None,
    };
    if name.contains('/') || name.is_empty() {
        return None;
    }
    Some((fam, name))
}

/// What a changeset may edit, for messages.
pub const CHANGESET_TARGETS: &str = "a changeset edits a design or spec document (`design/<topic>.rst` or `spec/<topic>.rst`, lowercase kebab-case), the principles, glossary or contributing document (`principles.rst`, `glossary.rst`, `contributing.rst`), or another existing `.rst` document under the document root (the staging folder's parent) by its project-relative path, outside `_palette/`, the staging folder and every family folder; `.md` documents are named in the record's Changes and edited directly, and records, indexes, changesets, staging and work documents are never changeset targets";

/// Why a `Create:` edit is refused outside design and spec, for messages.
pub const CREATE_TARGETS: &str =
    "`Create:` makes design and spec documents only; edit an existing document by its sections";

/// The forms the document of a `Changes` entry takes, for messages.
pub const CHANGES_FORMS: &str = "`design/<topic>.rst` or `spec/<topic>.rst` (sections in parentheses, or `created`), `principles.rst`, `glossary.rst` or `contributing.rst` (sections in parentheses), or the project-relative path of another project document ending in `.md` or `.rst` (what changes in parentheses)";

/// The document a `Changes` entry names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangesDoc {
    /// A design or spec document by its logical path.
    Maintained(Family),
    /// The principles, glossary or contributing document by its logical name.
    Single(Family),
    /// Another project document: the components of its project-relative path.
    Project(Vec<String>),
}

/// Reads the document part of a `Changes` entry by its form alone, without the file
/// system; the error says why the form is not accepted.
pub fn changes_doc(doc: &str) -> Result<ChangesDoc, String> {
    if let Some((fam, _)) = split_logical(doc) {
        return Ok(ChangesDoc::Maintained(fam));
    }
    for fam in [Family::Principles, Family::Glossary, Family::Contributing] {
        if doc.strip_suffix(".rst") == Some(fam.key()) {
            return Ok(ChangesDoc::Single(fam));
        }
    }
    let parts = project_parts(doc)?;
    let name = parts
        .last()
        .copied()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !(name.ends_with(".md") || name.ends_with(".rst")) {
        return Err(format!(
            "`{doc}` is not a document: Changes lists documents, not source files. Accepted forms: {CHANGES_FORMS}"
        ));
    }
    Ok(ChangesDoc::Project(
        parts.into_iter().map(str::to_string).collect(),
    ))
}

/// The components of a project-relative path written with `/`; the error says why the
/// spelling is not one.
fn project_parts(doc: &str) -> Result<Vec<&str>, String> {
    if doc.contains('\\') {
        return Err(format!(
            "`{doc}` uses a backslash; write a project-relative path with `/`"
        ));
    }
    let drive = doc.as_bytes().get(1) == Some(&b':');
    if doc.starts_with('/') || drive {
        return Err(format!(
            "`{doc}` is an absolute path; write the path relative to the project folder"
        ));
    }
    let parts: Vec<&str> = doc.split('/').collect();
    if parts
        .iter()
        .any(|c| c.is_empty() || *c == "." || *c == "..")
    {
        return Err(format!(
            "`{doc}` must stay inside the project: a project-relative path without `.`, `..` or empty components"
        ));
    }
    Ok(parts)
}

/// Classifies one document for [`Layout::infer`] and records its family's placement,
/// or a problem when the family was already found elsewhere.
fn infer_one(
    src: &dyn FileSource,
    root: &Path,
    path: &Path,
    found: &mut BTreeMap<Family, (String, String)>,
    problems: &mut Vec<LayoutProblem>,
) -> std::io::Result<()> {
    let Some(bytes) = src.read(path)? else {
        return Ok(());
    };
    let Ok(doc) = Doc::parse_bytes(&bytes) else {
        return Ok(());
    };
    if is_template(&doc) {
        return Ok(());
    }
    let Some((fam, placed)) = classify(&doc, root, path) else {
        return Ok(());
    };
    let rel = rel_slash(root, path);
    match found.get(&fam) {
        None => {
            found.insert(fam, (placed, rel));
        }
        Some((first, first_rel)) if *first != placed => {
            problems.push(LayoutProblem {
                line: 0,
                message: format!(
                    "{} looks like a {} document, but that family was already found at `{first}` (`{first_rel}`); with no layout.rst each family lives in one place",
                    rel,
                    fam.key()
                ),
                rel: Some(rel.clone()),
                family: Some(fam),
            });
        }
        Some(_) => {}
    }
    Ok(())
}

/// Collects the `.rst` files under `dir` into `out` and the folders named `staging`
/// into `staging`, both in walk order.
fn walk(
    src: &dyn FileSource,
    dir: &Path,
    out: &mut Vec<PathBuf>,
    staging: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    for e in src.list(dir)? {
        let p = dir.join(&e.name);
        if e.is_dir {
            // A linked folder may lead outside the project or into an excluded tree.
            let linked = std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink());
            if SKIPPED_DIRS.contains(&e.name.as_str()) || e.name.starts_with('.') || linked {
                continue;
            }
            if e.name == Family::Staging.key() {
                staging.push(p.clone());
            }
            walk(src, &p, out, staging)?;
        } else if e.name.ends_with(".rst") {
            out.push(p);
        }
    }
    Ok(())
}

/// Whether `doc` is a template rather than a document: its title carries a placeholder.
fn is_template(doc: &Doc) -> bool {
    doc.title()
        .is_some_and(|h| h.title.contains('<') && h.title.contains('>'))
}

fn record_number(title: &str, prefix: &str) -> bool {
    title
        .strip_prefix(prefix)
        .and_then(|rest| rest.split_once(':'))
        .is_some_and(|(n, _)| n.len() == 4 && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The family of `doc` and the project-relative placement it implies, when the
/// document's title and status identify one.
fn classify(doc: &Doc, root: &Path, path: &Path) -> Option<(Family, String)> {
    let title = doc.title()?.title.clone();
    let status = doc
        .fields(0, doc.src.len())
        .into_iter()
        .find(|f| f.name.trim() == "Status")
        .map(|f| f.value.trim().to_string());
    let parent = path.parent()?;
    let dir_name = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let up = |n: usize| {
        let mut p = parent;
        for _ in 0..n {
            p = p.parent()?;
        }
        Some(rel_slash(root, p))
    };
    let file = rel_slash(root, path);
    let fam = if record_number(&title, "RFC-") {
        Family::Rfc
    } else if record_number(&title, "ADR-") {
        Family::Adr
    } else if title.starts_with("Changeset: ") {
        Family::Changeset
    } else if title.starts_with("Glossary — ") {
        return Some((Family::Glossary, file));
    } else if title.starts_with("Principles — ") {
        return Some((Family::Principles, file));
    } else if title.starts_with("Contributing — ") {
        return Some((Family::Contributing, file));
    } else if title.starts_with("Backlog — ") {
        return Some((Family::Backlog, file));
    } else if title.starts_with("State — ") {
        return Some((Family::State, file));
    } else if title.starts_with("Phase ") && dir_name(parent).starts_with("phase-") {
        return Some((Family::Phase, up(1)?));
    } else if title.starts_with("Deliverable ") && dir_name(parent) == "deliverables" {
        return Some((Family::Deliverable, up(2)?));
    } else if status.as_deref() == Some("Contract") {
        Family::Spec
    } else if status.as_deref() == Some("Maintained") && title.ends_with(" design") {
        Family::Design
    } else {
        return None;
    };
    Some((fam, up(0)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(fields: &str) -> Layout {
        Layout::parse(&Source::from_text(&format!(
            "Layout — X\n==========\n\nFamilies\n--------\n\n{fields}"
        )))
    }

    const ALL_INTERNAL: &str = ":backlog: internal\n:phase: internal\n:deliverable: internal\n:state: internal\n:rfc: internal\n:adr: internal\n:changeset: internal\n:staging: internal\n:design: internal\n:spec: internal\n:principles: internal\n:glossary: internal\n:contributing: internal\n:checker: none\n";

    #[test]
    fn complete_layout_has_no_problems() {
        let l = layout(ALL_INTERNAL);
        assert!(l.problems.is_empty(), "{:?}", l.problems);
        assert_eq!(l.placements.len(), 13);
        assert_eq!(l.checker.as_ref().map(|c| c.0.as_str()), Some("none"));
    }

    #[test]
    fn unknown_missing_and_bad_paths_are_problems() {
        let l = layout(
            ":backlog: internal\n:deliverables: internal\n:rfc: ../outside\n:adr: /abs\n:state: _palette/x.rst\n:design: docs/x\n:spec: docs/x\n",
        );
        let text: Vec<&str> = l.problems.iter().map(|p| p.message.as_str()).collect();
        assert!(
            text.iter()
                .any(|m| m.contains("`deliverables` is not a document family"))
        );
        assert!(text.iter().any(|m| m.contains("leaves the project")));
        assert!(text.iter().any(|m| m.contains("absolute")));
        assert!(text.iter().any(|m| m.contains("inside `_palette/`")));
        assert!(text.iter().any(|m| m.contains("both placed at")));
        assert!(
            text.iter()
                .any(|m| m.contains("family `glossary` is missing"))
        );
    }

    #[test]
    fn locations_resolve_internal_and_project_paths() {
        let l = layout(
            &ALL_INTERNAL
                .replace(":rfc: internal", ":rfc: docs/rfc")
                .replace(":glossary: internal", ":glossary: docs/glossary.rst"),
        );
        let loc = l.locations(Path::new("/p"));
        assert_eq!(
            loc.dir_of(Family::Rfc),
            Path::new("/p").join("docs").join("rfc")
        );
        assert_eq!(
            loc.file_of(Family::Backlog),
            Path::new("/p").join("_palette").join("backlog.rst")
        );
        assert_eq!(
            loc.file_of(Family::Glossary),
            Path::new("/p").join("docs").join("glossary.rst")
        );
        let spec = loc.resolve_target("spec/a.rst").expect("spec target");
        assert_eq!(
            loc.target_file(&spec),
            Path::new("/p").join("_palette").join("spec").join("a.rst")
        );
        let design = loc.resolve_target("design/a.rst").expect("design target");
        assert_eq!(
            loc.mirror_file(&design),
            Path::new("/p")
                .join("_palette")
                .join("staging")
                .join("design")
                .join("a.rst")
        );
        // Staging is internal, so the document root is `_palette/` and no project
        // document is a target.
        assert!(loc.resolve_target("rfc/a.rst").is_err());
    }
    /// The fixture project's placements: every maintained and record family under
    /// `docs/`, the work families internal.
    const DOCS: &str = ":backlog: internal\n:phase: internal\n:deliverable: internal\n:state: internal\n:rfc: docs/rfc\n:adr: docs/adr\n:changeset: docs/changeset\n:staging: docs/staging\n:design: docs/design\n:spec: docs/spec\n:principles: docs/principles.rst\n:glossary: docs/glossary.rst\n:contributing: docs/contributing.rst\n:checker: none\n";

    fn comps(p: &Path) -> Vec<String> {
        p.components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn document_root_is_the_staging_folders_parent() {
        let root = Path::new("/p");
        let loc = layout(DOCS).locations(root);
        assert_eq!(loc.document_root(), root.join("docs"));
        let internal = layout(ALL_INTERNAL).locations(root);
        assert_eq!(internal.document_root(), root.join("_palette"));
        let top =
            layout(&DOCS.replace(":staging: docs/staging", ":staging: staging")).locations(root);
        assert_eq!(top.document_root(), root.to_path_buf());
    }

    #[test]
    fn family_targets_resolve_wherever_the_family_is_placed() {
        let root = Path::new("/p");
        for fields in [DOCS.to_string(), ALL_INTERNAL.to_string()] {
            let loc = layout(&fields).locations(root);
            for (spelling, fam) in [
                ("principles.rst", Family::Principles),
                ("glossary.rst", Family::Glossary),
                ("contributing.rst", Family::Contributing),
            ] {
                let t = loc.resolve_target(spelling).expect(spelling);
                assert_eq!(t, Target::Family(fam, spelling.to_string()));
                assert_eq!(loc.target_file(&t), loc.file_of(fam));
                assert_eq!(
                    loc.mirror_file(&t),
                    loc.dir_of(Family::Staging).join(spelling)
                );
                assert!(!t.can_create());
            }
            let t = loc.resolve_target("design/a-b.rst").expect("design");
            assert!(t.can_create());
            assert_eq!(
                loc.target_file(&t),
                loc.dir_of(Family::Design).join("a-b.rst")
            );
            assert_eq!(
                loc.mirror_file(&t),
                loc.dir_of(Family::Staging).join("design").join("a-b.rst")
            );
            assert!(
                loc.resolve_target("design/Not_Kebab.rst")
                    .is_err_and(|m| m.contains("kebab-case"))
            );
        }
    }

    #[test]
    fn a_document_under_the_root_mirrors_at_its_relative_path() {
        let root = Path::new("/p");
        let loc = layout(DOCS).locations(root);
        let t = loc
            .resolve_target("docs/guide/coding-style.rst")
            .expect("doc");
        assert_eq!(
            t,
            Target::Document(vec![
                "docs".to_string(),
                "guide".to_string(),
                "coding-style.rst".to_string()
            ])
        );
        assert!(!t.can_create());
        assert_eq!(
            comps(&loc.target_file(&t)),
            comps(&root.join("docs").join("guide").join("coding-style.rst"))
        );
        assert_eq!(
            comps(&loc.mirror_file(&t)),
            comps(
                &root
                    .join("docs")
                    .join("staging")
                    .join("guide")
                    .join("coding-style.rst")
            )
        );
    }

    #[test]
    fn refused_targets_name_the_rule() {
        let root = Path::new("/p");
        let loc = layout(DOCS).locations(root);
        for (spelling, needle) in [
            (
                "docs/notes.md",
                "`.md` documents are named in a record's Changes",
            ),
            ("README.md", "`.md` documents"),
            ("docs/rfc/rfc-0001-a.rst", "is a record or index"),
            ("docs/rfc/index.rst", "is a record or index"),
            ("docs/adr/adr-0001-a.rst", "is a record or index"),
            ("docs/changeset/rfc-0001.rst", "is a changeset or index"),
            ("docs/staging/spec/a.rst", "is a staging document"),
            ("docs/staging/guide.rst", "is a staging document"),
            ("_palette/backlog.rst", "inside `_palette/`"),
            ("_palette/phase-1/phase.rst", "inside `_palette/`"),
            ("guide.rst", "outside the document root `docs`"),
            ("src/doc/x.rst", "outside the document root"),
            ("docs/spec/a.rst", "write its logical name `spec/a.rst`"),
            ("docs/design/a.rst", "write its logical name `design/a.rst`"),
            ("docs/design/old/a.rst", "inside the design folder"),
            (
                "docs/principles.rst",
                "write its logical name `principles.rst`",
            ),
            ("docs/main.rs", "not an `.rst` document"),
            ("docs\\x.rst", "backslash"),
            ("/etc/x.rst", "absolute path"),
            ("docs/../x.rst", "must stay inside the project"),
        ] {
            let err = loc.resolve_target(spelling).expect_err(spelling);
            assert!(
                err.starts_with(&format!("`{spelling}` cannot be edited by a changeset"))
                    && err.contains(needle),
                "{spelling}: {err}"
            );
        }
        // Work documents placed at a project path are refused like internal ones.
        let worked = layout(
            &DOCS
                .replace(":backlog: internal", ":backlog: docs/backlog.rst")
                .replace(":phase: internal", ":phase: docs/plan"),
        )
        .locations(root);
        assert!(
            worked
                .resolve_target("docs/backlog.rst")
                .is_err_and(|m| m.contains("backlog work document"))
        );
        assert!(
            worked
                .resolve_target("docs/plan/phase-1/phase.rst")
                .is_err_and(|m| m.contains("work document"))
        );
        // With staging internal the document root is `_palette/`: no project document
        // is a target.
        let internal =
            layout(&DOCS.replace(":staging: docs/staging", ":staging: internal")).locations(root);
        assert!(
            internal
                .resolve_target("docs/guide.rst")
                .is_err_and(|m| m.contains("outside the document root `_palette`"))
        );
    }

    #[test]
    fn a_mirror_that_would_take_a_family_mirrors_place_is_refused() {
        let root = Path::new("/p");
        // The design family lives elsewhere, so `docs/design/` is an ordinary folder,
        // but its mirror would be the design mirror folder.
        let loc = layout(&DOCS.replace(":design: docs/design", ":design: docs/architecture"))
            .locations(root);
        assert!(
            loc.resolve_target("docs/design/x.rst")
                .is_err_and(|m| m.contains("mirrors the design family"))
        );
        let loc = layout(&DOCS.replace(":glossary: docs/glossary.rst", ":glossary: internal"))
            .locations(root);
        assert!(
            loc.resolve_target("docs/glossary.rst")
                .is_err_and(|m| m.contains("the mirror of the glossary document"))
        );
    }

    #[test]
    fn a_staging_placement_that_holds_a_family_is_refused() {
        let l = layout(&DOCS.replace(":staging: docs/staging", ":staging: docs"));
        let msgs: Vec<&str> = l.problems.iter().map(|p| p.message.as_str()).collect();
        for fam in ["rfc", "design", "spec", "principles"] {
            assert!(
                msgs.iter().any(
                    |m| m.starts_with("`staging` at `docs` contains the placement of")
                        && m.contains(&format!("`{fam}`"))
                ),
                "{msgs:?}"
            );
        }
        assert_eq!(l.placement(Family::Staging), Placement::Internal);
        let ok = layout(DOCS);
        assert!(ok.problems.is_empty(), "{:?}", ok.problems);
    }
}
