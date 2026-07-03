In Claude Code, `workslate_task_*` is that tracker, with namespaces per context:

| Context | Task tool | Why |
|---------|-----------|-----|
| Solo work | `workslate_task_*` (`ws:` namespace) | Footer auto-display, named sessions, disk persistence |
| Team leader | `workslate_task_*` (`ws:` own phases, `team:` task graph) | Unified tracking — footer shows both namespaces |
| Teammate | `workslate_task_*` (`team:` namespace) | Same SQLite DB, concurrent via WAL, self-claim via `workslate_task_update(owner=self)` |

The built-in `TaskCreate` / `TaskList` / `TaskUpdate` tools also exist under the single implicit team, but `workslate_task_*` is the system of record here — only it is surfaced by the doorbell footer on every tool call and shared cross-session via the DB. Do not split coordination across both.
