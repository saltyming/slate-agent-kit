//! MCP tool input types.
//!
//! Owns the serde and JSON-schema shape of every tool's input. Arrays, objects and
//! booleans also accept a JSON-encoded string, because some calling agents send them
//! that way. Field names are part of the tool contract and are stable within a major
//! version. Does not validate values (the tools do).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::de::{DeserializeOwned, Error as DeError};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

fn lenient<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let v = Value::deserialize(d)?;
    match v {
        Value::Null => Ok(None),
        Value::String(s) => match serde_json::from_str::<T>(&s) {
            Ok(t) => Ok(Some(t)),
            Err(e) => Err(D::Error::custom(format!(
                "expected a JSON value of the documented shape, got the string {s:?} ({e}); send the value itself, not a JSON-encoded string"
            ))),
        },
        other => serde_json::from_value::<T>(other)
            .map(Some)
            .map_err(|e| D::Error::custom(format!("invalid value: {e}"))),
    }
}

fn lenient_bool<'de, D: Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    let v = Value::deserialize(d)?;
    match v {
        Value::Null => Ok(None),
        Value::Bool(b) => Ok(Some(b)),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" => Ok(Some(true)),
            "false" => Ok(Some(false)),
            _ => Err(D::Error::custom(format!(
                "expected true or false, got {s:?}"
            ))),
        },
        _ => Err(D::Error::custom("expected true or false")),
    }
}

/// Input of `palette_lint`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct LintParams {
    /// Absolute path of the project folder (the one that contains `_palette/`).
    pub project: String,
    /// Optional project-relative files or folders; only findings inside them are returned.
    #[serde(default, deserialize_with = "lenient")]
    pub paths: Option<Vec<String>>,
}

/// Input of `palette_status`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct StatusParams {
    /// Absolute path of the project folder.
    pub project: String,
    /// Optional record identifier such as `RFC-0004`; without it the project summary is returned.
    pub record: Option<String>,
}

/// Input of `palette_layout`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct LayoutParams {
    /// Absolute path of the project folder.
    pub project: String,
}

/// Input of `palette_template`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TemplateParams {
    /// Family or template name: backlog, phase, deliverable, state, rfc, adr, changeset, design, spec, principles, glossary, layout, house-style or rubrics.
    pub family: String,
}

/// Input of `palette_init`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct InitParams {
    /// Absolute path of the project folder.
    pub project: String,
    /// Show the diff without writing.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub dry_run: Option<bool>,
    /// Project name used in document titles; defaults to the folder name.
    pub name: Option<String>,
    /// Placement per family (`internal` or a project-relative path); omitted families are internal.
    #[serde(default, deserialize_with = "lenient")]
    pub placements: Option<BTreeMap<String, String>>,
    /// The `checker` setting: `none` or a project command; defaults to `none`.
    pub checker: Option<String>,
}

/// Input of `palette_layout_set`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct LayoutSetParams {
    /// Absolute path of the project folder.
    pub project: String,
    /// Show the diff without writing.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub dry_run: Option<bool>,
    /// The family to move.
    pub family: String,
    /// `internal` or a project-relative path (a folder, or a `.rst` file for backlog, state, principles and glossary).
    pub placement: String,
    /// Must be true: moving a family changes who can see it, and the user decides that.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub confirmed_by_user: Option<bool>,
}

/// Input of `palette_backlog_add`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct BacklogAddParams {
    /// Absolute path of the project folder.
    pub project: String,
    /// Show the diff without writing.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub dry_run: Option<bool>,
    /// Item title.
    pub title: String,
    /// One of the Type values of the backlog template (feature, bug, refinement, tech-debt, test, spec-gap, rule-gap, chore).
    #[serde(rename = "type")]
    pub item_type: String,
    /// `user`, `phase-<N> close`, or a record or discrepancy identifier.
    pub source: String,
    /// none, high, medium, low, blocked or decision-gate; defaults to none.
    pub priority_signal: Option<String>,
    /// Identifiers of items this one depends on, such as `B-2`.
    #[serde(default, deserialize_with = "lenient")]
    pub depends: Option<Vec<String>>,
    /// What the item is and why it matters, one or two lines.
    pub body: Option<String>,
}

