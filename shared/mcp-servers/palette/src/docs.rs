//! A project snapshot: every palette document found through a [`FileSource`], with its
//! role, parsed once.
//!
//! Owns discovery of documents from the layout (which file belongs to which family),
//! or from the documents themselves when the project has no layout, and loading them;
//! the documents outside every family that a changeset names, held apart from the
//! palette documents so that no document rule reads them; and resolving a changeset
//! target against both. Does not interpret document content beyond the RST scan; the
//! models in `records`, `backlog`, `state` and `changeset` do that.
//! Entry points: [`Snapshot::load`], [`Snapshot::target`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::errors::{PalError, Res};
#[cfg(test)]
use crate::layout::Placement;
use crate::layout::{Family, Layout, Locations, Target};
use crate::rst::Doc;
use crate::text::Source;
use crate::util::{rel_slash, same_path_nocase};
use crate::vfs::FileSource;

/// What a document is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// `_palette/layout.rst`.
    Layout,
    /// The backlog.
    Backlog,
    /// The state document.
    State,
    /// A phase file; the number comes from its folder name.
    Phase(Option<u32>),
    /// A deliverable; the number is its phase's.
    Deliverable(Option<u32>),
    /// An RFC file.
    Rfc,
    /// An ADR file.
    Adr,
    /// A changeset file.
    Changeset,
    /// A maintained design document.
    Design,
    /// A maintained specification.
    Spec,
    /// The principles document.
    Principles,
    /// The glossary.
    Glossary,
    /// The contributing document.
    Contributing,
    /// A generated staging document: any `.rst` under the staging folder, with the
    /// family whose document it mirrors (`None` for the mirror of a document outside
    /// every family, or a file at no mirror path).
    Staging(Option<Family>),
    /// A generated `index.rst` of a record or changeset folder.
    Index(Family),
}

impl Role {
    /// The template name whose schema applies, if any.
    pub fn template(self) -> Option<&'static str> {
        Some(match self {
            Role::Layout => "layout",
            Role::Backlog => "backlog",
            Role::State => "state",
            Role::Phase(_) => "phase",
            Role::Deliverable(_) => "deliverable",
            Role::Rfc => "rfc",
            Role::Adr => "adr",
            Role::Design => "design",
            Role::Spec => "spec",
            Role::Principles => "principles",
            Role::Glossary => "glossary",
            Role::Contributing => "contributing",
            Role::Changeset | Role::Staging(_) | Role::Index(_) => return None,
        })
    }

    /// The role a document of `fam` has when its phase number is unknown.
    pub fn of_family(fam: Family) -> Role {
        match fam {
            Family::Backlog => Role::Backlog,
            Family::Phase => Role::Phase(None),
            Family::Deliverable => Role::Deliverable(None),
            Family::State => Role::State,
            Family::Rfc => Role::Rfc,
            Family::Adr => Role::Adr,
            Family::Changeset => Role::Changeset,
            Family::Staging => Role::Staging(None),
            Family::Design => Role::Design,
            Family::Spec => Role::Spec,
            Family::Principles => Role::Principles,
            Family::Glossary => Role::Glossary,
            Family::Contributing => Role::Contributing,
        }
    }

    /// Whether the document is a work document (backlog, phase, deliverable, state).
    pub fn is_work(self) -> bool {
        matches!(
            self,
            Role::Backlog | Role::State | Role::Phase(_) | Role::Deliverable(_)
        )
    }
}

/// One document file.
pub struct DocFile {
    /// Absolute path.
    pub path: PathBuf,
    /// Project-relative path with `/` separators.
    pub rel: String,
    /// File name.
    pub name: String,
    /// Role.
    pub role: Role,
    /// Parsed content; `None` when the bytes are not UTF-8.
    pub doc: Option<Doc>,
    /// 1-based line of the first invalid UTF-8 byte.
    pub bad_utf8: Option<usize>,
}

/// All palette documents of a project.
pub struct Snapshot {
    /// Canonical project root.
    pub root: PathBuf,
    /// The parsed layout, or the one inferred from the documents.
    pub layout: Layout,
    /// Whether the layout was inferred because `_palette/layout.rst` is absent.
    pub inferred: bool,
    /// Family locations.
    pub loc: Locations,
    /// Documents in discovery order.
    pub files: Vec<DocFile>,
    by_path: BTreeMap<PathBuf, usize>,
    /// Documents outside every family that a changeset names, by path. Not palette
    /// documents.
    referenced: BTreeMap<PathBuf, Referenced>,
}

