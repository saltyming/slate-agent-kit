//! The tool functions behind the MCP handlers: read tools return text, write tools
//! return a [`WriteResult`]. Kept free of the MCP types so tests call them directly.
//!
//! Owns input checks common to read tools (project resolution, path filters) and
//! rendering. The write tools themselves are in `ops`.
//! Entry points: [`lint`], [`status`], [`layout`], [`template`].

use std::path::PathBuf;

use crate::docs::Snapshot;
use crate::errors::{PalError, Res};
use crate::layout::Family;
use crate::lint::{self, Analysis};
use crate::ops::Ctx;
use crate::params::{LayoutParams, LintParams, StatusParams, TemplateParams};
use crate::records::RecId;
use crate::schema::template_text;
use crate::vfs::DiskSource;

fn load(ctx: &Ctx, project: &str) -> Res<Snapshot> {
    let root = ctx.roots.resolve_project(project)?;
    Snapshot::load(&DiskSource, &root)
}

/// `palette_lint`.
pub fn lint(ctx: &Ctx, p: LintParams) -> Res<String> {
    let snap = load(ctx, &p.project)?;
    let an = Analysis::build(&snap);
    let mut findings = lint::run(&DiskSource, &snap, &an);
    if let Some(paths) = p.paths.filter(|v| !v.is_empty()) {
        let mut list: Vec<PathBuf> = Vec::new();
        for raw in paths {
            let path = PathBuf::from(raw.trim());
            if path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err(PalError::invalid(format!(
                    "path `{raw}` contains `..`; give a project-relative path"
                )));
            }
            list.push(path);
        }
        findings = lint::filter_paths(findings, &snap.root, &list);
    }
    Ok(lint::to_json(&findings))
}

/// `palette_status`.
pub fn status(ctx: &Ctx, p: StatusParams) -> Res<String> {
    let snap = load(ctx, &p.project)?;
    let an = Analysis::build(&snap);
    let findings = lint::run(&DiskSource, &snap, &an);
    let record = match p.record.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) => Some(RecId::parse(r).ok_or_else(|| {
            PalError::invalid(format!("`{r}` is not a record identifier (`RFC-0001`)"))
        })?),
        None => None,
    };
    if let Some(id) = record
        && an.records.get(id).is_none()
    {
        return Err(PalError::not_found(format!("{id} does not exist")));
    }
    Ok(crate::status::summary(&snap, &an, &findings, record))
}

/// `palette_layout`.
pub fn layout(ctx: &Ctx, p: LayoutParams) -> Res<String> {
    let snap = load(ctx, &p.project)?;
    let loc = &snap.loc;
    let mut families = serde_json::Map::new();
    for fam in Family::ALL {
        let path = match fam {
            Family::Backlog | Family::State | Family::Principles | Family::Glossary => {
                loc.file_of(fam)
            }
            Family::Phase => loc.phase_root().join("phase-<N>").join("phase.rst"),
            Family::Deliverable => loc
                .deliverable_root()
                .join("phase-<N>")
                .join("deliverables")
                .join("deliverable-<N>-<slug>.rst"),
            other => loc.dir_of(other),
        };
        families.insert(
            fam.key().to_string(),
            serde_json::json!({
                "placement": loc.placement(fam).text(),
                "path": path.display().to_string(),
            }),
        );
    }
    let problems: Vec<String> = snap
        .layout
        .problems
        .iter()
        .map(|p| format!("line {}: {}", p.line + 1, p.message))
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "project": snap.root.display().to_string(),
        "families": families,
        "checker": snap.layout.checker.as_ref().map(|c| c.0.clone()),
        "problems": problems,
    }))
    .map_err(|e| PalError::invalid(e.to_string()))
}

/// `palette_template`.
pub fn template(p: TemplateParams) -> Res<String> {
    let name = p.family.trim().to_ascii_lowercase();
    if name == "staging" {
        return Err(PalError::invalid(
            "staging documents are generated from the maintained design and spec documents and have no template; use `design` or `spec`",
        ));
    }
    template_text(&name).map(str::to_string).ok_or_else(|| {
        PalError::invalid(format!(
            "unknown template `{name}`; available: backlog, phase, deliverable, state, rfc, adr, changeset, design, spec, principles, glossary, layout, house-style, rubrics"
        ))
    })
}
