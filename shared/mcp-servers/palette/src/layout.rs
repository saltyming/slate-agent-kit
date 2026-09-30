//! `layout.rst`: the twelve document families and where each one lives.
//!
//! Owns parsing the layout document, inferring a layout from the documents
//! themselves when the project has none, validating placements and resolving every
//! family (and the documents inside it) to an absolute path. Does not report
//! findings; `lint` turns [`LayoutProblem`]s into P012.
//! Entry points: [`Layout::parse`], [`Layout::infer`], [`Locations`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::rst::Doc;
use crate::text::Source;
use crate::util::rel_slash;
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
}

impl Family {
    /// Every family in layout order.
    pub const ALL: [Family; 12] = [
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
            Family::Backlog | Family::State | Family::Principles | Family::Glossary
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
                });
                continue;
            };
            if placements.contains_key(&fam) {
                problems.push(LayoutProblem {
                    line: f.start,
                    message: format!("family `{name}` is placed more than once"),
                    rel: None,
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
                            });
                            continue;
                        }
                        seen_paths.insert(p.clone(), fam);
                        Placement::Path(p)
                    }
                    Err(msg) => {
                        problems.push(LayoutProblem {
                            line: f.start,
                            message: format!("family `{name}`: {msg}"),
                            rel: None,
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
                });
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
    /// named `staging` whose `spec/` and `design/` subfolders hold the mirrors. A
    /// family found in two places keeps the first and reports the second as a
    /// problem; a family with no documents is internal. The checker is `none`. A
    /// template (a document whose title still carries a `<...>` placeholder) and
    /// anything under a `fixtures` folder are not project documents and are skipped.
    pub fn infer(src: &dyn FileSource, root: &Path) -> std::io::Result<Layout> {
        let mut found: BTreeMap<Family, (String, String)> = BTreeMap::new();
        let mut problems = Vec::new();
        let mut files: Vec<PathBuf> = Vec::new();
        walk(src, root, &mut files)?;
        for path in files {
            let Some(bytes) = src.read(&path)? else {
                continue;
            };
            let Ok(doc) = Doc::parse_bytes(&bytes) else {
                continue;
            };
            if is_template(&doc) {
                continue;
            }
            let Some((fam, placed)) = classify(&doc, root, &path) else {
                continue;
            };
            let rel = rel_slash(root, &path);
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
                    });
                }
                Some(_) => {}
            }
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

    /// Resolves a maintained document written as `design/<topic>.rst` or
    /// `spec/<topic>.rst` to its file in the design or spec family.
    pub fn maintained_file(&self, logical: &str) -> Option<PathBuf> {
        let (fam, name) = split_logical(logical)?;
        Some(self.dir_of(fam).join(name))
    }

    /// The staging file that mirrors a maintained document.
    pub fn staging_file(&self, logical: &str) -> Option<PathBuf> {
        let (fam, name) = split_logical(logical)?;
        Some(self.dir_of(Family::Staging).join(fam.key()).join(name))
    }

    /// Whether `path` lies inside the `_palette` folder.
    pub fn is_internal(&self, path: &Path) -> bool {
        path.starts_with(self.palette_dir())
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

fn walk(src: &dyn FileSource, dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for e in src.list(dir)? {
        let p = dir.join(&e.name);
        if e.is_dir {
            if SKIPPED_DIRS.contains(&e.name.as_str()) || e.name.starts_with('.') {
                continue;
            }
            walk(src, &p, out)?;
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
    // `<staging>/spec/x.rst` and `<staging>/design/x.rst` mirror maintained documents.
    let mirrored = matches!(fam, Family::Spec | Family::Design)
        && dir_name(parent) == fam.key()
        && parent.parent().is_some_and(|g| dir_name(g) == "staging");
    if mirrored {
        return Some((Family::Staging, up(1)?));
    }
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

    const ALL_INTERNAL: &str = ":backlog: internal\n:phase: internal\n:deliverable: internal\n:state: internal\n:rfc: internal\n:adr: internal\n:changeset: internal\n:staging: internal\n:design: internal\n:spec: internal\n:principles: internal\n:glossary: internal\n:checker: none\n";

    #[test]
    fn complete_layout_has_no_problems() {
        let l = layout(ALL_INTERNAL);
        assert!(l.problems.is_empty(), "{:?}", l.problems);
        assert_eq!(l.placements.len(), 12);
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
        assert_eq!(
            loc.maintained_file("spec/a.rst"),
            Some(Path::new("/p").join("_palette").join("spec").join("a.rst"))
        );
        assert_eq!(
            loc.staging_file("design/a.rst"),
            Some(
                Path::new("/p")
                    .join("_palette")
                    .join("staging")
                    .join("design")
                    .join("a.rst")
            )
        );
        assert_eq!(loc.maintained_file("rfc/a.rst"), None);
    }
}