/// Input of `palette_backlog_update`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct BacklogUpdateParams {
    /// Absolute path of the project folder.
    pub project: String,
    /// Show the diff without writing.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub dry_run: Option<bool>,
    /// The item, such as `B-3`.
    pub item: String,
    /// New title.
    pub title: Option<String>,
    /// New type.
    #[serde(rename = "type")]
    pub item_type: Option<String>,
    /// New source.
    pub source: Option<String>,
    /// New priority signal.
    pub priority_signal: Option<String>,
    /// New dependency list (replaces the old one; empty means none).
    #[serde(default, deserialize_with = "lenient")]
    pub depends: Option<Vec<String>>,
    /// New body text.
    pub body: Option<String>,
    /// New status: approved, in-phase-<N>, done or dropped (moves only forward; dropped from any status).
    pub status: Option<String>,
    /// Pointer to the RFC, ADR or changelog entry that records the result.
    pub outcome: Option<String>,
}

/// One phase assumption.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct AssumptionIn {
    /// The assumption.
    pub assumption: String,
    /// What happens if it is wrong.
    pub risk: String,
}

/// Input of `palette_phase_open`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PhaseOpenParams {
    /// Absolute path of the project folder.
    pub project: String,
    /// Show the diff without writing.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub dry_run: Option<bool>,
    /// Phase title.
    pub title: String,
    /// The single outcome the phase exists for.
    pub goal: String,
    /// What makes this phase the next one.
    pub reason: String,
    /// Assumptions, each with the risk if wrong.
    #[serde(default, deserialize_with = "lenient")]
    pub assumptions: Option<Vec<AssumptionIn>>,
    /// Outcomes a person can check when the phase is finished.
    #[serde(default, deserialize_with = "lenient")]
    pub exit_criteria: Option<Vec<String>>,
    /// Approved items to move into the phase, such as `["B-1", "B-2"]`.
    #[serde(default, deserialize_with = "lenient")]
    pub items: Option<Vec<String>>,
}

/// Input of `palette_deliverable_create`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct DeliverableCreateParams {
    /// Absolute path of the project folder.
    pub project: String,
    /// Show the diff without writing.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub dry_run: Option<bool>,
    /// The backlog item, in the active phase.
    pub item: String,
    /// Deliverable title.
    pub title: String,
    /// The problem this deliverable removes and the change that removes it.
    pub what_and_why: String,
    /// Outcomes a person can check, stated against the contract.
    #[serde(default, deserialize_with = "lenient")]
    pub done_when: Option<Vec<String>>,
    /// Explicit boundaries.
    #[serde(default, deserialize_with = "lenient")]
    pub not_this: Option<Vec<String>>,
    /// Pointers the implementer cannot easily find.
    #[serde(default, deserialize_with = "lenient")]
    pub implementation_reference: Option<Vec<String>>,
}

/// Input of `palette_deliverable_update`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct DeliverableUpdateParams {
    /// Absolute path of the project folder.
    pub project: String,
    /// Show the diff without writing.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub dry_run: Option<bool>,
    /// The backlog item whose deliverable changes.
    pub item: String,
    /// New title.
    pub title: Option<String>,
    /// New What and why text.
    pub what_and_why: Option<String>,
    /// New Done when list.
    #[serde(default, deserialize_with = "lenient")]
    pub done_when: Option<Vec<String>>,
    /// New boundaries list.
    #[serde(default, deserialize_with = "lenient")]
    pub not_this: Option<Vec<String>>,
    /// New implementation reference list.
    #[serde(default, deserialize_with = "lenient")]
    pub implementation_reference: Option<Vec<String>>,
}

/// The fate of one item when a phase closes.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CloseItem {
    /// The item, such as `B-3`.
    pub item: String,
    /// `done` or `dropped`.
    pub result: String,
    /// Required for `done`: pointer to the RFC, ADR or changelog entry that records the result.
    pub outcome: Option<String>,
}

/// A new proposed item recorded at phase close.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct NewItem {
    /// Item title.
    pub title: String,
    /// Item type.
    #[serde(rename = "type")]
    pub item_type: String,
    /// Priority signal; defaults to none.
    pub priority_signal: Option<String>,
    /// What it is and why it matters.
    pub body: Option<String>,
}

