//! `palette_init` and `palette_layout_set`.
//!
//! Owns creating a project's `_palette/` (layout, backlog, state, `.gitignore`) from the
//! templates, and moving one document family to a new placement with every link to and
//! from the moved files rewritten. Does not decide where families should live.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{Ctx, Session, WriteResult, run_init, run_write};
use crate::docs::Role;
use crate::edit;
use crate::errors::{ErrCode, PalError, Res};
use crate::layout::{Family, Layout, Placement, normalize_path};
use crate::params::{InitParams, LayoutSetParams};
use crate::rst::{Doc, split_anchor};
use crate::schema::schemas;
use crate::text::{Eol, Source};
use crate::util::{join_lexical, relative_link};

/// The text of `layout.rst` for the given placements.
pub fn layout_source(
    name: &str,
    placements: &BTreeMap<Family, Placement>,
    checker: &str,
) -> Source {
    let (title, section) = schemas()
        .get("layout")
        .map(|s| {
            (
                s.title.raw.replace("<Project>", name),
                s.sections
                    .first()
                    .and_then(|x| x.title.literal())
                    .unwrap_or("Families")
                    .to_string(),
            )
        })
        .unwrap_or_else(|| (format!("Layout — {name}"), "Families".to_string()));
    let mut lines = edit::heading_lines(1, &title);
    lines.push(String::new());
    lines.extend(edit::heading_lines(2, &section));
    lines.push(String::new());
    for fam in Family::ALL {
        let p = placements.get(&fam).cloned().unwrap_or(Placement::Internal);
        lines.push(format!(":{}: {}", fam.key(), p.text()));
    }
    lines.push(format!(":checker: {checker}"));
    Source::from_lines(&lines, Eol::Lf)
}

/// An empty document for template `tpl`: the title, the `Updated` date if the template
/// has one, and every section stating `None.`.
pub fn skeleton(tpl: &str, name: &str, today: &str) -> Source {
    let Some(schema) = schemas().get(tpl) else {
        return Source::default();
    };
    let mut lines = edit::heading_lines(1, &schema.title.raw.replace("<Project>", name));
    let dated: Vec<&str> = schema
        .header
        .iter()
        .filter(|f| f.value.raw.trim() == "<YYYY-MM-DD>")
        .filter_map(|f| f.literal_name())
        .collect();
    if !dated.is_empty() {
        lines.push(String::new());
        for n in dated {
            lines.push(format!(":{n}: {today}"));
        }
    }
    for sec in &schema.sections {
        let Some(title) = sec.title.literal() else {
            continue;
        };
        lines.push(String::new());
        lines.extend(edit::heading_lines(2, title));
        lines.push(String::new());
        lines.push("None.".to_string());
    }
    Source::from_lines(&lines, Eol::Lf)
}

