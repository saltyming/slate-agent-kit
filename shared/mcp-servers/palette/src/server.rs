//! The MCP server: tool registration, descriptions and result formatting.
//!
//! Owns the tool list (names, descriptions, annotations), running each tool on a
//! blocking thread, and the mapping of errors to `error.code` results. The tool logic
//! is in `tools` and `ops`. Entry points: [`PaletteServer`], [`read_only_tool_names`].

use rmcp::{
    RoleServer, ServerHandler,
    handler::server::{router::tool::ToolRouter, tool::ToolCallContext, wrapper::Parameters},
    model::{
        CallToolRequestParams, CallToolResult, Content, ListToolsResult, PaginatedRequestParams,
        ServerCapabilities, ServerInfo, Tool,
    },
    service::RequestContext,
    tool, tool_router,
};
use serde_json::json;

use crate::errors::{PalError, Res};
use crate::ops::{self, Ctx};
use crate::params::*;
use crate::tools;

/// The palette MCP server.
#[derive(Clone)]
pub struct PaletteServer {
    ctx: Ctx,
    tool_router: ToolRouter<Self>,
}

impl PaletteServer {
    /// A server with the given settings.
    pub fn new(ctx: Ctx) -> PaletteServer {
        PaletteServer {
            ctx,
            tool_router: Self::tool_router(),
        }
    }
}

fn ok_text(text: String) -> CallToolResult {
    CallToolResult::success(vec![Content::text(text)])
}

/// A structured error result carrying a stable `code` and a message that says how to fix it.
pub fn err_result(e: &PalError) -> CallToolResult {
    let body = serde_json::to_string_pretty(&json!({
        "error": { "code": e.code.as_str(), "message": e.message }
    }))
    .unwrap_or_else(|_| e.message.clone());
    CallToolResult::error(vec![Content::text(body)])
}

async fn blocking<F>(f: F) -> CallToolResult
where
    F: FnOnce() -> Res<String> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(text)) => ok_text(text),
        Ok(Err(e)) => err_result(&e),
        Err(join) => err_result(&PalError::new(
            crate::errors::ErrCode::IoError,
            format!("the tool stopped unexpectedly: {join}"),
        )),
    }
}