/// A document outside every family as a changeset spelling found it.
enum Referenced {
    /// Its lines.
    Text(Source),
    /// The 1-based line of the first invalid UTF-8 byte.
    BadUtf8(usize),
    /// The spelling differs in case from the folder entries on disk, which are given
    /// here; on a case-insensitive file system both would name one file.
    Spelled(String),
}

fn io_err(action: &str, e: &std::io::Error) -> PalError {
    PalError::io(action, e)
}

impl Snapshot {
    /// Reads the layout and every document it places. Without a layout, the families
    /// are placed where their documents are found ([`Layout::infer`]); the internal
    /// families are then empty, which is the shape a checkout without `_palette/` has.
    pub fn load(src: &dyn FileSource, root: &Path) -> Res<Snapshot> {
        let layout_path = root.join(crate::layout::PALETTE_DIR).join("layout.rst");
        let layout_bytes = src
            .read(&layout_path)
            .map_err(|e| io_err("reading layout.rst", &e))?;
        let (layout, inferred) = match layout_bytes {
            Some(bytes) => {
                let layout_src = Source::from_bytes(&bytes).map_err(|line| {
                    PalError::parse("_palette/layout.rst", line, "the file is not valid UTF-8")
                })?;
                (Layout::parse(&layout_src), false)
            }
            None => (
                Layout::infer(src, root).map_err(|e| io_err("scanning for documents", &e))?,
                true,
            ),
        };
        let loc = layout.locations(root);
        let mut snap = Snapshot {
            root: root.to_path_buf(),
            layout,
            inferred,
            loc,
            files: Vec::new(),
            by_path: BTreeMap::new(),
            referenced: BTreeMap::new(),
        };
        if !inferred {
            snap.add(src, layout_path, Role::Layout)?;
        }
        snap.discover(src)?;
        // A document found at a second location for its family is loaded too, so the
        // conflict is reported (P012) and the document is checked.
        let extra: Vec<(PathBuf, Role)> = snap
            .layout
            .problems
            .iter()
            .filter_map(|p| {
                let rel = p.rel.as_ref()?;
                let path = rel.split('/').fold(root.to_path_buf(), |a, c| a.join(c));
                Some((path, Role::of_family(p.family?)))
            })
            .collect();
        for (path, role) in extra {
            snap.add(src, path, role)?;
        }
        let named: Vec<String> = snap
            .files
            .iter()
            .filter(|f| f.role == Role::Changeset)
            .filter_map(|f| f.doc.as_ref())
            .flat_map(|d| d.headings_at(2).map(|(_, h)| h.title.clone()))
            .collect();
        for spelling in named {
            snap.reference(src, &spelling)?;
        }
        Ok(snap)
    }

    /// Reads the document a changeset target names when it lies outside every family,
    /// so that [`Snapshot::target`] can resolve it. Any other spelling reads nothing.
    pub fn reference(&mut self, src: &dyn FileSource, spelling: &str) -> Res<()> {
        let Ok(t) = self.loc.resolve_target(spelling) else {
            return Ok(());
        };
        let Target::Document(parts) = &t else {
            return Ok(());
        };
        let path = self.loc.target_file(&t);
        if self.referenced.contains_key(&path) {
            return Ok(());
        }
        // One file has one spelling: every component must match its folder entry
        // exactly, so that two spellings never stage or promote one file twice.
        let mut dir = self.root.clone();
        let mut on_disk: Vec<String> = Vec::new();
        let mut exact = true;
        for part in parts {
            let entries = src
                .list(&dir)
                .map_err(|e| io_err(&format!("listing {}", dir.display()), &e))?;
            let entry = entries
                .iter()
                .find(|e| e.name == *part)
                .or_else(|| entries.iter().find(|e| e.name.eq_ignore_ascii_case(part)));
            let Some(entry) = entry else {
                return Ok(());
            };
            exact &= entry.name == *part;
            on_disk.push(entry.name.clone());
            dir = dir.join(&entry.name);
        }
        if !exact {
            self.referenced
                .insert(path, Referenced::Spelled(on_disk.join("/")));
            return Ok(());
        }
        let bytes = src
            .read(&path)
            .map_err(|e| io_err(&format!("reading {}", path.display()), &e))?;
        if let Some(bytes) = bytes {
            let r = match Source::from_bytes(&bytes) {
                Ok(s) => Referenced::Text(s),
                Err(line) => Referenced::BadUtf8(line),
            };
            self.referenced.insert(path, r);
        }
        Ok(())
    }

