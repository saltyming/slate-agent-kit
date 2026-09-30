//! State tools: `palette_state_record` and `palette_state_resolve`.
//!
//! Owns adding decisions, open questions and discrepancies to the state document with
//! a server-allocated identifier, and resolving them (graduating a decision to a
//! pointer, answering or withdrawing a question, removing a fixed discrepancy).
//! State entries are one bullet each; the `Updated` date is stamped on every change.

use std::path::{Path, PathBuf};

use super::{Ctx, Session, WriteResult, run_write};
use crate::edit;
use crate::errors::{PalError, Res};
use crate::layout::Family;
use crate::params::{StateRecordParams, StateResolveParams};
use crate::records::RecId;
use crate::rst::Doc;
use crate::state::{EntryKind, State};
use crate::text::Source;
use crate::util::{is_iso_date, join_lexical, relative_link};

/// Evidence kinds listed by the state template's discrepancy entry.
pub const EVIDENCE_KINDS: [&str; 3] = ["static", "build", "runtime"];

fn load(s: &Session<'_, '_>) -> Res<(PathBuf, Source, State)> {
    let path = s.snap.loc.file_of(Family::State);
    let f = s.file(&path)?;
    let doc = f
        .doc
        .as_ref()
        .ok_or_else(|| PalError::not_found(format!("{} does not exist", f.rel)))?;
    let st = State::strict(doc, &f.rel)?;
    Ok((path, doc.src.clone(), st))
}

fn strip_period(s: &str) -> String {
    edit::one_line(s).trim_end_matches('.').trim().to_string()
}

/// Text as a sentence for an entry: no trailing period doubled, and none added after
/// a question or exclamation mark (the template's period follows the text directly).
fn sentence(s: &str) -> String {
    let t = strip_period(s);
    if t.ends_with('?') || t.ends_with('!') {
        t
    } else {
        format!("{t}.")
    }
}

fn require(name: &str, v: &Option<String>) -> Res<String> {
    match v.as_deref().map(edit::one_line) {
        Some(x) if !x.is_empty() => Ok(x),
        _ => Err(PalError::invalid(format!("{name} is required"))),
    }
}

fn append_entry(src: &mut Source, rel: &str, kind: EntryKind, lines: &[String]) -> Res<()> {
    let doc = Doc::parse(src.clone());
    let st = State::strict(&doc, rel)?;
    let sec = st
        .section(kind)
        .ok_or_else(|| PalError::parse(rel, 1, "the state document lacks a section"))?;
    let blank_before = sec.entries.is_empty();
    edit::append_to_section(src, &doc, sec.heading, lines, blank_before);
    Ok(())
}

fn touch(src: &mut Source, today: &str) -> Res<()> {
    edit::set_header_field(src, "Updated", today)
        .map(|_| ())
        .map_err(|m| PalError::parse("state", 1, m))
}

fn remove_entry(src: &mut Source, rel: &str, kind: EntryKind, number: u32) -> Res<()> {
    let doc = Doc::parse(src.clone());
    let st = State::strict(&doc, rel)?;
    let e = st
        .entry(kind, number)
        .ok_or_else(|| PalError::not_found(format!("{}{number} does not exist", kind.prefix())))?;
    src.splice(e.start..e.end, &[]);
    ensure_none_if_empty(src, rel, kind)
}

fn ensure_none_if_empty(src: &mut Source, rel: &str, kind: EntryKind) -> Res<()> {
    let doc = Doc::parse(src.clone());
    let st = State::strict(&doc, rel)?;
    if let Some(sec) = st.section(kind) {
        let has_content =
            (sec.body_start..sec.body_end).any(|i| !doc.src.text(i).trim().is_empty());
        if !has_content {
            edit::replace_section_body(src, &doc, sec.heading, &["None.".to_string()]);
        }
    }
    Ok(())
}