/// `palette_init`.
pub fn init(ctx: &Ctx, p: InitParams) -> Res<WriteResult> {
    run_init(
        ctx,
        &p.project,
        p.dry_run.unwrap_or(false),
        |ov, root, today| {
            let layout_path = root.join(crate::layout::PALETTE_DIR).join("layout.rst");
            let existing = crate::vfs::FileSource::read(&*ov, &layout_path)
                .map_err(|e| PalError::io("reading layout.rst", &e))?;
            if existing.is_some() {
                return Err(PalError::new(
                    ErrCode::AlreadyInitialized,
                    format!(
                        "{} already has a _palette/layout.rst; use palette_layout_set to move a family",
                        root.display()
                    ),
                ));
            }
            let name = p
                .name
                .clone()
                .filter(|n| !n.trim().is_empty())
                .or_else(|| root.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "project".to_string());
            let mut placements: BTreeMap<Family, Placement> = BTreeMap::new();
            for (k, v) in p.placements.clone().unwrap_or_default() {
                let fam = Family::from_key(&k).ok_or_else(|| {
                    PalError::invalid(format!(
                        "`{k}` is not a document family; families are: {}",
                        Family::ALL.map(Family::key).join(", ")
                    ))
                })?;
                let v = v.trim();
                let placement = if v == "internal" {
                    Placement::Internal
                } else {
                    Placement::Path(
                        normalize_path(v, fam)
                            .map_err(|m| PalError::invalid(format!("{k}: {m}")))?,
                    )
                };
                placements.insert(fam, placement);
            }
            let checker = p.checker.clone().unwrap_or_else(|| "none".to_string());
            let layout_src = layout_source(&name, &placements, edit::one_line(&checker).as_str());
            let layout = Layout::parse(&layout_src);
            if let Some(prob) = layout.problems.first() {
                return Err(PalError::invalid(format!(
                    "the placements are not valid: {}",
                    prob.message
                )));
            }
            let loc = layout.locations(root);
            let put = |ov: &mut crate::vfs::Overlay<'_>, path: &Path, src: &Source| -> Res<()> {
                ov.write(path, src.to_bytes())
                    .map_err(|e| PalError::io(&format!("writing {}", path.display()), &e))
            };
            put(ov, &layout_path, &layout_src)?;
            put(
                ov,
                &loc.palette_dir().join(".gitignore"),
                &Source::from_text("*\n"),
            )?;
            for (fam, tpl) in [(Family::Backlog, "backlog"), (Family::State, "state")] {
                let path = loc.file_of(fam);
                let present = crate::vfs::FileSource::read(&*ov, &path)
                    .map_err(|e| PalError::io("reading", &e))?
                    .is_some();
                if !present {
                    put(ov, &path, &skeleton(tpl, &name, today))?;
                }
            }
            Ok(Vec::new())
        },
    )
}

fn family_of(role: Role) -> Option<Family> {
    Some(match role {
        Role::Backlog => Family::Backlog,
        Role::State => Family::State,
        Role::Phase(_) => Family::Phase,
        Role::Deliverable(_) => Family::Deliverable,
        Role::Rfc | Role::Index(Family::Rfc) => Family::Rfc,
        Role::Adr | Role::Index(Family::Adr) => Family::Adr,
        Role::Changeset | Role::Index(Family::Changeset) => Family::Changeset,
        Role::Design => Family::Design,
        Role::Spec => Family::Spec,
        Role::Principles => Family::Principles,
        Role::Glossary => Family::Glossary,
        Role::Staging(_) => Family::Staging,
        Role::Layout | Role::Index(_) => return None,
    })
}

fn re_root(old: &Path, old_base: &Path, new_base: &Path) -> Option<PathBuf> {
    old.strip_prefix(old_base)
        .ok()
        .map(|rest| new_base.join(rest))
}

/// `palette_layout_set`.
pub fn layout_set(ctx: &Ctx, p: LayoutSetParams) -> Res<WriteResult> {
    if p.confirmed_by_user != Some(true) {
        return Err(PalError::invalid(
            "palette_layout_set needs confirmed_by_user: true. Moving a family changes who can see its documents, so ask the user first and pass true only after they agree.",
        ));
    }
    let fam = Family::from_key(p.family.trim()).ok_or_else(|| {
        PalError::invalid(format!(
            "`{}` is not a document family; families are: {}",
            p.family,
            Family::ALL.map(Family::key).join(", ")
        ))
    })?;
    let value = p.placement.trim().to_string();
    let placement = if value == "internal" {
        Placement::Internal
    } else {
        Placement::Path(normalize_path(&value, fam).map_err(PalError::invalid)?)
    };
    run_write(ctx, &p.project, p.dry_run.unwrap_or(false), |s| {
        move_family(s, fam, placement)
    })
}

