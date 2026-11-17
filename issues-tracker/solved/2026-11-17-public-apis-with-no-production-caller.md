# Public APIs and fields with no production caller

| | |
| --- | --- |
| Kind | chore |
| Reported | 2026-11-17 |
| Area | `hearth-core`, `hearth-memory`, `hearth-runtime`, `hearth-cli` |

## Observed

The following public items have no caller outside tests. Some are seams that
should become test-only; others are remnants of a removed surface.

- `Session::latest_commit`, `Session::latest_usage`, `Session::usage_for_task`,
  `Session::prompt_for_entry`, `Session::latest_conversation`, and
  `Session::claim_task`. The production task paths use
  `claim_task_for_kind`, `claim_task_by_id`, and the usage queries built into the
  IPC status path.
- `Memory::valid_at` and the `hearth_memory::is_memory_line` re-export. Only the
  store SQL and an integration test use the validity rule or the line predicate.
- `ToolError::with_detail` has no caller, and the `detail` field is never set, so
  the detail branch in `ToolError::to_json` cannot run.
- `SkillLibrary::history` and `SkillLibrary::version_body` have only test
  callers. `save_learned` still copies a version into `.history/<name>/` on every
  revision. The version reader was removed when the Skills tab became read-only
  in Draft 0.38. The solved `skill-history-has-no-surface` issue added that
  reader, then a later change removed it without removing the writer or the
  library API.
- `Catalog::len`, `Catalog::is_empty`, and the public `fetch_catalog` have no
  production caller.
- `ToolGroups::schema_tokens`, `EventBus::subscriber_count`,
  `Onboarding::extras_input`, `ui::editor::display_width`, and
  `helpers::is_terminal` are test-only or unused.
- `Agent::run_submission_with_cancel`,
  `OpenAiCompatibleProvider::wire_body`, and `spill::ensure_root_owned` are
  test seams. `ensure_root_owned` is already `cfg(test)`, the other two are
  ordinary public methods.

## Wanted

Remove the items with no caller. Keep a test seam only if the test is the
contract, and mark it `#[cfg(test)]` so the production API stays honest. For the
skill history writer, decide whether to restore a reader or stop writing copies;
do not keep writing files that no current surface reads.

## Verification

`grep` for each symbol outside `crates/**/tests` and the module that defines it.
The two skill history methods appear only in `crates/hearth-runtime/tests/`, and
the skill writer still runs in `save_learned`.

## Resolution

Removed the items with no caller or no test contract: `ToolError::with_detail`
and the never-set `detail` field, `EventBus::subscriber_count`,
`Onboarding::extras_input`, `ui::editor::display_width`, `helpers::is_terminal`,
`Catalog::len`, `Catalog::is_empty`, `Memory::valid_at`, `Session::prompt_for_entry`,
`Session::latest_conversation`, and the skill-history reader and writer.
`SkillLibrary::save_learned` no longer copies a version into `.history/`.
`ToolGroups::schema_tokens` stays because the production schema-budget checks
use it.

The remaining test-only seams are split into
`2026-11-17-test-only-core-and-provider-seams.md`: `Session::latest_commit`,
`latest_usage`, `usage_for_task`, `claim_task`, the `is_memory_line` re-export,
`fetch_catalog`, `Agent::run_submission_with_cancel`, and
`OpenAiCompatibleProvider::wire_body`.

Commit: this resolution's parent. The workspace builds, the core, tools, model,
and runtime suites pass, and `scripts/ci.sh` is green.