fn decision_lines(n: u32, text: &str, source: &str, today: &str, target: &str) -> Vec<String> {
    edit::bullet_lines(&format!(
        "D-{n} {} Source: {source}, {today}. Target: {target}.",
        sentence(text)
    ))
}

/// `palette_state_record`.
pub fn state_record(ctx: &Ctx, p: StateRecordParams) -> Res<WriteResult> {
    run_write(ctx, &p.project, p.dry_run.unwrap_or(false), |s| {
        let (path, mut src, st) = load(s)?;
        let rel = s.rel(&path);
        let text = strip_period(&p.text);
        if text.is_empty() {
            return Err(PalError::invalid("text is required"));
        }
        let today = s.today.clone();
        let (kind, lines, label) = match p.kind.trim() {
            "decision" => {
                let source = require("source", &p.source)?;
                let target = strip_period(&require("target", &p.target)?);
                let n = st.max_number(EntryKind::Decision) + 1;
                (
                    EntryKind::Decision,
                    decision_lines(n, &text, &source, &today, &target),
                    format!("D-{n}"),
                )
            }
            "question" => {
                let affects = strip_period(&require("affects", &p.affects)?);
                let n = st.max_number(EntryKind::Question) + 1;
                let proposal = match p
                    .proposal
                    .as_deref()
                    .map(edit::one_line)
                    .filter(|x| !x.is_empty())
                {
                    Some(pr) => {
                        let by = require("proposal_by", &p.proposal_by)?;
                        format!("{} ({by}, {today})", strip_period(&pr))
                    }
                    None => "none".to_string(),
                };
                let lines = edit::bullet_lines(&format!(
                    "Q-{n} {} Affects: {affects}. Proposal: {proposal}.",
                    sentence(&text)
                ));
                (EntryKind::Question, lines, format!("Q-{n}"))
            }
            "discrepancy" => {
                let ev = require("evidence", &p.evidence)?;
                if !EVIDENCE_KINDS.contains(&ev.as_str()) {
                    return Err(PalError::invalid(format!(
                        "evidence must be one of {}",
                        EVIDENCE_KINDS.join(", ")
                    )));
                }
                let date = p.date.clone().unwrap_or_else(|| today.clone());
                if !is_iso_date(date.trim()) {
                    return Err(PalError::invalid("date must be YYYY-MM-DD"));
                }
                let n = st.max_number(EntryKind::Discrepancy) + 1;
                let lines = edit::bullet_lines(&format!(
                    "X-{n} {} Evidence: {ev}, {}.",
                    sentence(&text),
                    date.trim()
                ));
                (EntryKind::Discrepancy, lines, format!("X-{n}"))
            }
            other => {
                return Err(PalError::invalid(format!(
                    "kind must be decision, question or discrepancy, got `{other}`"
                )));
            }
        };
        append_entry(&mut src, &rel, kind, &lines)?;
        touch(&mut src, &today)?;
        s.put(&path, &src)?;
        Ok(vec![label])
    })
}

fn written_link(s: &Session<'_, '_>, from_dir: &Path, written: &str) -> Res<String> {
    let w = written.trim();
    if let Some(id) = RecId::parse(w) {
        let rec = s.an.records.get(id).ok_or_else(|| s.missing_record(id))?;
        let target = &s.snap.files[rec.file].path;
        return Ok(format!("`{id} <{}>`_", relative_link(from_dir, target)));
    }
    let target = join_lexical(&s.snap.root, w)
        .ok_or_else(|| PalError::invalid(format!("`{w}` leaves the project")))?;
    if !target.starts_with(&s.snap.root) {
        return Err(PalError::invalid(format!("`{w}` leaves the project")));
    }
    let exists = crate::vfs::FileSource::read(&*s.ov, &target)
        .map_err(|e| PalError::io("reading", &e))?
        .is_some();
    if !exists {
        return Err(PalError::not_found(format!(
            "`{w}` does not exist; a graduated decision must point at the record or document it was written into"
        )));
    }
    Ok(format!("`{w} <{}>`_", relative_link(from_dir, &target)))
}