fn move_family(s: &mut Session<'_, '_>, fam: Family, placement: Placement) -> Res<Vec<String>> {
    if s.snap.layout.placement(fam) == placement {
        return Err(PalError::invalid(format!(
            "{} is already placed at `{}`",
            fam.key(),
            placement.text()
        )));
    }
    let root = s.snap.root.clone();
    let old_loc = s.snap.loc.clone();
    let new_layout = s.snap.layout.with_placement(fam, placement.clone());
    let new_loc = new_layout.locations(&root);
    // Where each file of the family goes.
    let mut moved: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
    for f in &s.snap.files {
        if family_of(f.role) != Some(fam) {
            continue;
        }
        let new = match fam {
            Family::Backlog | Family::State | Family::Principles | Family::Glossary => {
                Some(new_loc.file_of(fam))
            }
            Family::Phase => re_root(&f.path, &old_loc.phase_root(), &new_loc.phase_root()),
            Family::Deliverable => re_root(
                &f.path,
                &old_loc.deliverable_root(),
                &new_loc.deliverable_root(),
            ),
            other => re_root(&f.path, &old_loc.dir_of(other), &new_loc.dir_of(other)),
        };
        if let Some(n) = new
            && n != f.path
        {
            if let Some(line) = f.bad_utf8 {
                return Err(PalError::parse(
                    &f.rel,
                    line,
                    "the file is not valid UTF-8, so it cannot be moved with its links rewritten",
                ));
            }
            moved.insert(f.path.clone(), n);
        }
    }
    for new in moved.values() {
        let exists = crate::vfs::FileSource::read(&*s.ov, new)
            .map_err(|e| PalError::io("reading", &e))?
            .is_some();
        if exists {
            return Err(PalError::invariant(format!(
                "{} already exists; move or remove it before placing {} there",
                s.rel(new),
                fam.key()
            )));
        }
    }
    // Rewrite links and move files.
    let mut plan: Vec<(PathBuf, Option<PathBuf>, Source)> = Vec::new();
    for f in &s.snap.files {
        let Some(doc) = &f.doc else { continue };
        if matches!(f.role, Role::Layout | Role::Staging(_) | Role::Index(_)) {
            if let Some(new) = moved.get(&f.path) {
                plan.push((f.path.clone(), Some(new.clone()), doc.src.clone()));
            }
            continue;
        }
        let new_path = moved.get(&f.path).cloned();
        let old_dir = f.path.parent().map(Path::to_path_buf).unwrap_or_default();
        let new_dir = new_path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| old_dir.clone());
        let mut src = doc.src.clone();
        let mut edits: Vec<(usize, (usize, usize), String)> = Vec::new();
        for l in doc.links() {
            if crate::rst::is_external(&l.target) {
                continue;
            }
            let (path_part, anchor) = split_anchor(&l.target);
            if path_part.is_empty() {
                continue;
            }
            let Some(target_old) = join_lexical(&old_dir, path_part) else {
                continue;
            };
            let target_new = moved
                .get(&target_old)
                .cloned()
                .unwrap_or_else(|| target_old.clone());
            if target_new == target_old && new_dir == old_dir {
                continue;
            }
            let mut text = relative_link(&new_dir, &target_new);
            if let Some(a) = anchor {
                text.push('#');
                text.push_str(a);
            }
            if text != l.target {
                edits.push((l.line, l.target_span, text));
            }
        }
        edits.sort_by_key(|e| std::cmp::Reverse((e.0, (e.1).0)));
        for (line, span, text) in &edits {
            let mut t = src.text(*line).to_string();
            t.replace_range(span.0..span.1, text);
            let eol = src.lines[*line].eol;
            src.lines[*line].text = t;
            src.lines[*line].eol = eol;
        }
        if new_path.is_some() || !edits.is_empty() {
            plan.push((f.path.clone(), new_path, src));
        }
    }
    for (old, new, src) in plan {
        match new {
            Some(n) => {
                s.put_raw(&n, &src)?;
                s.remove(&old)?;
            }
            None => s.put_raw(&old, &src)?,
        }
    }
    // The layout line for the family.
    let layout_path = old_loc.layout_file();
    let mut lsrc = s.source(&layout_path)?;
    let ldoc = Doc::parse(lsrc.clone());
    let field = ldoc
        .fields(0, ldoc.src.len())
        .into_iter()
        .find(|f| f.name == fam.key())
        .ok_or_else(|| {
            PalError::invariant(format!(
                "layout.rst has no `:{}:` line to change; add it by hand first",
                fam.key()
            ))
        })?;
    edit::replace_field(&mut lsrc, &field, &placement.text());
    s.put(&layout_path, &lsrc)?;
    Ok(Vec::new())
}
