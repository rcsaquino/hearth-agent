# Test-only core and provider seams remain public

| | |
| --- | --- |
| Kind | chore |
| Reported | 2026-11-17 |
| Area | `hearth-core`, `hearth-memory`, `hearth-model`, `hearth-runtime` |

## Observed

The first pass over `public-apis-with-no-production-caller` removed the dead
helpers that had no test contract: `ToolError`'s never-set `detail`,
`EventBus::subscriber_count`, `Onboarding::extras_input`, `ui::editor::display_width`,
`helpers::is_terminal`, `Catalog::len`, `Catalog::is_empty`, `Memory::valid_at`,
and the skill-history reader and writer.

These items still have only test callers:

- `Session::latest_commit`, `Session::latest_usage`, `Session::usage_for_task`,
  and `Session::claim_task`. Production uses `claim_task_for_kind`,
  `claim_task_by_id`, and the usage queries in the IPC status path.
- The `hearth_memory::is_memory_line` re-export and its digest predicate.
- The `hearth_model::fetch_catalog` re-export; `lookup` uses the internal
  `catalog::fetch`.
- `Agent::run_submission_with_cancel` and
  `OpenAiCompatibleProvider::wire_body`. They are ordinary public methods, not
  `#[cfg(test)]` seams.

## Wanted

Keep each seam only if the test is the contract, and make it test-only. The
handler for this issue can move the tests that need a seam into unit tests, add
`#[cfg(test)]`, or replace the helper with the production API. For a seam that
must stay public, say in its doc comment which contract it exists for.

## Verification

`grep` for each symbol outside `crates/**/tests` and its defining module. The
remaining hits are tests only.

## Resolution

`Session::latest_commit`, `Session::latest_usage`, `Session::usage_for_task`,
and `Session::claim_task` are now `#[doc(hidden)]` and their doc comments name
the test contract they serve. `OpenAiCompatibleProvider::wire_body` is the same.
`Agent::run_submission_with_cancel` is removed and its tests call
`run_submission_with_events` with `NoopSink`.

The `is_memory_line` re-export is gone and the predicate is `#[cfg(test)]`.
The `fetch_catalog` re-export is gone; `catalog::fetch` is private and its HTTP
tests moved into the `catalog` unit module. No production caller changed.

Commit: `3e5815a`. Verified by a workspace `grep`, the model, memory, core,
runtime, and tools suites, and `scripts/ci.sh`.
