//! Backlog, phase and deliverable tools.
//!
//! `palette_backlog_add`, `palette_backlog_update`, `palette_phase_open`,
//! `palette_phase_close`, `palette_deliverable_create` and `palette_deliverable_update`.
//! Each edits the backlog (and the phase or deliverable files) line by line, keeping
//! everything the operation does not concern. Item status lives only in the backlog.

use std::path::{Path, PathBuf};

use super::{Ctx, Session, WriteResult, run_write};
use crate::backlog::{Backlog, in_phase, parse_item_ref};
use crate::docs::Role;
use crate::edit;
use crate::errors::{PalError, Res};
use crate::layout::Family;
use crate::params::*;
use crate::rst::Doc;
use crate::schema::{SectionSpec, schemas};
use crate::text::{Eol, Source};
use crate::util::{relative_link, slugify};

fn item_spec() -> Res<&'static SectionSpec> {
    schemas()
        .get("backlog")
        .and_then(|s| s.sections.get(1))
        .and_then(|s| s.subs.first())
        .ok_or_else(|| PalError::invariant("the backlog template has no item block"))
}

fn check_item_field(name: &str, value: &str) -> Res<()> {
    let spec = item_spec()?;
    let f = spec
        .fields
        .iter()
        .find(|f| f.literal_name() == Some(name))
        .ok_or_else(|| {
            PalError::invariant(format!("the backlog template has no `{name}` field"))
        })?;
    f.value
        .check(value, f.repeating)
        .map_err(|m| PalError::invalid(format!("{name}: {m}")))
}

pub(super) fn load_backlog(s: &Session<'_, '_>) -> Res<(PathBuf, Source, Backlog)> {
    let path = s.snap.loc.file_of(Family::Backlog);
    let f = s.file(&path)?;
    let doc = f
        .doc
        .as_ref()
        .ok_or_else(|| PalError::not_found(format!("{} does not exist", f.rel)))?;
    let b = Backlog::strict(doc, &f.rel)?;
    Ok((path, doc.src.clone(), b))
}

fn reparse(src: &Source, rel: &str) -> Res<(Doc, Backlog)> {
    let doc = Doc::parse(src.clone());
    let b = Backlog::strict(&doc, rel)?;
    Ok((doc, b))
}

fn next_item_id(b: &Backlog) -> u32 {
    b.items.iter().map(|i| i.id).max().unwrap_or(0) + 1
}

