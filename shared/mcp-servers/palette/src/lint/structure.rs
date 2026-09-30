//! P002 structure, P003 identity, P012 layout and P014 time-varying fields.
//!
//! Applies the schemas derived from the templates to each document: title pattern,
//! required header fields and values, required sections, and the item and phase-entry
//! blocks of the backlog. Also checks that file names, numbers and titles agree.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

use super::{Cx, Finding};
use crate::docs::{DocFile, Role};
use crate::records::{RecordKind, Records};
use crate::rst::{Doc, Field};
use crate::schema::{FieldSpec, SectionSpec, schemas};
use crate::util::{is_kebab_rst, re};

/// Header fields only the server writes; P014 owns their value format.
pub const TIME_VARYING: [&str; 4] = ["Implementation", "Verification", "Implementers", "Revised"];

static RECORD_NAME: LazyLock<Regex> =
    LazyLock::new(|| re(r"^(rfc|adr)-(\d{4})-[a-z0-9]+(?:-[a-z0-9]+)*\.rst$"));
static CHANGESET_NAME: LazyLock<Regex> = LazyLock::new(|| re(r"^(rfc|adr)-(\d{4})\.rst$"));
static CHANGESET_TITLE: LazyLock<Regex> = LazyLock::new(|| re(r"^Changeset:\s*(RFC|ADR)-(\d{4})$"));
static DELIVERABLE_NAME: LazyLock<Regex> =
    LazyLock::new(|| re(r"^deliverable-(\d+)-[a-z0-9]+(?:-[a-z0-9]+)*\.rst$"));
static DELIVERABLE_TITLE: LazyLock<Regex> = LazyLock::new(|| re(r"^Deliverable (\d+):"));
static PHASE_TITLE: LazyLock<Regex> = LazyLock::new(|| re(r"^Phase (\d+)\b"));

pub(super) fn check(cx: &Cx, out: &mut Vec<Finding>) {
    layout_problems(cx, out);
    for f in &cx.snap.files {
        if !is_kebab_rst(&f.name) {
            out.push(Finding::error(
                "P003",
                f,
                0,
                format!("file name `{}` is not lowercase ASCII kebab-case (`words-joined-by-hyphens.rst`)", f.name),
            ));
        }
        let (Some(doc), Some(tpl)) = (&f.doc, f.role.template()) else {
            continue;
        };
        let Some(schema) = schemas().get(tpl) else {
            continue;
        };
        check_title(f, doc, &schema.title, out);
        let header = doc.header_fields();
        let anchor = doc.title().map(|t| t.line).unwrap_or(0);
        check_fields(f, &header, &schema.header, anchor, f.role, out);
        check_sections(f, doc, &schema.sections, f.role, out);
    }
    identity(cx, out);
}

fn check_title(f: &DocFile, doc: &Doc, pattern: &crate::schema::Pattern, out: &mut Vec<Finding>) {
    let titles: Vec<_> = doc.headings_at(1).collect();
    match titles.first() {
        None => out.push(Finding::error(
            "P002",
            f,
            0,
            format!(
                "the document has no title; expected `{}` underlined with `=`",
                pattern.raw
            ),
        )),
        Some((_, t)) => {
            if !pattern.is_match(&t.title) {
                out.push(Finding::error(
                    "P002",
                    f,
                    t.line,
                    format!("title `{}` does not match `{}`", t.title, pattern.raw),
                ));
            }
        }
    }
    for (_, extra) in titles.iter().skip(1) {
        out.push(Finding::error(
            "P002",
            f,
            extra.line,
            format!(
                "extra title-level section `{}`; a document has one `=` title",
                extra.title
            ),
        ));
    }
}

fn check_fields(
    f: &DocFile,
    actual: &[Field],
    specs: &[FieldSpec],
    anchor: usize,
    role: Role,
    out: &mut Vec<Finding>,
) {
    let mut furthest: Option<usize> = None;
    let record = matches!(role, Role::Rfc | Role::Adr);
    for spec in specs {
        let Some(name) = spec.literal_name() else {
            continue;
        };
        if role == Role::Layout && name != "checker" {
            continue;
        }
        let Some(pos) = actual.iter().position(|a| a.name == name) else {
            out.push(Finding::error(
                "P002",
                f,
                anchor,
                format!("missing required field `:{name}:` ({})", spec.value.raw),
            ));
            continue;
        };
        if let Some(fur) = furthest
            && pos < fur
        {
            out.push(Finding::error(
                "P002",
                f,
                actual[pos].start,
                format!("field `:{name}:` is out of order; keep the template's order"),
            ));
        }
        furthest = Some(furthest.map_or(pos, |x| x.max(pos)));
        if let Err(msg) = spec.value.check(&actual[pos].value, spec.repeating) {
            let (rule, what) = if record && TIME_VARYING.contains(&name) {
                ("P014", "is not in its template format")
            } else {
                ("P002", "has a value outside the allowed values")
            };
            out.push(Finding::error(
                rule,
                f,
                actual[pos].start,
                format!("field `:{name}:` {what}: {msg}"),
            ));
        }
    }
}