/// Input of `palette_phase_close`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PhaseCloseParams {
    /// Absolute path of the project folder.
    pub project: String,
    /// Show the diff without writing.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub dry_run: Option<bool>,
    /// The phase number.
    pub phase: u32,
    /// The fate of every item in the phase.
    #[serde(default, deserialize_with = "lenient")]
    pub items: Option<Vec<CloseItem>>,
    /// New proposed items to record.
    #[serde(default, deserialize_with = "lenient")]
    pub new_items: Option<Vec<NewItem>>,
}

/// Input of `palette_state_record`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct StateRecordParams {
    /// Absolute path of the project folder.
    pub project: String,
    /// Show the diff without writing.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub dry_run: Option<bool>,
    /// `decision`, `question` or `discrepancy`.
    pub kind: String,
    /// The decision, the question (without choosing an answer) or the two sources that disagree.
    pub text: String,
    /// Decision: who decided.
    pub source: Option<String>,
    /// Decision: the record, document or file it will be written into.
    pub target: Option<String>,
    /// Question: items or records it affects.
    pub affects: Option<String>,
    /// Question: an optional proposal.
    pub proposal: Option<String>,
    /// Question: who proposed it.
    pub proposal_by: Option<String>,
    /// Discrepancy: `static`, `build` or `runtime`.
    pub evidence: Option<String>,
    /// Discrepancy: the date of the evidence, `YYYY-MM-DD`; defaults to today.
    pub date: Option<String>,
}

/// Input of `palette_state_resolve`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct StateResolveParams {
    /// Absolute path of the project folder.
    pub project: String,
    /// Show the diff without writing.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub dry_run: Option<bool>,
    /// Entry identifier: `D-<n>`, `Q-<n>` or `X-<n>`.
    pub id: String,
    /// `graduated` (decision), `answered` or `withdrawn` (question), `fixed` (discrepancy).
    pub resolution: String,
    /// graduated: the record identifier or project-relative path it was written into.
    pub written: Option<String>,
    /// answered: the decision text.
    pub answer: Option<String>,
    /// answered: who decided.
    pub source: Option<String>,
    /// answered: where the decision will be written.
    pub target: Option<String>,
}

/// A relation entry with its reason.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct RelationIn {
    /// Record identifier such as `RFC-0003`.
    pub record: String,
    /// The parenthetical: what is used, replaced or gathered.
    pub note: String,
    /// `supersedes` only: true replaces a part of the older record (written
    /// `in part: <note>`), which then keeps its status.
    #[serde(default)]
    pub partial: Option<bool>,
}

/// A maintained document touched by a record.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ChangeIn {
    /// `design/<topic>.rst` or `spec/<topic>.rst`.
    pub document: String,
    /// Section titles, or `created` for a new document.
    #[serde(default, deserialize_with = "lenient")]
    pub sections: Option<Vec<String>>,
}

/// Input of `palette_record_create`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct RecordCreateParams {
    /// Absolute path of the project folder.
    pub project: String,
    /// Show the diff without writing.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub dry_run: Option<bool>,
    /// `rfc` or `adr`.
    pub kind: String,
    /// Record title.
    pub title: String,
    /// Optional file name slug; defaults to the kebab-case title.
    pub slug: Option<String>,
    /// The actual authors.
    pub authors: String,
    /// RFC only: affected areas separated by semicolons.
    pub areas: Option<String>,
    /// ADR only: `RFC-<NNNN> (<permitted choice it resolves>)` or `<maintained document path> (<section>)`.
    pub within: Option<String>,
    /// One sentence.
    pub description: String,
    /// Actual reviewers; defaults to `none yet`.
    pub reviewers: Option<String>,
    /// The scope of the implementation in one line; defaults to a generic phrase.
    pub implementation_scope: Option<String>,
    /// Direct uses of earlier records.
    #[serde(default, deserialize_with = "lenient")]
    pub depends: Option<Vec<RelationIn>>,
    /// Older records this one replaces (their status becomes Superseded).
    #[serde(default, deserialize_with = "lenient")]
    pub supersedes: Option<Vec<RelationIn>>,
    /// Context gathered from other records.
    #[serde(default, deserialize_with = "lenient")]
    pub related: Option<Vec<RelationIn>>,
    /// Maintained documents this record changes.
    #[serde(default, deserialize_with = "lenient")]
    pub changes: Option<Vec<ChangeIn>>,
    /// Section title to text; sections not given state `None.`.
    #[serde(default, deserialize_with = "lenient")]
    pub sections: Option<BTreeMap<String, String>>,
}