#[tool_router]
impl PaletteServer {
    #[tool(
        description = "Checks the palette documents of one project against the house style and the templates and returns the findings, errors first. Input: `project` (absolute path of the project folder; without _palette/layout.rst the families are found from the documents themselves), optional `paths` (project-relative files or folders; only findings inside them are returned, but every rule still runs on the whole project so cross-file rules stay correct). Output: JSON with `errors`, `warnings` and `findings`, each finding having `rule` (P001 to P015), `severity` (error or warning), `file`, `line` and `message`. Changes nothing. Refuses a project outside the server's roots and a project without _palette/layout.rst.",
        annotations(read_only_hint = true)
    )]
    async fn palette_lint(
        &self,
        Parameters(p): Parameters<LintParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ctx = self.ctx.clone();
        Ok(blocking(move || tools::lint(&ctx, p)).await)
    }

    #[tool(
        description = "Summarizes a palette project or one record in under 4,000 characters, and says what it omitted. Input: `project`, optional `record` (such as RFC-0004). Without `record`: phases and their status, item counts by status, open questions, discrepancies, decisions not yet graduated to a record, and the lint error count. With `record`: its header, the records that link to it, its dependency closure, what supersedes it, its pending changeset edits and the staging documents they affect. Changes nothing. Refuses an unknown record (`not_found`).",
        annotations(read_only_hint = true)
    )]
    async fn palette_status(
        &self,
        Parameters(p): Parameters<StatusParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ctx = self.ctx.clone();
        Ok(blocking(move || tools::status(&ctx, p)).await)
    }

    #[tool(
        description = "Returns where each of the twelve document families lives in one project: its placement (`internal` or a project path) and its resolved absolute path, plus the `checker` setting and any problems in layout.rst. Input: `project`. Changes nothing. Refuses a project without _palette/layout.rst (`no_layout`).",
        annotations(read_only_hint = true)
    )]
    async fn palette_layout(
        &self,
        Parameters(p): Parameters<LayoutParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ctx = self.ctx.clone();
        Ok(blocking(move || tools::layout(&ctx, p)).await)
    }

    #[tool(
        description = "Returns the text of a document template embedded in this server, which is the schema the lint checks against (a value written `a | b | c` allows exactly those tokens; `<...>` is text the author supplies; a value followed by ` — <...>` takes free text after the dash). Input: `family`, one of backlog, phase, deliverable, state, rfc, adr, changeset, design, spec, principles, glossary, layout, house-style, rubrics. Needs no project. Changes nothing. Refuses `staging` (generated, no template) and unknown names.",
        annotations(read_only_hint = true)
    )]
    async fn palette_template(
        &self,
        Parameters(p): Parameters<TemplateParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        Ok(blocking(move || tools::template(p)).await)
    }

    #[tool(
        description = "Sets a project up for palette: creates _palette/, _palette/.gitignore (containing `*`), layout.rst from the given placements, and empty backlog and state documents at their resolved locations (an existing backlog or state file is kept). Input: `project`, optional `name` (default: the folder name), `placements` (family to `internal` or a project-relative path; omitted families are internal), `checker` (default none), `dry_run`. Fails with `already_initialized` when a layout exists. Runs as one transaction: does not take the project lock (the folder does not exist yet); parses every file it will change and fails with `parse_error` (naming the file and line) without changing anything when one does not parse; changes only the lines the operation concerns and keeps each file's line endings; regenerates the record and changeset indexes and the staging documents it affects; lints the files it touched and fails with `invariant_violation`, changing nothing, if the result would contain a lint error; then writes every changed file through a temporary sibling and a rename, and restores every file if any step fails (`conflict` if a file changed after it was read). With dry_run true it returns the same diff and writes nothing. Returns JSON with the unified diff of every file it changed (or would change), the file list and the identifiers it allocated. Refuses a project outside the server's project root and extra roots (`outside_roots`), a project without _palette/layout.rst (`no_layout`), and any path that contains `..` or a symbolic link.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn palette_init(
        &self,
        Parameters(p): Parameters<InitParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ctx = self.ctx.clone();
        Ok(blocking(move || ops::init::init(&ctx, p).map(|r| r.to_json())).await)
    }

    #[tool(
        description = "Moves one document family to a new placement: moves its files, rewrites every link to them and every relative link inside them, and updates layout.rst. Input: `project`, `family`, `placement` (`internal` or a project-relative folder; a `.rst` file for backlog, state, principles and glossary), `confirmed_by_user` (must be true: the move changes who can see the documents, so ask the user first), `dry_run`. Only palette documents are rewritten; links from other files (a README, source code) are not. Refuses without confirmed_by_user, a destination that already holds a file, and a placement outside the project or inside _palette/. Runs as one transaction: takes the per-project lock (waits at most 10 seconds, else fails with `locked`); parses every file it will change and fails with `parse_error` (naming the file and line) without changing anything when one does not parse; changes only the lines the operation concerns and keeps each file's line endings; regenerates the record and changeset indexes and the staging documents it affects; lints the files it touched and fails with `invariant_violation`, changing nothing, if the result would contain a lint error; then writes every changed file through a temporary sibling and a rename, and restores every file if any step fails (`conflict` if a file changed after it was read). With dry_run true it returns the same diff and writes nothing. Returns JSON with the unified diff of every file it changed (or would change), the file list and the identifiers it allocated. Refuses a project outside the server's project root and extra roots (`outside_roots`), a project without _palette/layout.rst (`no_layout`), and any path that contains `..` or a symbolic link.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn palette_layout_set(
        &self,
        Parameters(p): Parameters<LayoutSetParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ctx = self.ctx.clone();
        Ok(blocking(move || ops::init::layout_set(&ctx, p).map(|r| r.to_json())).await)
    }

    #[tool(
        description = "Adds a backlog item with status `proposed` and returns its identifier `B-<n>`. Input: `project`, `title`, `type` (feature, bug, refinement, tech-debt, test, spec-gap, rule-gap, chore), `source` (`user`, `phase-<N> close`, or a record or discrepancy identifier), optional `priority_signal` (none, high, medium, low, blocked, decision-gate; default none), `depends` (item identifiers), `body` (one or two lines: what it is and why it matters), `dry_run`. Refuses values outside the template's allowed values and dependencies on items that do not exist. Runs as one transaction: takes the per-project lock (waits at most 10 seconds, else fails with `locked`); parses every file it will change and fails with `parse_error` (naming the file and line) without changing anything when one does not parse; changes only the lines the operation concerns and keeps each file's line endings; regenerates the record and changeset indexes and the staging documents it affects; lints the files it touched and fails with `invariant_violation`, changing nothing, if the result would contain a lint error; then writes every changed file through a temporary sibling and a rename, and restores every file if any step fails (`conflict` if a file changed after it was read). With dry_run true it returns the same diff and writes nothing. Returns JSON with the unified diff of every file it changed (or would change), the file list and the identifiers it allocated. Refuses a project outside the server's project root and extra roots (`outside_roots`), a project without _palette/layout.rst (`no_layout`), and any path that contains `..` or a symbolic link.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn palette_backlog_add(
        &self,
        Parameters(p): Parameters<BacklogAddParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ctx = self.ctx.clone();
        Ok(blocking(move || ops::backlog::backlog_add(&ctx, p).map(|r| r.to_json())).await)
    }

    #[tool(
        description = "Changes fields of one backlog item. Input: `project`, `item` (B-<n>), any of `title`, `type`, `source`, `priority_signal`, `depends` (replaces the list), `body`, `status`, `outcome`, `dry_run`. Status moves only proposed → approved → in-phase-<N> → done, or to dropped from any status; in-phase-<N> requires phase <N> to be active. Refuses any other status move (`invariant_violation`). Item status is recorded only here. Runs as one transaction: takes the per-project lock (waits at most 10 seconds, else fails with `locked`); parses every file it will change and fails with `parse_error` (naming the file and line) without changing anything when one does not parse; changes only the lines the operation concerns and keeps each file's line endings; regenerates the record and changeset indexes and the staging documents it affects; lints the files it touched and fails with `invariant_violation`, changing nothing, if the result would contain a lint error; then writes every changed file through a temporary sibling and a rename, and restores every file if any step fails (`conflict` if a file changed after it was read). With dry_run true it returns the same diff and writes nothing. Returns JSON with the unified diff of every file it changed (or would change), the file list and the identifiers it allocated. Refuses a project outside the server's project root and extra roots (`outside_roots`), a project without _palette/layout.rst (`no_layout`), and any path that contains `..` or a symbolic link.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn palette_backlog_update(
        &self,
        Parameters(p): Parameters<BacklogUpdateParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ctx = self.ctx.clone();
        Ok(blocking(move || ops::backlog::backlog_update(&ctx, p).map(|r| r.to_json())).await)
    }

    #[tool(
        description = "Opens the next phase: creates phase-<N>/phase.rst from the goal, reason, assumptions and exit criteria, adds the phase to the backlog as `active`, and moves the given approved items to in-phase-<N>. Input: `project`, `title`, `goal`, `reason`, `exit_criteria` (at least one outcome a person can check), optional `assumptions` (each with its risk if wrong) and `items`, `dry_run`. Returns `phase-<N>`. Fails when another phase is active and when an item is not approved. Runs as one transaction: takes the per-project lock (waits at most 10 seconds, else fails with `locked`); parses every file it will change and fails with `parse_error` (naming the file and line) without changing anything when one does not parse; changes only the lines the operation concerns and keeps each file's line endings; regenerates the record and changeset indexes and the staging documents it affects; lints the files it touched and fails with `invariant_violation`, changing nothing, if the result would contain a lint error; then writes every changed file through a temporary sibling and a rename, and restores every file if any step fails (`conflict` if a file changed after it was read). With dry_run true it returns the same diff and writes nothing. Returns JSON with the unified diff of every file it changed (or would change), the file list and the identifiers it allocated. Refuses a project outside the server's project root and extra roots (`outside_roots`), a project without _palette/layout.rst (`no_layout`), and any path that contains `..` or a symbolic link.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn palette_phase_open(
        &self,
        Parameters(p): Parameters<PhaseOpenParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ctx = self.ctx.clone();
        Ok(blocking(move || ops::backlog::phase_open(&ctx, p).map(|r| r.to_json())).await)
    }

    #[tool(
        description = "Writes the deliverable of one backlog item in the active phase and links it from the item. Input: `project`, `item` (in the active phase), `title`, `what_and_why`, `done_when` (at least one outcome a person can check, stated against the contract), optional `not_this` and `implementation_reference`, `dry_run`. The file is deliverable-<n>-<slug>.rst where <n> is the item number. Refuses an item that already has a deliverable and an item outside the active phase. Runs as one transaction: takes the per-project lock (waits at most 10 seconds, else fails with `locked`); parses every file it will change and fails with `parse_error` (naming the file and line) without changing anything when one does not parse; changes only the lines the operation concerns and keeps each file's line endings; regenerates the record and changeset indexes and the staging documents it affects; lints the files it touched and fails with `invariant_violation`, changing nothing, if the result would contain a lint error; then writes every changed file through a temporary sibling and a rename, and restores every file if any step fails (`conflict` if a file changed after it was read). With dry_run true it returns the same diff and writes nothing. Returns JSON with the unified diff of every file it changed (or would change), the file list and the identifiers it allocated. Refuses a project outside the server's project root and extra roots (`outside_roots`), a project without _palette/layout.rst (`no_layout`), and any path that contains `..` or a symbolic link.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn palette_deliverable_create(
        &self,
        Parameters(p): Parameters<DeliverableCreateParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ctx = self.ctx.clone();
        Ok(blocking(move || ops::backlog::deliverable_create(&ctx, p).map(|r| r.to_json())).await)
    }

    #[tool(
        description = "Changes sections of an existing deliverable of the active phase; sections not given are kept. Input: `project`, `item`, any of `title`, `what_and_why`, `done_when`, `not_this`, `implementation_reference`, `dry_run`. Refuses an item without a deliverable and an item outside the active phase. Runs as one transaction: takes the per-project lock (waits at most 10 seconds, else fails with `locked`); parses every file it will change and fails with `parse_error` (naming the file and line) without changing anything when one does not parse; changes only the lines the operation concerns and keeps each file's line endings; regenerates the record and changeset indexes and the staging documents it affects; lints the files it touched and fails with `invariant_violation`, changing nothing, if the result would contain a lint error; then writes every changed file through a temporary sibling and a rename, and restores every file if any step fails (`conflict` if a file changed after it was read). With dry_run true it returns the same diff and writes nothing. Returns JSON with the unified diff of every file it changed (or would change), the file list and the identifiers it allocated. Refuses a project outside the server's project root and extra roots (`outside_roots`), a project without _palette/layout.rst (`no_layout`), and any path that contains `..` or a symbolic link.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn palette_deliverable_update(
        &self,
        Parameters(p): Parameters<DeliverableUpdateParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ctx = self.ctx.clone();
        Ok(blocking(move || ops::backlog::deliverable_update(&ctx, p).map(|r| r.to_json())).await)
    }

    #[tool(
        description = "Closes an active phase: gives every item in it a result (`done` with an outcome pointer, or `dropped`), records new proposed items, marks the phase `closed` in the backlog, removes the state pointers of graduated decisions, and deletes the phase file and its deliverable files (item deliverable links become none). This deletes files. Input: `project`, `phase`, `items` (item, result, outcome), optional `new_items` (title, type, priority_signal, body), `dry_run`. Returns the identifiers of the new items. Refuses a phase that is not active and any item of the phase without a result. Runs as one transaction: takes the per-project lock (waits at most 10 seconds, else fails with `locked`); parses every file it will change and fails with `parse_error` (naming the file and line) without changing anything when one does not parse; changes only the lines the operation concerns and keeps each file's line endings; regenerates the record and changeset indexes and the staging documents it affects; lints the files it touched and fails with `invariant_violation`, changing nothing, if the result would contain a lint error; then writes every changed file through a temporary sibling and a rename, and restores every file if any step fails (`conflict` if a file changed after it was read). With dry_run true it returns the same diff and writes nothing. Returns JSON with the unified diff of every file it changed (or would change), the file list and the identifiers it allocated. Refuses a project outside the server's project root and extra roots (`outside_roots`), a project without _palette/layout.rst (`no_layout`), and any path that contains `..` or a symbolic link.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn palette_phase_close(
        &self,
        Parameters(p): Parameters<PhaseCloseParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ctx = self.ctx.clone();
        Ok(blocking(move || ops::backlog::phase_close(&ctx, p).map(|r| r.to_json())).await)
    }

    #[tool(
        description = "Adds an entry to the state document and stamps its Updated date: a decision (`text`, `source` who decided, `target` where it will be written), an open question (`text`, `affects`, optional `proposal` with `proposal_by`) or a discrepancy (`text`, `evidence` static, build or runtime, optional `date`). Input also has `project`, `kind` (decision, question, discrepancy), `dry_run`. Returns D-<n>, Q-<n> or X-<n>. One entry is one bullet; keep it to one line of substance. Runs as one transaction: takes the per-project lock (waits at most 10 seconds, else fails with `locked`); parses every file it will change and fails with `parse_error` (naming the file and line) without changing anything when one does not parse; changes only the lines the operation concerns and keeps each file's line endings; regenerates the record and changeset indexes and the staging documents it affects; lints the files it touched and fails with `invariant_violation`, changing nothing, if the result would contain a lint error; then writes every changed file through a temporary sibling and a rename, and restores every file if any step fails (`conflict` if a file changed after it was read). With dry_run true it returns the same diff and writes nothing. Returns JSON with the unified diff of every file it changed (or would change), the file list and the identifiers it allocated. Refuses a project outside the server's project root and extra roots (`outside_roots`), a project without _palette/layout.rst (`no_layout`), and any path that contains `..` or a symbolic link.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn palette_state_record(
        &self,
        Parameters(p): Parameters<StateRecordParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ctx = self.ctx.clone();
        Ok(blocking(move || ops::state::state_record(&ctx, p).map(|r| r.to_json())).await)
    }

    #[tool(
        description = "Resolves a state entry. A decision `graduated` (with `written`: the record identifier or project path it was written into) becomes a one-line pointer; a question `answered` (with `answer`, `source`, `target`) becomes a new decision and returns its D-<n>; a question `withdrawn` or a discrepancy `fixed` is removed. Input also has `project`, `id` (D-<n>, Q-<n> or X-<n>), `dry_run`. Refuses a resolution that does not fit the entry kind and a `written` target that does not exist. Runs as one transaction: takes the per-project lock (waits at most 10 seconds, else fails with `locked`); parses every file it will change and fails with `parse_error` (naming the file and line) without changing anything when one does not parse; changes only the lines the operation concerns and keeps each file's line endings; regenerates the record and changeset indexes and the staging documents it affects; lints the files it touched and fails with `invariant_violation`, changing nothing, if the result would contain a lint error; then writes every changed file through a temporary sibling and a rename, and restores every file if any step fails (`conflict` if a file changed after it was read). With dry_run true it returns the same diff and writes nothing. Returns JSON with the unified diff of every file it changed (or would change), the file list and the identifiers it allocated. Refuses a project outside the server's project root and extra roots (`outside_roots`), a project without _palette/layout.rst (`no_layout`), and any path that contains `..` or a symbolic link.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn palette_state_resolve(
        &self,
        Parameters(p): Parameters<StateResolveParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ctx = self.ctx.clone();
        Ok(blocking(move || ops::state::state_resolve(&ctx, p).map(|r| r.to_json())).await)
    }

    #[tool(
        description = "Creates the next RFC or ADR from its template and returns its identifier. Input: `project`, `kind` (rfc or adr), `title`, `authors`, `description` (one sentence), `areas` (RFC) or `within` (ADR), optional `slug`, `reviewers`, `implementation_scope`, `depends`, `supersedes` (each older record's status becomes Superseded), `related`, `changes` (maintained documents and sections), `sections` (section title to text; sections not given state `None.`), `dry_run`. The record starts as Draft with the time-varying fields (Implementation, Verification, Implementers, Revised) at their initial values; only this server writes them. Refuses an unknown section title and a supersession of a record that is not Accepted. Runs as one transaction: takes the per-project lock (waits at most 10 seconds, else fails with `locked`); parses every file it will change and fails with `parse_error` (naming the file and line) without changing anything when one does not parse; changes only the lines the operation concerns and keeps each file's line endings; regenerates the record and changeset indexes and the staging documents it affects; lints the files it touched and fails with `invariant_violation`, changing nothing, if the result would contain a lint error; then writes every changed file through a temporary sibling and a rename, and restores every file if any step fails (`conflict` if a file changed after it was read). With dry_run true it returns the same diff and writes nothing. Returns JSON with the unified diff of every file it changed (or would change), the file list and the identifiers it allocated. Refuses a project outside the server's project root and extra roots (`outside_roots`), a project without _palette/layout.rst (`no_layout`), and any path that contains `..` or a symbolic link.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn palette_record_create(
        &self,
        Parameters(p): Parameters<RecordCreateParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ctx = self.ctx.clone();
        Ok(blocking(move || ops::records::record_create(&ctx, p).map(|r| r.to_json())).await)
    }

    #[tool(
        description = "Changes a record. A Draft or Proposed record may change freely. An Accepted (or later) record's body changes only with `clarification: true` (a mechanical correction) and a `revision_note`, which adds a Revised entry; a change of substance needs a new record that supersedes it. Status moves Draft → Proposed → Accepted (needs `accepted_by`), or to Rejected or Withdrawn; Superseded is set only through a newer record's Supersedes. Input: `project`, `record`, any of `title`, `description`, `authors`, `reviewers`, `areas`, `within`, `depends`, `supersedes`, `related`, `changes`, `sections`, `status`, `accepted_by`, `clarification`, `revision_note`, `dry_run`. Runs as one transaction: takes the per-project lock (waits at most 10 seconds, else fails with `locked`); parses every file it will change and fails with `parse_error` (naming the file and line) without changing anything when one does not parse; changes only the lines the operation concerns and keeps each file's line endings; regenerates the record and changeset indexes and the staging documents it affects; lints the files it touched and fails with `invariant_violation`, changing nothing, if the result would contain a lint error; then writes every changed file through a temporary sibling and a rename, and restores every file if any step fails (`conflict` if a file changed after it was read). With dry_run true it returns the same diff and writes nothing. Returns JSON with the unified diff of every file it changed (or would change), the file list and the identifiers it allocated. Refuses a project outside the server's project root and extra roots (`outside_roots`), a project without _palette/layout.rst (`no_layout`), and any path that contains `..` or a symbolic link.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn palette_record_update(
        &self,
        Parameters(p): Parameters<RecordUpdateParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ctx = self.ctx.clone();
        Ok(blocking(move || ops::records::record_update(&ctx, p).map(|r| r.to_json())).await)
    }

    #[tool(
        description = "Adds, replaces or removes one edit in a record's changeset (edits to maintained design and spec documents that the source does not implement yet); creates the changeset file with the first edit and deletes it when the last edit is removed. Input: `project`, `record`, `action` (add, replace, remove), `kind` (replace, insert_after, insert_into, delete, create), `document` (design/<topic>.rst or spec/<topic>.rst), `target` (section title, prefixed by parent titles and \" / \" when not unique; for create, the new document's title), `body` (the new section title underlined with ^, then its text; subsections use \"; for create, the header fields then sections underlined with ^), `dry_run`. Refuses an edit that does not resolve against its target with the record's dependency closure applied, and two independent changesets editing one section. Runs as one transaction: takes the per-project lock (waits at most 10 seconds, else fails with `locked`); parses every file it will change and fails with `parse_error` (naming the file and line) without changing anything when one does not parse; changes only the lines the operation concerns and keeps each file's line endings; regenerates the record and changeset indexes and the staging documents it affects; lints the files it touched and fails with `invariant_violation`, changing nothing, if the result would contain a lint error; then writes every changed file through a temporary sibling and a rename, and restores every file if any step fails (`conflict` if a file changed after it was read). With dry_run true it returns the same diff and writes nothing. Returns JSON with the unified diff of every file it changed (or would change), the file list and the identifiers it allocated. Refuses a project outside the server's project root and extra roots (`outside_roots`), a project without _palette/layout.rst (`no_layout`), and any path that contains `..` or a symbolic link.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn palette_changeset_edit(
        &self,
        Parameters(p): Parameters<ChangesetEditParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ctx = self.ctx.clone();
        Ok(blocking(move || ops::records::changeset_edit(&ctx, p).map(|r| r.to_json())).await)
    }

    #[tool(
        description = "Merges implemented edits of an Accepted record into their maintained documents, removes them from the changeset (deleting an empty changeset), and sets the record's Implementation (`partial` or `complete`), Implementers and Verification from the given values. Input: `project`, `record`, optional `edits` (kind, document, target; omit to promote all), `implementation`, optional `implementation_scope`, `implementers`, `verification` (none, documentation, static, build, runtime), `verification_note` (the limit of the check), `dry_run`. `complete` requires every edit promoted. Refuses when a record this one depends on still has edits to the same document, and when an edit does not resolve. Runs as one transaction: takes the per-project lock (waits at most 10 seconds, else fails with `locked`); parses every file it will change and fails with `parse_error` (naming the file and line) without changing anything when one does not parse; changes only the lines the operation concerns and keeps each file's line endings; regenerates the record and changeset indexes and the staging documents it affects; lints the files it touched and fails with `invariant_violation`, changing nothing, if the result would contain a lint error; then writes every changed file through a temporary sibling and a rename, and restores every file if any step fails (`conflict` if a file changed after it was read). With dry_run true it returns the same diff and writes nothing. Returns JSON with the unified diff of every file it changed (or would change), the file list and the identifiers it allocated. Refuses a project outside the server's project root and extra roots (`outside_roots`), a project without _palette/layout.rst (`no_layout`), and any path that contains `..` or a symbolic link.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn palette_changeset_promote(
        &self,
        Parameters(p): Parameters<ChangesetPromoteParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let ctx = self.ctx.clone();
        Ok(blocking(move || ops::records::changeset_promote(&ctx, p).map(|r| r.to_json())).await)
    }
}

