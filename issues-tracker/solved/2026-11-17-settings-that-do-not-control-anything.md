# Several settings are accepted but do not control anything

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-17 |
| Area | `hearth-config`, `hearth-runtime` |

## Observed

Three groups of settings are parsed, validated, and shown in the form, but the
running code does not use the value.

- `logging.level` accepts `trace`, `debug`, `info`, `warn`, and `error`. The form
  labels it "Diagnostic verbosity" and the tooltip lists the levels.
  `hearth_runtime::init_logging` installs `tracing_subscriber` with
  `with_max_level(tracing::Level::INFO)` and never reads the setting.
- `compaction.idle_minutes` and `compaction.idle_min_tokens` are validated as
  positive and have form rows. `TelegramWorker::run` hardcodes
  `3_600_000_000` microseconds and `100_000` tokens (clamped by the budget) when
  it calls `idle_compaction_candidate`.
- `memory.rules_max_tokens` and `memory.recall_max_items` are validated only as
  positive. `MemoryConfig::bounded` silently clamps them to 50 through 4,000 and
  1 through 64. A file that says `rules_max_tokens: 1` or
  `recall_max_items: 1000` is accepted, the form displays the file value, and the
  store uses a different value.

## Wanted

A setting that is documented and displayed should either control its named
behavior or be rejected with the accepted range. Silently clamping or ignoring a
value makes the file and the running system disagree, which is worse than a
clear validation error. Remove a setting that no longer has a behavior, and
update the form and SPEC with the removal.

## Verification

Set `logging.level` to `debug` and emit a `tracing::debug!` line. The log file
has no line. Set `compaction.idle_minutes` to 1 and wait two minutes with a large
idle conversation. Idle compaction still uses the one-hour constant. Set
`memory.rules_max_tokens` to 1 and inspect `MemoryService::config` through the
status path; it reports 50.

## Resolution

`init_logging` maps `logging.level` to the subscriber's maximum level.
`TelegramWorker` passes `compaction.idle_minutes` and
`compaction.idle_min_tokens` to `idle_compaction_candidate` and uses the same
idle interval for its repeat gate. `memory.rules_max_tokens` and
`memory.recall_max_items` are validated in their accepted ranges, so a file
outside 50 to 4,000 or 1 to 64 is refused instead of silently clamped.

Commits: `480673f` (test), `3381b06` (fix). Verified by
`the_daemon_log_redacts_shape_and_configured_credentials`, which emits a
`debug` line from a debug-level home and requires it in the log, and by
`a_memory_budget_outside_its_range_is_rejected`.
