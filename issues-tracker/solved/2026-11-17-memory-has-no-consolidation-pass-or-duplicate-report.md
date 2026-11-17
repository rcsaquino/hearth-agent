# Memory has no consolidation pass and no duplicate report

| | |
| --- | --- |
| Kind | feature |
| Reported | 2026-11-17 |
| Area | `hearth-memory` |

## Observed

- `hearth-memory` has no consolidation API. `rg consolidate` finds metadata,
  docs, and tests, but no implementation. `last_consolidation` in
  `hearth-runtime/src/ipc.rs` is read and exposed, but no production path writes
  it.
- `ensure_maintenance_tasks` in `crates/hearth-runtime/src/maintenance.rs`
  schedules backup, skill review, and helper retention. The test
  `the_maintenance_tasks_are_created_once` in
  `crates/hearth-runtime/tests/s7_maintenance.rs` asserts that no
  `memory.consolidate` task exists.
- `StatusContext` exposes `last_consolidation`, but nothing writes it.
- The Memory tab lists current and retired rows, but has no duplicate marker
  and no candidate list.
- `store::related` reports a neighbour only after a created `memory_save`.
  The extractor records `written` and `skipped` in `CaptureReport`, and the
  daemon logs only `written`.
- `store::forget` deletes a row, its vector, and its index entry. Closed rows
  remain forever. No pass closes accidental duplicates.

## Wanted

A deterministic audit and repair path:

- Scan current rows and report four groups: canonical duplicate groups,
  near-duplicate pairs, same-content rows with different non-empty subjects,
  and compound rows that carry more than one statement.
- Close canonical duplicates automatically. Keep one survivor, set `valid_to`
  and `superseded_by` on the others. Do not delete. Do not rewrite content.
- Report near duplicates and compound rows. Do not close them automatically.
- Expose the report through `hearth status`, the Memory tab, and the activity
  log. The agent must be able to see the counts.
- Run without a user request: bounded at startup, nightly as
  `memory.consolidate`, or both. Write `last_consolidation` and the report.
- Collapse duplicate canonical keys in the digest and say how many rows were
  hidden, so the model does not spend tokens on the same statement twice
  before the repair pass runs.

The pass must be idempotent, bounded, and restart-safe. A crash must leave each
row either open or closed, never half-closed.

## Why

Without a pass, an accidental duplicate that slips past the write check stays
live indefinitely. There is no age-based deletion. The digest shows a ranked
subset, so the agent may never see both rows. The user has to know to ask, and
the Memory tab cannot show which rows are candidates. Exact duplicates are safe
to close automatically; near duplicates need a decision, but they must at least
be visible.

## Scope

- No model call.
- No hard deletion. `memory_forget` remains the only delete path.
- No content rewrite.
- Do not change the current write rules or retrieval ranking.
- Do not replace
  `open/2026-11-17-the-memory-tab-shows-one-total-and-no-breakdown.md`. That
  issue asks for the pinned, active, and retired totals. This issue asks for
  duplicate candidates.

## Test

- A store with two canonical duplicates yields one current row and one closed
  row with `superseded_by` set.
- A store with two near duplicates changes neither row and reports a candidate.
- Two compound rows are reported, not split.
- A second run changes nothing.
- A crash or restart during the pass leaves no row half-closed.
- `hearth status` and the Memory tab show the same counts.
- The digest hides a duplicate and reports the hidden count.
- A duplicate created before the pass is repaired by the next run.

## Not established

- Startup pass, nightly task, or both.
- The threshold and report format for near duplicates.
- Whether the Memory tab can merge candidates or only display them.
- Whether `last_consolidation` should be a task checkpoint, a meta key, or both.
- Whether old closed rows should have a retention policy. This issue does not
  propose deletion.


## Resolution

`hearth-memory` now has a deterministic audit in `src/consolidate.rs`, exposed as
`MemoryService::consolidate`.

- Canonical means the same fingerprint under the same subject. The survivor is
  the pinned row first, then the higher `Kind::prior`, then the newest update,
  then the smaller id. Every other row in the group gets `valid_to`,
  `superseded_by`, and a cleared pin. Nothing is deleted and no content changes.
- Near-duplicate pairs, one content under different non-empty subjects, and
  compound rows are reported and left current.
- The pass is one write transaction. The report and its `last_consolidation`
  instant are written in that transaction as the meta keys
  `last_consolidation` and `last_consolidation_report`. A crash rolls the whole
  pass back.
- `memory.consolidate` is a nightly task at 03:30 local, registered in both the
  Telegram daemon and the maintenance-only scheduler. The handler publishes a
  `memory.consolidated` activity event with the counts.
- The digest collapses a canonical duplicate and reports the hidden count, so the
  prompt does not pay for the statement twice before the pass runs.
- The status payload gains `memory.pinned`, `memory.active`, `memory.retired`,
  and `memory.audit`. The Memory tab shows the same totals and an audit line, and
  marks the sampled candidates with `⚠`.

The pass is bounded: canonical groups are found over the whole store with a
grouped query, while near and compound reporting reads the newest 10,000 current
rows and keeps at most 20 examples per group. There is no startup pass; the
nightly task is the automatic path, and a duplicate created before it is repaired
by the next run.

`hearth` has no `status` subcommand; the only user-facing entry is the TUI. The
report is therefore in the daemon status payload (read by the TUI and
`read_status`) and in the Memory tab, which is the same split the issue's scope
describes.

Commits: `a7931ec` (test), `9b91fcf` (feat), `4c51009` (status task count).

Verified by the nine cases in `crates/hearth-memory/tests/consolidate.rs`,
including the idempotent second run, the report stamp, the pinned survivor, the
kind prior, and a duplicate created before the pass; by
`a_repeated_statement_is_shown_once_and_counted` in
`crates/hearth-memory/src/digest.rs`; by `memory_state_counts_the_store_not_the_loaded_page`
and `memory_state_reads_the_last_audit_report` in
`crates/hearth-runtime/tests/inspect.rs`; by
`status_reports_the_memory_breakdown_and_the_last_audit` in
`crates/hearth-runtime/tests/ipc_status.rs`; by
`the_consolidation_handler_repairs_publishes_and_sleeps` in
`crates/hearth-runtime/tests/maintenance_backup.rs`; and by
`a_crash_during_consolidation_leaves_no_row_half_closed` in
`crates/hearth-eval/tests/s7_crash.rs`, which runs the driver with
`HEARTH_CRASH_AT=memory.consolidate`, reopens the home, and sees both rows
current with no stamp. `./scripts/ci.sh` passes.