fn depends_value(refs: &[u32]) -> String {
    if refs.is_empty() {
        "none".to_string()
    } else {
        refs.iter()
            .map(|n| format!("B-{n}"))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

fn parse_depends(b: &Backlog, raw: &[String], own: Option<u32>) -> Res<Vec<u32>> {
    let mut out = Vec::new();
    for r in raw {
        let n = parse_item_ref(r).ok_or_else(|| {
            PalError::invalid(format!("`{r}` is not an item identifier (`B-<n>`)"))
        })?;
        if b.item(n).is_none() {
            return Err(PalError::not_found(format!("item B-{n} does not exist")));
        }
        if Some(n) == own {
            return Err(PalError::invalid("an item cannot depend on itself"));
        }
        if !out.contains(&n) {
            out.push(n);
        }
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn item_block(
    id: u32,
    title: &str,
    ty: &str,
    source: &str,
    signal: &str,
    depends: &[u32],
    body: Option<&str>,
) -> Vec<String> {
    let mut lines = edit::heading_lines(3, &format!("B-{id} {}", edit::one_line(title)));
    lines.push(String::new());
    lines.extend(edit::field_lines("Status", "proposed"));
    lines.extend(edit::field_lines("Type", ty));
    lines.extend(edit::field_lines("Source", source));
    lines.extend(edit::field_lines("Priority-signal", signal));
    lines.extend(edit::field_lines("Deliverable", "none"));
    lines.extend(edit::field_lines("Depends", &depends_value(depends)));
    lines.extend(edit::field_lines("Outcome", "none"));
    if let Some(b) = body.filter(|b| !b.trim().is_empty()) {
        lines.push(String::new());
        lines.extend(edit::paragraph_lines(b));
    }
    lines
}

fn append_item(src: &mut Source, rel: &str, block: &[String]) -> Res<()> {
    let (doc, b) = reparse(src, rel)?;
    let ih = b
        .items_heading
        .ok_or_else(|| PalError::parse(rel, 1, "the backlog has no Items section"))?;
    edit::append_to_section(src, &doc, ih, block, true);
    Ok(())
}

fn set_item_field(src: &mut Source, rel: &str, id: u32, name: &str, value: &str) -> Res<()> {
    let (_, b) = reparse(src, rel)?;
    let it = b
        .item(id)
        .ok_or_else(|| PalError::not_found(format!("item B-{id} does not exist")))?;
    let f = it.field(name).ok_or_else(|| {
        PalError::parse(
            rel,
            it.line + 1,
            format!("item B-{id} has no `{name}` field"),
        )
    })?;
    edit::replace_field(src, f, value);
    Ok(())
}

/// `palette_backlog_add`.
pub fn backlog_add(ctx: &Ctx, p: BacklogAddParams) -> Res<WriteResult> {
    run_write(ctx, &p.project, p.dry_run.unwrap_or(false), |s| {
        if p.title.trim().is_empty() {
            return Err(PalError::invalid("title is required"));
        }
        let (path, mut src, b) = load_backlog(s)?;
        let rel = s.rel(&path);
        let signal = p
            .priority_signal
            .clone()
            .unwrap_or_else(|| "none".to_string());
        check_item_field("Type", p.item_type.trim())?;
        check_item_field("Source", p.source.trim())?;
        check_item_field("Priority-signal", signal.trim())?;
        let depends = parse_depends(&b, &p.depends.clone().unwrap_or_default(), None)?;
        let id = next_item_id(&b);
        let block = item_block(
            id,
            &p.title,
            p.item_type.trim(),
            p.source.trim(),
            signal.trim(),
            &depends,
            p.body.as_deref(),
        );
        append_item(&mut src, &rel, &block)?;
        s.put(&path, &src)?;
        Ok(vec![format!("B-{id}")])
    })
}

fn check_transition(cur: &str, new: &str, b: &Backlog) -> Res<()> {
    if cur == new {
        return Err(PalError::invalid(format!("the item is already {cur}")));
    }
    if new == "dropped" {
        return Ok(());
    }
    let ok = match (cur, new) {
        ("proposed", "approved") => true,
        ("approved", n) if let Some(k) = in_phase(n) => {
            return match b.phase(k) {
                Some(ph) if ph.state.as_deref() == Some("active") => Ok(()),
                Some(_) => Err(PalError::invariant(format!(
                    "phase {k} is not active; an item can enter only the active phase"
                ))),
                None => Err(PalError::invariant(format!(
                    "phase {k} does not exist; open it with palette_phase_open"
                ))),
            };
        }
        (c, "done") => in_phase(c).is_some(),
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(PalError::invariant(format!(
            "an item cannot move from {cur} to {new}; status moves proposed → approved → in-phase-<N> → done, or to dropped from any status"
        )))
    }
}

/// `palette_backlog_update`.
pub fn backlog_update(ctx: &Ctx, p: BacklogUpdateParams) -> Res<WriteResult> {
    run_write(ctx, &p.project, p.dry_run.unwrap_or(false), |s| {
        let (path, mut src, b) = load_backlog(s)?;
        let rel = s.rel(&path);
        let id = parse_item_ref(&p.item).ok_or_else(|| {
            PalError::invalid(format!("`{}` is not an item identifier (`B-<n>`)", p.item))
        })?;
        let item = b
            .item(id)
            .ok_or_else(|| PalError::not_found(format!("item B-{id} does not exist")))?;
        let cur_status = item.value("Status").unwrap_or("").to_string();
        if let Some(t) = &p.item_type {
            check_item_field("Type", t.trim())?;
        }
        if let Some(t) = &p.source {
            check_item_field("Source", t.trim())?;
        }
        if let Some(t) = &p.priority_signal {
            check_item_field("Priority-signal", t.trim())?;
        }
        if let Some(t) = &p.status {
            check_item_field("Status", t.trim())?;
            check_transition(&cur_status, t.trim(), &b)?;
        }
        let depends = match &p.depends {
            Some(d) => Some(parse_depends(&b, d, Some(id))?),
            None => None,
        };
        if let Some(t) = &p.title {
            if t.trim().is_empty() {
                return Err(PalError::invalid("title cannot be empty"));
            }
            let (doc, b2) = reparse(&src, &rel)?;
            let it = b2
                .item(id)
                .ok_or_else(|| PalError::not_found(format!("item B-{id} does not exist")))?;
            edit::retitle(
                &mut src,
                &doc,
                it.heading,
                &format!("B-{id} {}", edit::one_line(t)),
            );
        }
        for (name, v) in [
            ("Status", p.status.as_deref()),
            ("Type", p.item_type.as_deref()),
            ("Source", p.source.as_deref()),
            ("Priority-signal", p.priority_signal.as_deref()),
            ("Outcome", p.outcome.as_deref()),
        ] {
            if let Some(v) = v {
                if v.trim().is_empty() {
                    return Err(PalError::invalid(format!("{name} cannot be empty")));
                }
                set_item_field(&mut src, &rel, id, name, v.trim())?;
            }
        }
        if let Some(d) = depends {
            set_item_field(&mut src, &rel, id, "Depends", &depends_value(&d))?;
        }
        if let Some(body) = &p.body {
            let (_, b2) = reparse(&src, &rel)?;
            let it = b2
                .item(id)
                .ok_or_else(|| PalError::not_found(format!("item B-{id} does not exist")))?;
            let new_lines = if body.trim().is_empty() {
                Vec::new()
            } else {
                edit::paragraph_lines(body)
            };
            match it.body {
                Some((bs, be)) => {
                    let (start, repl): (usize, Vec<String>) =
                        if new_lines.is_empty() && bs > 0 && src.text(bs - 1).trim().is_empty() {
                            (bs - 1, Vec::new())
                        } else {
                            (bs, new_lines)
                        };
                    src.splice(start..be, &repl);
                }
                None if !new_lines.is_empty() => {
                    let mut ins = vec![String::new()];
                    ins.extend(new_lines);
                    src.splice(it.fields_end..it.fields_end, &ins);
                }
                None => {}
            }
        }
        s.put(&path, &src)?;
        Ok(Vec::new())
    })
}

fn bullets_or_none(items: &[String]) -> Vec<String> {
    let clean: Vec<&String> = items.iter().filter(|i| !i.trim().is_empty()).collect();
    if clean.is_empty() {
        vec!["None.".to_string()]
    } else {
        clean.iter().flat_map(|i| edit::bullet_lines(i)).collect()
    }
}

fn section_titles(tpl: &str) -> Vec<String> {
    schemas()
        .get(tpl)
        .map(|s| {
            s.sections
                .iter()
                .filter_map(|x| x.title.literal().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn assemble(title_line: &str, header: &[String], sections: Vec<(String, Vec<String>)>) -> Source {
    let mut lines = edit::heading_lines(1, title_line);
    if !header.is_empty() {
        lines.push(String::new());
        lines.extend(header.iter().cloned());
    }
    for (t, body) in sections {
        lines.push(String::new());
        lines.extend(edit::heading_lines(2, &t));
        lines.push(String::new());
        lines.extend(body);
    }
    Source::from_lines(&lines, Eol::Lf)
}

fn link_text(text: &str, target: &str) -> String {
    format!("`{text} <{target}>`_")
}

/// `palette_phase_open`.
pub fn phase_open(ctx: &Ctx, p: PhaseOpenParams) -> Res<WriteResult> {
    run_write(ctx, &p.project, p.dry_run.unwrap_or(false), |s| {
        let (bpath, mut bsrc, b) = load_backlog(s)?;
        let rel = s.rel(&bpath);
        if let Some(a) = b.active_phase() {
            return Err(PalError::invariant(format!(
                "phase {} is still active; close it with palette_phase_close first",
                a.number
            )));
        }
        let criteria = p.exit_criteria.clone().unwrap_or_default();
        if criteria.iter().all(|c| c.trim().is_empty()) {
            return Err(PalError::invalid(
                "exit_criteria needs at least one outcome a person can check",
            ));
        }
        for (name, v) in [
            ("title", &p.title),
            ("goal", &p.goal),
            ("reason", &p.reason),
        ] {
            if v.trim().is_empty() {
                return Err(PalError::invalid(format!("{name} is required")));
            }
        }
        let on_disk = s.snap.files.iter().filter_map(|f| match f.role {
            Role::Phase(Some(n)) | Role::Deliverable(Some(n)) => Some(n),
            _ => None,
        });
        let n = b
            .phases
            .iter()
            .map(|x| x.number)
            .chain(on_disk)
            .max()
            .unwrap_or(0)
            + 1;
        let mut item_ids = Vec::new();
        for r in p.items.clone().unwrap_or_default() {
            let id = parse_item_ref(&r).ok_or_else(|| {
                PalError::invalid(format!("`{r}` is not an item identifier (`B-<n>`)"))
            })?;
            let it = b
                .item(id)
                .ok_or_else(|| PalError::not_found(format!("item B-{id} does not exist")))?;
            let st = it.value("Status").unwrap_or("");
            if st != "approved" {
                return Err(PalError::invariant(format!(
                    "item B-{id} is {st}; only approved items can enter a phase"
                )));
            }
            if !item_ids.contains(&id) {
                item_ids.push(id);
            }
        }
        let titles = section_titles("phase");
        if titles.len() != 4 {
            return Err(PalError::invariant(
                "the phase template no longer has four sections",
            ));
        }
        let title_raw = schemas()
            .get("phase")
            .map(|x| x.title.raw.clone())
            .unwrap_or_default();
        let title_line = title_raw
            .replace("<N>", &n.to_string())
            .replace("<Title>", &edit::one_line(&p.title));
        let assumptions: Vec<String> = p
            .assumptions
            .clone()
            .unwrap_or_default()
            .iter()
            .map(|a| {
                format!(
                    "{} — risk if wrong: {}",
                    edit::one_line(&a.assumption),
                    edit::one_line(&a.risk)
                )
            })
            .collect();
        let phase_src = assemble(
            &title_line,
            &[],
            vec![
                (titles[0].clone(), edit::paragraph_lines(&p.goal)),
                (titles[1].clone(), edit::paragraph_lines(&p.reason)),
                (titles[2].clone(), bullets_or_none(&assumptions)),
                (titles[3].clone(), bullets_or_none(&criteria)),
            ],
        );
        let phase_path = s.snap.loc.phase_file(n);
        s.put(&phase_path, &phase_src)?;
        let bdir = bpath.parent().map(Path::to_path_buf).unwrap_or_default();
        let entry = edit::field_lines(
            &format!("phase-{n}"),
            &format!(
                "{} — active",
                link_text(
                    &relative_link(&bdir, &phase_path).to_string(),
                    &relative_link(&bdir, &phase_path)
                )
            ),
        );
        let (doc, b2) = reparse(&bsrc, &rel)?;
        let ph = b2
            .phases_heading
            .ok_or_else(|| PalError::parse(&rel, 1, "the backlog has no Phases section"))?;
        edit::append_to_section(&mut bsrc, &doc, ph, &entry, false);
        for id in item_ids {
            set_item_field(&mut bsrc, &rel, id, "Status", &format!("in-phase-{n}"))?;
        }
        s.put(&bpath, &bsrc)?;
        Ok(vec![format!("phase-{n}")])
    })
}

/// `palette_phase_close`.
pub fn phase_close(ctx: &Ctx, p: PhaseCloseParams) -> Res<WriteResult> {
    run_write(ctx, &p.project, p.dry_run.unwrap_or(false), |s| {
        let (bpath, mut bsrc, b) = load_backlog(s)?;
        let rel = s.rel(&bpath);
        let n = p.phase;
        let entry = b
            .phase(n)
            .ok_or_else(|| PalError::not_found(format!("phase {n} does not exist")))?;
        if entry.state.as_deref() != Some("active") {
            return Err(PalError::invariant(format!("phase {n} is not active")));
        }
        let in_phase_ids: Vec<u32> = b
            .items
            .iter()
            .filter(|i| in_phase(i.value("Status").unwrap_or("")) == Some(n))
            .map(|i| i.id)
            .collect();
        let given = p.items.clone().unwrap_or_default();
        let mut fates: Vec<(u32, bool, String)> = Vec::new();
        for c in &given {
            let id = parse_item_ref(&c.item).ok_or_else(|| {
                PalError::invalid(format!("`{}` is not an item identifier (`B-<n>`)", c.item))
            })?;
            if !in_phase_ids.contains(&id) {
                return Err(PalError::invalid(format!(
                    "item B-{id} is not in phase {n}"
                )));
            }
            if fates.iter().any(|f| f.0 == id) {
                return Err(PalError::invalid(format!("item B-{id} is listed twice")));
            }
            match c.result.trim() {
                "done" => {
                    let out = c.outcome.clone().unwrap_or_default();
                    if out.trim().is_empty() {
                        return Err(PalError::invalid(format!(
                            "item B-{id}: a done item needs an outcome pointer (the RFC, ADR or changelog entry that records the result)"
                        )));
                    }
                    fates.push((id, true, edit::one_line(&out)));
                }
                "dropped" => fates.push((id, false, "none".to_string())),
                other => {
                    return Err(PalError::invalid(format!(
                        "item B-{id}: result must be `done` or `dropped`, got `{other}`"
                    )));
                }
            }
        }
        let missing: Vec<String> = in_phase_ids
            .iter()
            .filter(|i| !fates.iter().any(|f| f.0 == **i))
            .map(|i| format!("B-{i}"))
            .collect();
        if !missing.is_empty() {
            return Err(PalError::invalid(format!(
                "phase {n} still holds {}; give each a result (done with an outcome, or dropped)",
                missing.join(", ")
            )));
        }
        let new_items = p.new_items.clone().unwrap_or_default();
        for ni in &new_items {
            if ni.title.trim().is_empty() {
                return Err(PalError::invalid("a new item needs a title"));
            }
            check_item_field("Type", ni.item_type.trim())?;
            check_item_field(
                "Priority-signal",
                ni.priority_signal.as_deref().unwrap_or("none").trim(),
            )?;
        }
        for (id, done, outcome) in &fates {
            set_item_field(
                &mut bsrc,
                &rel,
                *id,
                "Status",
                if *done { "done" } else { "dropped" },
            )?;
            set_item_field(&mut bsrc, &rel, *id, "Outcome", outcome)?;
        }
        let (_, b2) = reparse(&bsrc, &rel)?;
        let ph = b2
            .phase(n)
            .ok_or_else(|| PalError::not_found(format!("phase {n} does not exist")))?;
        // The link to the phase file stays; only the state changes.
        let value = ph.field.value.trim();
        let closed = match value.rsplit_once(" — ") {
            Some((head, _)) => format!("{head} — closed"),
            None => format!("{value} — closed"),
        };
        edit::replace_field(&mut bsrc, &ph.field, &closed);
        let mut allocated = Vec::new();
        for (next, ni) in (next_item_id(&b)..).zip(new_items.iter()) {
            let signal = ni
                .priority_signal
                .clone()
                .unwrap_or_else(|| "none".to_string());
            let block = item_block(
                next,
                &ni.title,
                ni.item_type.trim(),
                &format!("phase-{n} close"),
                signal.trim(),
                &[],
                ni.body.as_deref(),
            );
            append_item(&mut bsrc, &rel, &block)?;
            allocated.push(format!("B-{next}"));
        }
        s.put(&bpath, &bsrc)?;
        super::state::drop_pointers_and_touch(s)?;
        // The phase and deliverable files stay: they record what was approved, and the
        // backlog's `closed` mark and item outcomes are the status.
        Ok(allocated)
    })
}

fn deliverable_sections(
    what: &str,
    done: &[String],
    not_this: &[String],
    reference: &[String],
) -> Res<Vec<(String, Vec<String>)>> {
    let t = section_titles("deliverable");
    if t.len() != 4 {
        return Err(PalError::invariant(
            "the deliverable template no longer has four sections",
        ));
    }
    Ok(vec![
        (t[0].clone(), edit::paragraph_lines(what)),
        (t[1].clone(), bullets_or_none(done)),
        (t[2].clone(), bullets_or_none(not_this)),
        (t[3].clone(), bullets_or_none(reference)),
    ])
}

fn active_item(b: &Backlog, raw: &str) -> Res<(u32, u32)> {
    let id = parse_item_ref(raw)
        .ok_or_else(|| PalError::invalid(format!("`{raw}` is not an item identifier (`B-<n>`)")))?;
    let it = b
        .item(id)
        .ok_or_else(|| PalError::not_found(format!("item B-{id} does not exist")))?;
    let st = it.value("Status").unwrap_or("");
    let ph = in_phase(st).ok_or_else(|| {
        PalError::invariant(format!(
            "item B-{id} is {st}; a deliverable belongs to an item in the active phase"
        ))
    })?;
    match b.phase(ph) {
        Some(e) if e.state.as_deref() == Some("active") => Ok((id, ph)),
        _ => Err(PalError::invariant(format!("phase {ph} is not active"))),
    }
}

/// `palette_deliverable_create`.
pub fn deliverable_create(ctx: &Ctx, p: DeliverableCreateParams) -> Res<WriteResult> {
    run_write(ctx, &p.project, p.dry_run.unwrap_or(false), |s| {
        let (bpath, mut bsrc, b) = load_backlog(s)?;
        let rel = s.rel(&bpath);
        let (id, phase) = active_item(&b, &p.item)?;
        let it = b
            .item(id)
            .ok_or_else(|| PalError::not_found(format!("item B-{id} does not exist")))?;
        if it.value("Deliverable").map(str::trim) != Some("none") {
            return Err(PalError::invariant(format!(
                "item B-{id} already has a deliverable; use palette_deliverable_update"
            )));
        }
        let done = p.done_when.clone().unwrap_or_default();
        if done.iter().all(|d| d.trim().is_empty()) {
            return Err(PalError::invalid(
                "done_when needs at least one outcome a person can check",
            ));
        }
        let slug = slugify(&p.title);
        if slug.is_empty() {
            return Err(PalError::invalid(
                "title needs at least one ASCII letter or digit for the file name",
            ));
        }
        let title_raw = schemas()
            .get("deliverable")
            .map(|x| x.title.raw.clone())
            .unwrap_or_default();
        let title_line = title_raw
            .replace("<N>", &id.to_string())
            .replace("<Title>", &edit::one_line(&p.title));
        let sections = deliverable_sections(
            &p.what_and_why,
            &done,
            &p.not_this.clone().unwrap_or_default(),
            &p.implementation_reference.clone().unwrap_or_default(),
        )?;
        let src = assemble(
            &title_line,
            &edit::field_lines("Backlog", &format!("B-{id}")),
            sections,
        );
        let path = s
            .snap
            .loc
            .deliverables_dir(phase)
            .join(format!("deliverable-{id}-{slug}.rst"));
        if s.snap.file(&path).is_some() {
            return Err(PalError::invariant(format!(
                "{} already exists",
                s.rel(&path)
            )));
        }
        s.put(&path, &src)?;
        let bdir = bpath.parent().map(Path::to_path_buf).unwrap_or_default();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        set_item_field(
            &mut bsrc,
            &rel,
            id,
            "Deliverable",
            &link_text(&name, &relative_link(&bdir, &path)),
        )?;
        s.put(&bpath, &bsrc)?;
        Ok(vec![format!("deliverable-{id}")])
    })
}

/// `palette_deliverable_update`.
pub fn deliverable_update(ctx: &Ctx, p: DeliverableUpdateParams) -> Res<WriteResult> {
    run_write(ctx, &p.project, p.dry_run.unwrap_or(false), |s| {
        let (bpath, _, b) = load_backlog(s)?;
        let (id, _) = active_item(&b, &p.item)?;
        let it = b
            .item(id)
            .ok_or_else(|| PalError::not_found(format!("item B-{id} does not exist")))?;
        let link = it
            .value("Deliverable")
            .and_then(|v| v.split("<").nth(1))
            .and_then(|v| v.split(">`").next())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                PalError::not_found(format!(
                    "item B-{id} has no deliverable; create one with palette_deliverable_create"
                ))
            })?
            .to_string();
        let dir = bpath.parent().map(Path::to_path_buf).unwrap_or_default();
        let path = crate::util::join_lexical(&dir, &link).ok_or_else(|| {
            PalError::invalid(format!("the deliverable link `{link}` is not valid"))
        })?;
        let mut src = s.source(&path)?;
        let rel = s.rel(&path);
        let titles = section_titles("deliverable");
        if let Some(t) = &p.title {
            let doc = Doc::parse(src.clone());
            let th = doc
                .title()
                .ok_or_else(|| PalError::parse(&rel, 1, "the deliverable has no title"))?;
            let title_raw = schemas()
                .get("deliverable")
                .map(|x| x.title.raw.clone())
                .unwrap_or_default();
            let new_title = title_raw
                .replace("<N>", &id.to_string())
                .replace("<Title>", &edit::one_line(t));
            let idx = doc
                .headings
                .iter()
                .position(|h| h.line == th.line)
                .unwrap_or(0);
            edit::retitle(&mut src, &doc, idx, &new_title);
        }
        let updates: [(usize, Option<Vec<String>>); 4] = [
            (0, p.what_and_why.as_ref().map(|w| edit::paragraph_lines(w))),
            (1, p.done_when.as_ref().map(|v| bullets_or_none(v))),
            (2, p.not_this.as_ref().map(|v| bullets_or_none(v))),
            (
                3,
                p.implementation_reference
                    .as_ref()
                    .map(|v| bullets_or_none(v)),
            ),
        ];
        for (i, body) in updates {
            let Some(body) = body else { continue };
            let title = titles.get(i).cloned().unwrap_or_default();
            let doc = Doc::parse(src.clone());
            let idx = doc
                .headings_at(2)
                .find(|(_, h)| h.title == title)
                .map(|(x, _)| x)
                .ok_or_else(|| {
                    PalError::parse(&rel, 1, format!("the deliverable has no `{title}` section"))
                })?;
            edit::replace_section_body(&mut src, &doc, idx, &body);
        }
        s.put(&path, &src)?;
        Ok(Vec::new())
    })
}