fn check_sections(
    f: &DocFile,
    doc: &Doc,
    specs: &[SectionSpec],
    role: Role,
    out: &mut Vec<Finding>,
) {
    let actual: Vec<(usize, &crate::rst::Heading)> = doc.headings_at(2).collect();
    let mut used = vec![false; actual.len()];
    let mut mapped: Vec<(usize, usize)> = Vec::new(); // (spec index, actual index)
    for (si, spec) in specs.iter().enumerate() {
        if spec.repeating {
            for (j, (_, h)) in actual.iter().enumerate() {
                if !used[j] && spec.title.is_match(&h.title) {
                    used[j] = true;
                }
            }
            continue;
        }
        match actual
            .iter()
            .enumerate()
            .find(|(j, (_, h))| !used[*j] && spec.title.is_match(&h.title))
        {
            Some((j, _)) => {
                used[j] = true;
                mapped.push((si, j));
            }
            None => out.push(Finding::error(
                "P002",
                f,
                doc.title().map(|t| t.line).unwrap_or(0),
                format!("missing required section `{}`", spec.title.raw),
            )),
        }
    }
    for (j, (_, h)) in actual.iter().enumerate() {
        if !used[j] {
            out.push(Finding::error(
                "P002",
                f,
                h.line,
                format!(
                    "extra section `{}`; the template has no such section",
                    h.title
                ),
            ));
        }
    }
    let mut furthest = 0usize;
    for (si, j) in &mapped {
        if *j < furthest {
            out.push(Finding::error(
                "P002",
                f,
                actual[*j].1.line,
                format!(
                    "section `{}` is out of order; the template order is: {}",
                    specs[*si].title.raw,
                    order_text(specs)
                ),
            ));
        }
        furthest = furthest.max(*j);
    }
    for (si, spec) in specs.iter().enumerate() {
        if spec.fields.is_empty() && spec.subs.is_empty() {
            continue;
        }
        for (j, (hidx, h)) in actual.iter().enumerate() {
            let is_this = if spec.repeating {
                spec.title.is_match(&h.title)
            } else {
                mapped.contains(&(si, j))
            };
            if is_this {
                check_section_body(f, doc, *hidx, spec, role, out);
            }
        }
    }
}