/// `palette_state_resolve`.
pub fn state_resolve(ctx: &Ctx, p: StateResolveParams) -> Res<WriteResult> {
    run_write(ctx, &p.project, p.dry_run.unwrap_or(false), |s| {
        let (path, mut src, st) = load(s)?;
        let rel = s.rel(&path);
        let id = p.id.trim();
        let kind = EntryKind::of_id(id).ok_or_else(|| {
            PalError::invalid(format!(
                "`{id}` is not an entry identifier (`D-<n>`, `Q-<n>` or `X-<n>`)"
            ))
        })?;
        let number: u32 = id[2..]
            .parse()
            .map_err(|_| PalError::invalid(format!("`{id}` is not an entry identifier")))?;
        let Some(entry) = st.entry(kind, number).cloned() else {
            return Err(match st.pointer_naming(kind, number) {
                Some(p) => PalError::invariant(format!(
                    "{id} is already graduated: the pointer `{}` names it",
                    p.text.trim_start_matches("- ")
                )),
                None => PalError::not_found(format!("{id} does not exist in the state document")),
            });
        };
        let today = s.today.clone();
        let mut allocated = Vec::new();
        match (kind, p.resolution.trim()) {
            (EntryKind::Decision, "graduated") => {
                if entry.is_pointer() {
                    return Err(PalError::invariant(format!(
                        "{id} is already a pointer to where it was written"
                    )));
                }
                let written = require("written", &p.written)?;
                let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
                let link = written_link(s, &dir, &written)?;
                // The pointer keeps only the id: the decision's content now lives in the
                // record or document it was written into. It is one line, however long
                // the link is.
                let lines = vec![format!("- {}{number} Graduated to {link}.", kind.prefix())];
                src.splice(entry.start..entry.end, &lines);
            }
            (EntryKind::Question, "answered") => {
                let answer = require("answer", &p.answer)?;
                let source = require("source", &p.source)?;
                let target = strip_period(&require("target", &p.target)?);
                remove_entry(&mut src, &rel, kind, number)?;
                let n = st.max_number(EntryKind::Decision) + 1;
                append_entry(
                    &mut src,
                    &rel,
                    EntryKind::Decision,
                    &decision_lines(n, &answer, &source, &today, &target),
                )?;
                allocated.push(format!("D-{n}"));
            }
            (EntryKind::Question, "withdrawn") | (EntryKind::Discrepancy, "fixed") => {
                remove_entry(&mut src, &rel, kind, number)?;
            }
            (k, r) => {
                let allowed = match k {
                    EntryKind::Decision => "graduated",
                    EntryKind::Question => "answered or withdrawn",
                    EntryKind::Discrepancy => "fixed",
                };
                return Err(PalError::invalid(format!(
                    "{id} can be resolved as {allowed}, not `{r}`"
                )));
            }
        }
        touch(&mut src, &today)?;
        s.put(&path, &src)?;
        Ok(allocated)
    })
}

/// Removes the graduated-pointer decisions and stamps `Updated`; used when a phase closes.
pub(super) fn drop_pointers_and_touch(s: &mut Session<'_, '_>) -> Res<()> {
    let (path, mut src, st) = load(s)?;
    let rel = s.rel(&path);
    let mut spans: Vec<(usize, usize)> = st
        .section(EntryKind::Decision)
        .map(|sec| {
            sec.entries
                .iter()
                .filter(|e| e.is_pointer())
                .map(|e| (e.start, e.end))
                .collect()
        })
        .unwrap_or_default();
    spans.sort();
    for (a, b) in spans.into_iter().rev() {
        src.splice(a..b, &[]);
    }
    ensure_none_if_empty(&mut src, &rel, EntryKind::Decision)?;
    let today = s.today.clone();
    touch(&mut src, &today)?;
    s.put(&path, &src)
}