    /// Resolves the spelling of a changeset target: the location rules of
    /// [`Locations::resolve_target`], and for a document outside every family, that it
    /// exists, is valid UTF-8 and is no palette document. Every reader of a target
    /// (the changeset parser, staging, promotion, P004) resolves it here, so they never
    /// disagree about one.
    pub fn target(&self, spelling: &str) -> Result<Target, String> {
        let t = self.loc.resolve_target(spelling)?;
        if let Target::Document(_) = &t {
            let path = self.loc.target_file(&t);
            let refuse = |why: &str| format!("`{spelling}` cannot be edited by a changeset: {why}");
            if let Some(f) = self.files.iter().find(|f| same_path_nocase(&f.path, &path)) {
                return Err(refuse(&format!(
                    "it is a palette document ({}); records, indexes, changesets, staging and work documents are generated or frozen and are never changeset targets, and a family document is named by its logical name",
                    role_label(f.role)
                )));
            }
            match self.referenced.get(&path) {
                None => return Err(refuse("it does not exist")),
                Some(Referenced::BadUtf8(line)) => {
                    return Err(refuse(&format!("it is not valid UTF-8 (line {line})")));
                }
                Some(Referenced::Spelled(actual)) => {
                    return Err(refuse(&format!(
                        "it is spelled `{actual}` on disk; a document has one spelling, so write it as it is on disk"
                    )));
                }
                Some(Referenced::Text(_)) => {}
            }
        }
        Ok(t)
    }

    /// The current lines of the document `t` names; `None` when it does not exist or
    /// is not valid UTF-8.
    pub fn target_source(&self, t: &Target) -> Option<Source> {
        let path = self.loc.target_file(t);
        match t {
            Target::Family(..) => self.file(&path)?.doc.as_ref().map(|d| d.src.clone()),
            Target::Document(_) => match self.referenced.get(&path)? {
                Referenced::Text(s) => Some(s.clone()),
                _ => None,
            },
        }
    }

    /// Whether `path` is a document outside every family that a changeset names.
    pub fn is_referenced(&self, path: &Path) -> bool {
        matches!(self.referenced.get(path), Some(Referenced::Text(_)))
    }

    fn add(&mut self, src: &dyn FileSource, path: PathBuf, role: Role) -> Res<()> {
        if self.by_path.contains_key(&path) {
            return Ok(());
        }
        let bytes = src
            .read(&path)
            .map_err(|e| io_err(&format!("reading {}", path.display()), &e))?;
        let Some(bytes) = bytes else { return Ok(()) };
        let (doc, bad_utf8) = match Doc::parse_bytes(&bytes) {
            Ok(d) => (Some(d), None),
            Err(line) => (None, Some(line)),
        };
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.by_path.insert(path.clone(), self.files.len());
        self.files.push(DocFile {
            rel: rel_slash(&self.root, &path),
            path,
            name,
            role,
            doc,
            bad_utf8,
        });
        Ok(())
    }

    fn list_rst(&self, src: &dyn FileSource, dir: &Path) -> Res<Vec<PathBuf>> {
        let entries = src
            .list(dir)
            .map_err(|e| io_err(&format!("listing {}", dir.display()), &e))?;
        Ok(entries
            .into_iter()
            .filter(|e| !e.is_dir && e.name.ends_with(".rst"))
            .map(|e| dir.join(e.name))
            .collect())
    }