/// Input of `palette_record_update`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct RecordUpdateParams {
    /// Absolute path of the project folder.
    pub project: String,
    /// Show the diff without writing.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub dry_run: Option<bool>,
    /// The record, such as `RFC-0004`.
    pub record: String,
    /// New title text.
    pub title: Option<String>,
    /// New one-sentence description.
    pub description: Option<String>,
    /// New authors.
    pub authors: Option<String>,
    /// New reviewers.
    pub reviewers: Option<String>,
    /// New areas (RFC).
    pub areas: Option<String>,
    /// New Within value (ADR).
    pub within: Option<String>,
    /// New Depends list (replaces the old one).
    #[serde(default, deserialize_with = "lenient")]
    pub depends: Option<Vec<RelationIn>>,
    /// New Supersedes list; new entries set the older record to Superseded.
    #[serde(default, deserialize_with = "lenient")]
    pub supersedes: Option<Vec<RelationIn>>,
    /// New Related list.
    #[serde(default, deserialize_with = "lenient")]
    pub related: Option<Vec<RelationIn>>,
    /// New Changes list.
    #[serde(default, deserialize_with = "lenient")]
    pub changes: Option<Vec<ChangeIn>>,
    /// Section title to new text; replaces that section's content.
    #[serde(default, deserialize_with = "lenient")]
    pub sections: Option<BTreeMap<String, String>>,
    /// New status: Proposed, Accepted (needs `accepted_by`), Rejected or Withdrawn.
    pub status: Option<String>,
    /// Who accepted the record; required when moving to Accepted.
    pub accepted_by: Option<String>,
    /// True for a mechanical correction of an accepted record; adds a Revised entry.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub clarification: Option<bool>,
    /// The Revised entry text (one line) for a clarification.
    pub revision_note: Option<String>,
}

/// Input of `palette_changeset_edit`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ChangesetEditParams {
    /// Absolute path of the project folder.
    pub project: String,
    /// Show the diff without writing.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub dry_run: Option<bool>,
    /// The record whose changeset changes, such as `RFC-0004`.
    pub record: String,
    /// `add`, `replace` or `remove`.
    pub action: String,
    /// `replace`, `insert_after`, `insert_into`, `delete` or `create`.
    pub kind: String,
    /// The maintained document: `design/<topic>.rst` or `spec/<topic>.rst`.
    pub document: String,
    /// The section title (prefixed by parent titles and " / " when not unique); for `create`, the new document's title.
    pub target: String,
    /// The edit body in changeset syntax: the new section title underlined with `^`, then its text; subsections use `"`. For `create`, the header fields, then top-level sections underlined with `^`. Not used by `delete` or `remove`.
    pub body: Option<String>,
}

/// One edit named by kind, document and target.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct EditRef {
    /// `replace`, `insert_after`, `insert_into`, `delete` or `create`.
    pub kind: String,
    /// The maintained document.
    pub document: String,
    /// The edit's target as written in the changeset.
    pub target: String,
}

/// Input of `palette_changeset_promote`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ChangesetPromoteParams {
    /// Absolute path of the project folder.
    pub project: String,
    /// Show the diff without writing.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub dry_run: Option<bool>,
    /// The accepted record, such as `RFC-0004`.
    pub record: String,
    /// The implemented edits; omit to promote every edit.
    #[serde(default, deserialize_with = "lenient")]
    pub edits: Option<Vec<EditRef>>,
    /// `partial` or `complete` (complete requires every edit promoted).
    pub implementation: String,
    /// The scope in one line; defaults to the current scope text.
    pub implementation_scope: Option<String>,
    /// Who implemented it.
    pub implementers: String,
    /// none, documentation, static, build or runtime.
    pub verification: String,
    /// The limit of the verification, in one line.
    pub verification_note: String,
}