fn order_text(specs: &[SectionSpec]) -> String {
    specs
        .iter()
        .map(|s| s.title.raw.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

fn check_section_body(
    f: &DocFile,
    doc: &Doc,
    hidx: usize,
    spec: &SectionSpec,
    role: Role,
    out: &mut Vec<Finding>,
) {
    let h = &doc.headings[hidx];
    let actual = doc.body_fields(hidx);
    let literal: Vec<FieldSpec> = spec
        .fields
        .iter()
        .filter(|s| s.literal_name().is_some())
        .cloned()
        .collect();
    if !literal.is_empty() {
        check_fields(f, &actual, &literal, h.line, role, out);
    }
    let patterned: Vec<&FieldSpec> = spec
        .fields
        .iter()
        .filter(|s| s.literal_name().is_none())
        .collect();
    if !patterned.is_empty() {
        for a in &actual {
            match patterned.iter().find(|s| s.name.is_match(&a.name)) {
                None => out.push(Finding::error(
                    "P002",
                    f,
                    a.start,
                    format!(
                        "field `:{}:` does not match `:{}:`",
                        a.name, patterned[0].name.raw
                    ),
                )),
                Some(s) => {
                    if let Err(msg) = s.value.check(&a.value, s.repeating) {
                        out.push(Finding::error(
                            "P002",
                            f,
                            a.start,
                            format!(
                                "field `:{}:` has a value outside the allowed values: {msg}",
                                a.name
                            ),
                        ));
                    }
                }
            }
        }
    }
    if !spec.subs.is_empty() {
        for c in doc.children(hidx) {
            let ch = &doc.headings[c];
            match spec.subs.iter().find(|s| s.title.is_match(&ch.title)) {
                None => out.push(Finding::error(
                    "P002",
                    f,
                    ch.line,
                    format!("`{}` does not match `{}`", ch.title, spec.subs[0].title.raw),
                )),
                Some(sub) => check_section_body(f, doc, c, sub, role, out),
            }
        }
    }
}

fn layout_problems(cx: &Cx, out: &mut Vec<Finding>) {
    let Some(f) = cx.snap.file(&cx.snap.loc.layout_file()) else {
        return;
    };
    for p in &cx.snap.layout.problems {
        out.push(Finding::error("P012", f, p.line, p.message.clone()));
    }
}

fn identity(cx: &Cx, out: &mut Vec<Finding>) {
    for rec in &cx.an.records.list {
        let f = &cx.snap.files[rec.file];
        let role_kind = if f.role == Role::Rfc {
            RecordKind::Rfc
        } else {
            RecordKind::Adr
        };
        if is_kebab_rst(&f.name) {
            match RECORD_NAME.captures(&f.name) {
                None => out.push(Finding::error(
                    "P003",
                    f,
                    0,
                    format!(
                        "file name `{}` must be `{}-NNNN-<slug>.rst`",
                        f.name,
                        role_kind.lower()
                    ),
                )),
                Some(c) if c[1] != *role_kind.lower() => out.push(Finding::error(
                    "P003",
                    f,
                    0,
                    format!(
                        "`{}` is in the {} folder but is named as {}",
                        f.name,
                        role_kind.upper(),
                        c[1].to_uppercase()
                    ),
                )),
                Some(_) => {}
            }
        }
        if let (Some(t), Some(n)) = (rec.id, rec.file_id)
            && t != n
        {
            let line = f
                .doc
                .as_ref()
                .and_then(|d| d.title())
                .map(|h| h.line)
                .unwrap_or(0);
            out.push(Finding::error(
                "P003",
                f,
                line,
                format!("the file name says {n} but the title says {t}"),
            ));
        }
    }
    let mut groups: BTreeMap<crate::records::RecId, Vec<usize>> = BTreeMap::new();
    for r in &cx.an.records.list {
        if let Some(id) = Records::id_of(r) {
            groups.entry(id).or_default().push(r.file);
        }
    }
    for (id, mut files) in groups {
        if files.len() < 2 {
            continue;
        }
        files.sort_by(|a, b| cx.snap.files[*a].rel.cmp(&cx.snap.files[*b].rel));
        for dup in &files[1..] {
            out.push(Finding::error(
                "P003",
                &cx.snap.files[*dup],
                0,
                format!(
                    "duplicate number {id}; it is also used by {}",
                    cx.snap.files[files[0]].rel
                ),
            ));
        }
    }
    let mut deliverables: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (i, f) in cx.snap.files.iter().enumerate() {
        let Some(doc) = &f.doc else { continue };
        let title = doc.title();
        match f.role {
            Role::Changeset => {
                let name = CHANGESET_NAME.captures(&f.name);
                if name.is_none() && is_kebab_rst(&f.name) {
                    out.push(Finding::error(
                        "P003",
                        f,
                        0,
                        format!(
                            "file name `{}` must be `rfc-NNNN.rst` or `adr-NNNN.rst`",
                            f.name
                        ),
                    ));
                }
                if let (Some(n), Some(t)) = (
                    name,
                    title.and_then(|t| {
                        CHANGESET_TITLE
                            .captures(&t.title.clone())
                            .map(|c| (c[1].to_string(), c[2].to_string()))
                    }),
                ) && (n[1].to_uppercase() != t.0 || n[2] != t.1)
                {
                    out.push(Finding::error(
                        "P003",
                        f,
                        title.map(|h| h.line).unwrap_or(0),
                        format!(
                            "the file name says {}-{} but the title says {}-{}",
                            n[1].to_uppercase(),
                            &n[2],
                            t.0,
                            t.1
                        ),
                    ));
                }
                if title.is_some_and(|t| !CHANGESET_TITLE.is_match(&t.title)) {
                    out.push(Finding::error(
                        "P002",
                        f,
                        title.map(|h| h.line).unwrap_or(0),
                        "title must be `Changeset: RFC-NNNN` or `Changeset: ADR-NNNN`",
                    ));
                }
            }
            Role::Phase(Some(n)) => {
                if let Some(t) = title
                    && let Some(c) = PHASE_TITLE.captures(&t.title)
                    && c[1].parse::<u32>().ok() != Some(n)
                {
                    out.push(Finding::error(
                        "P003",
                        f,
                        t.line,
                        format!("the folder is phase-{n} but the title says Phase {}", &c[1]),
                    ));
                }
            }
            Role::Deliverable(_) => {
                let name = DELIVERABLE_NAME.captures(&f.name);
                if name.is_none() && is_kebab_rst(&f.name) {
                    out.push(Finding::error(
                        "P003",
                        f,
                        0,
                        format!(
                            "file name `{}` must be `deliverable-<N>-<slug>.rst`",
                            f.name
                        ),
                    ));
                }
                if let Some(n) = name.and_then(|c| c[1].parse::<u32>().ok()) {
                    deliverables.entry(n).or_default().push(i);
                    if let Some(t) = title
                        && let Some(c) = DELIVERABLE_TITLE.captures(&t.title)
                        && c[1].parse::<u32>().ok() != Some(n)
                    {
                        out.push(Finding::error("P003", f, t.line, format!("the file name says deliverable {n} but the title says Deliverable {}", &c[1])));
                    }
                }
            }
            _ => {}
        }
    }
    for (n, files) in deliverables {
        for dup in files.iter().skip(1) {
            out.push(Finding::error(
                "P003",
                &cx.snap.files[*dup],
                0,
                format!(
                    "duplicate deliverable number {n}; it is also used by {}",
                    cx.snap.files[files[0]].rel
                ),
            ));
        }
    }
}