    fn list_rst_deep(&self, src: &dyn FileSource, dir: &Path, out: &mut Vec<PathBuf>) -> Res<()> {
        let entries = src
            .list(dir)
            .map_err(|e| io_err(&format!("listing {}", dir.display()), &e))?;
        for e in entries {
            let p = dir.join(&e.name);
            if e.is_dir {
                // A linked folder may lead outside the project, or back into itself.
                let linked =
                    std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink());
                if !linked {
                    self.list_rst_deep(src, &p, out)?;
                }
            } else if e.name.ends_with(".rst") {
                out.push(p);
            }
        }
        Ok(())
    }

    fn discover(&mut self, src: &dyn FileSource) -> Res<()> {
        // Every document under the staging folder is a staging document, whatever its
        // name or depth; loading these first keeps any other family from claiming one.
        let staging = self.loc.dir_of(Family::Staging);
        let mut mirrors = Vec::new();
        self.list_rst_deep(src, &staging, &mut mirrors)?;
        for p in mirrors {
            let parts: Vec<String> = p
                .strip_prefix(&staging)
                .map(|r| {
                    r.components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            let fam = match parts.as_slice() {
                [dir, _] if dir == "design" => Some(Family::Design),
                [dir, _] if dir == "spec" => Some(Family::Spec),
                [file] => [Family::Principles, Family::Glossary, Family::Contributing]
                    .into_iter()
                    .find(|f| *file == format!("{}.rst", f.key())),
                _ => None,
            };
            self.add(src, p, Role::Staging(fam))?;
        }
        for (fam, role) in [
            (Family::Backlog, Role::Backlog),
            (Family::State, Role::State),
            (Family::Principles, Role::Principles),
            (Family::Glossary, Role::Glossary),
            (Family::Contributing, Role::Contributing),
        ] {
            let p = self.loc.file_of(fam);
            self.add(src, p, role)?;
        }
        // phase-<N>/phase.rst and phase-<N>/deliverables/*.rst
        for (base, is_phase) in [
            (self.loc.phase_root(), true),
            (self.loc.deliverable_root(), false),
        ] {
            let entries = src
                .list(&base)
                .map_err(|e| io_err(&format!("listing {}", base.display()), &e))?;
            for e in entries.into_iter().filter(|e| e.is_dir) {
                let Some(n) = e.name.strip_prefix("phase-") else {
                    continue;
                };
                let Ok(num) = n.parse::<u32>() else { continue };
                let dir = base.join(&e.name);
                if is_phase {
                    self.add(src, dir.join("phase.rst"), Role::Phase(Some(num)))?;
                } else {
                    for p in self.list_rst(src, &dir.join("deliverables"))? {
                        self.add(src, p, Role::Deliverable(Some(num)))?;
                    }
                }
            }
        }
        for (fam, role) in [
            (Family::Rfc, Role::Rfc),
            (Family::Adr, Role::Adr),
            (Family::Changeset, Role::Changeset),
            (Family::Design, Role::Design),
            (Family::Spec, Role::Spec),
        ] {
            let dir = self.loc.dir_of(fam);
            for p in self.list_rst(src, &dir)? {
                let is_index = p.file_name().is_some_and(|n| n == "index.rst");
                let r = if is_index && matches!(fam, Family::Rfc | Family::Adr | Family::Changeset)
                {
                    Role::Index(fam)
                } else {
                    role
                };
                self.add(src, p, r)?;
            }
        }
        Ok(())
    }

    /// The file at `path`.
    pub fn file(&self, path: &Path) -> Option<&DocFile> {
        self.by_path.get(path).map(|i| &self.files[*i])
    }

    /// Index of the file at `path`.
    pub fn index_of(&self, path: &Path) -> Option<usize> {
        self.by_path.get(path).copied()
    }

    /// Files with `role`.
    pub fn with_role(&self, role: Role) -> impl Iterator<Item = &DocFile> {
        self.files.iter().filter(move |f| f.role == role)
    }

    /// The single file of a single-file family, when it exists.
    pub fn single(&self, fam: Family) -> Option<&DocFile> {
        self.file(&self.loc.file_of(fam))
    }
}

/// What a palette document is, for messages.
fn role_label(role: Role) -> &'static str {
    match role {
        Role::Layout => "the layout",
        Role::Backlog | Role::State | Role::Phase(_) | Role::Deliverable(_) => "a work document",
        Role::Rfc | Role::Adr => "a record",
        Role::Changeset => "a changeset",
        Role::Index(_) => "an index",
        Role::Staging(_) => "a staging document",
        Role::Design => "a design document",
        Role::Spec => "a specification",
        Role::Principles => "the principles document",
        Role::Glossary => "the glossary",
        Role::Contributing => "the contributing document",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::DiskSource;
    use std::fs;

    const LAYOUT: &str = "Layout — X\n==========\n\nFamilies\n--------\n\n:backlog: internal\n:phase: internal\n:deliverable: internal\n:state: internal\n:rfc: docs/rfc\n:adr: internal\n:changeset: internal\n:staging: internal\n:design: internal\n:spec: internal\n:principles: internal\n:glossary: internal\n:contributing: internal\n:checker: none\n";

    #[test]
    fn discovers_documents_by_family() {
        let t = tempfile::tempdir().expect("tmp");
        let r = t.path().canonicalize().expect("canon");
        fs::create_dir_all(r.join("_palette/phase-2/deliverables")).expect("mk");
        fs::create_dir_all(r.join("docs/rfc")).expect("mk");
        fs::write(r.join("_palette/layout.rst"), LAYOUT).expect("w");
        fs::write(r.join("_palette/backlog.rst"), "Backlog — X\n===========\n").expect("w");
        fs::write(
            r.join("_palette/phase-2/phase.rst"),
            "Phase 2 — X\n===========\n",
        )
        .expect("w");
        fs::write(
            r.join("_palette/phase-2/deliverables/deliverable-1-a.rst"),
            "x\n",
        )
        .expect("w");
        fs::write(
            r.join("docs/rfc/rfc-0001-a.rst"),
            "RFC-0001: A\n===========\n",
        )
        .expect("w");
        fs::write(r.join("docs/rfc/index.rst"), "Index\n=====\n").expect("w");
        fs::write(r.join("docs/rfc/notes.txt"), "ignored").expect("w");
        let snap = Snapshot::load(&DiskSource, &r).expect("load");
        let roles: Vec<(String, Role)> =
            snap.files.iter().map(|f| (f.rel.clone(), f.role)).collect();
        assert!(roles.contains(&("_palette/layout.rst".to_string(), Role::Layout)));
        assert!(roles.contains(&(
            "_palette/phase-2/phase.rst".to_string(),
            Role::Phase(Some(2))
        )));
        assert!(roles.contains(&(
            "_palette/phase-2/deliverables/deliverable-1-a.rst".to_string(),
            Role::Deliverable(Some(2))
        )));
        assert!(roles.contains(&("docs/rfc/rfc-0001-a.rst".to_string(), Role::Rfc)));
        assert!(roles.contains(&("docs/rfc/index.rst".to_string(), Role::Index(Family::Rfc))));
        assert!(!roles.iter().any(|(p, _)| p.ends_with("notes.txt")));
    }

    #[test]
    fn without_a_layout_the_families_are_inferred_from_the_documents() {
        let t = tempfile::tempdir().expect("tmp");
        let r = t.path().canonicalize().expect("canon");
        for d in [
            "notes/rfc",
            "notes/adr",
            "notes/staging/spec",
            "notes/spec",
            "target/x",
        ] {
            fs::create_dir_all(r.join(d)).expect("mk");
        }
        fs::write(
            r.join("notes/rfc/rfc-0001-a.rst"),
            "RFC-0001: A\n===========\n",
        )
        .expect("w");
        fs::write(r.join("notes/rfc/index.rst"), "Index — RFC\n===========\n").expect("w");
        fs::write(
            r.join("notes/adr/adr-0002-b.rst"),
            "ADR-0002: B\n===========\n",
        )
        .expect("w");
        fs::write(
            r.join("notes/spec/thing.rst"),
            "Thing\n=====\n\n:Status: Contract\n",
        )
        .expect("w");
        fs::write(
            r.join("notes/staging/spec/thing.rst"),
            "Thing\n=====\n\n:Status: Contract\n",
        )
        .expect("w");
        fs::write(r.join("notes/glossary.rst"), "Glossary — X\n============\n").expect("w");
        fs::write(
            r.join("target/x/rfc-0009-z.rst"),
            "RFC-0009: Z\n===========\n",
        )
        .expect("w");
        fs::write(r.join("README.rst"), "Read me\n=======\n").expect("w");
        let snap = Snapshot::load(&DiskSource, &r).expect("load");
        assert!(snap.inferred);
        assert_eq!(
            snap.layout.placement(Family::Rfc),
            Placement::Path("notes/rfc".into())
        );
        assert_eq!(
            snap.layout.placement(Family::Adr),
            Placement::Path("notes/adr".into())
        );
        assert_eq!(
            snap.layout.placement(Family::Spec),
            Placement::Path("notes/spec".into())
        );
        assert_eq!(
            snap.layout.placement(Family::Staging),
            Placement::Path("notes/staging".into())
        );
        assert_eq!(
            snap.layout.placement(Family::Glossary),
            Placement::Path("notes/glossary.rst".into())
        );
        assert_eq!(snap.layout.placement(Family::Backlog), Placement::Internal);
        assert!(
            snap.layout.problems.is_empty(),
            "{:?}",
            snap.layout.problems
        );
        let roles: Vec<(String, Role)> =
            snap.files.iter().map(|f| (f.rel.clone(), f.role)).collect();
        assert!(roles.contains(&("notes/rfc/rfc-0001-a.rst".to_string(), Role::Rfc)));
        assert!(roles.contains(&("notes/rfc/index.rst".to_string(), Role::Index(Family::Rfc))));
        assert!(roles.contains(&(
            "notes/staging/spec/thing.rst".to_string(),
            Role::Staging(Some(Family::Spec))
        )));
        assert!(
            !roles
                .iter()
                .any(|(p, _)| p.starts_with("target/") || p == "README.rst")
        );
        assert!(!roles.iter().any(|(_, r)| *r == Role::Layout));
    }

    #[test]
    fn templates_and_fixtures_are_not_documents() {
        let t = tempfile::tempdir().expect("tmp");
        let r = t.path().canonicalize().expect("canon");
        fs::create_dir_all(r.join("templates")).expect("mk");
        fs::create_dir_all(r.join("tests/fixtures/x")).expect("mk");
        fs::write(
            r.join("templates/rfc.rst"),
            "RFC-<NNNN>: <Title>\n===================\n",
        )
        .expect("w");
        fs::write(
            r.join("templates/glossary.rst"),
            "Glossary — <Project>\n====================\n",
        )
        .expect("w");
        fs::write(
            r.join("tests/fixtures/x/rfc-0001-a.rst"),
            "RFC-0001: A\n===========\n",
        )
        .expect("w");
        let snap = Snapshot::load(&DiskSource, &r).expect("load");
        assert!(
            snap.files.is_empty(),
            "{:?}",
            snap.files.iter().map(|f| &f.rel).collect::<Vec<_>>()
        );
        assert_eq!(snap.layout.placement(Family::Rfc), Placement::Internal);
    }

    #[test]
    fn a_family_found_in_two_places_is_a_layout_problem() {
        let t = tempfile::tempdir().expect("tmp");
        let r = t.path().canonicalize().expect("canon");
        fs::create_dir_all(r.join("a")).expect("mk");
        fs::create_dir_all(r.join("b")).expect("mk");
        fs::write(r.join("a/rfc-0001-a.rst"), "RFC-0001: A\n===========\n").expect("w");
        fs::write(r.join("b/rfc-0002-b.rst"), "RFC-0002: B\n===========\n").expect("w");
        let snap = Snapshot::load(&DiskSource, &r).expect("load");
        assert_eq!(
            snap.layout.placement(Family::Rfc),
            Placement::Path("a".into())
        );
        assert_eq!(snap.layout.problems.len(), 1);
        assert_eq!(
            snap.layout.problems[0].rel.as_deref(),
            Some("b/rfc-0002-b.rst")
        );
        // The second document is in the snapshot, so the lint can report it.
        assert!(
            snap.files
                .iter()
                .any(|f| f.rel == "b/rfc-0002-b.rst" && f.role == Role::Rfc)
        );
        let an = crate::lint::Analysis::build(&snap);
        let findings = crate::lint::run(&DiskSource, &snap, &an);
        assert!(
            findings
                .iter()
                .any(|f| f.rule == "P012" && f.file.ends_with("rfc-0002-b.rst")),
            "{}",
            crate::lint::to_text(&findings)
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_folder_is_not_walked() {
        let t = tempfile::tempdir().expect("tmp");
        let r = t.path().canonicalize().expect("canon");
        fs::create_dir_all(r.join("tests/fixtures/x")).expect("mk");
        fs::write(
            r.join("tests/fixtures/x/rfc-0001-a.rst"),
            "RFC-0001: A\n===========\n",
        )
        .expect("w");
        std::os::unix::fs::symlink(r.join("tests/fixtures"), r.join("testdata")).expect("ln");
        let snap = Snapshot::load(&DiskSource, &r).expect("load");
        assert!(snap.files.is_empty());
    }
}