impl ServerHandler for PaletteServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Reads, checks and writes the palette documents of one project. Every tool takes `project`, the absolute path of the project folder (the write tools and palette_status need its _palette/layout.rst; palette_lint also works on a checkout without _palette/, finding the families from the documents themselves); it must lie inside the server's project root or an extra root (PALETTE_EXTRA_ROOTS). \
             Read tools (palette_lint, palette_status, palette_layout, palette_template) change nothing and are safe to pre-approve. Write tools change several files as one transaction: all files change or none, only the lines the operation concerns are touched, line endings are kept, and the result is linted first; use dry_run to see the diff. \
             The server owns identifiers, item status, links, sections, budgets, record relations, changesets, staging and indexes; you supply the prose through typed fields. Only this server writes the Implementation, Verification, Implementers and Revised fields, the generated indexes and the staging documents. Hand edits to other text are allowed; the next palette_lint reports what drifted. \
             Errors carry a stable `error.code`: invalid_params, no_project_root, outside_roots, no_layout, already_initialized, not_found, parse_error, locked, invariant_violation, conflict, io_error. A document under a project path never links into _palette/, and only the user's approval authorizes execution, wherever a document lives.",
        )
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let tcc = ToolCallContext::new(self, request, context);
        self.tool_router.call(tcc).await
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        Ok(ListToolsResult {
            tools: self.tool_router.list_all(),
            meta: None,
            next_cursor: None,
        })
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tool_router.get(name).cloned()
    }
}

/// The tools marked read-only, in registration order; the source of `palette --read-only-tools`.
pub fn read_only_tool_names() -> Vec<String> {
    PaletteServer::tool_router()
        .list_all()
        .into_iter()
        .filter(|t| t.annotations.as_ref().and_then(|a| a.read_only_hint) == Some(true))
        .map(|t| t.name.to_string())
        .collect()
}

/// All tools with their annotations, for tests: `(name, read_only, destructive)`.
pub fn tool_annotations() -> Vec<(String, Option<bool>, Option<bool>)> {
    PaletteServer::tool_router()
        .list_all()
        .into_iter()
        .map(|t| {
            let a = t.annotations.as_ref();
            (
                t.name.to_string(),
                a.and_then(|x| x.read_only_hint),
                a.and_then(|x| x.destructive_hint),
            )
        })
        .collect()
}
