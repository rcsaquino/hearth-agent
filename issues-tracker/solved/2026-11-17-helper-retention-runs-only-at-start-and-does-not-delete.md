# Helper retention runs only at startup and does not delete what the form says

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-17 |
| Area | `hearth-runtime`, `hearth-core` |

## Observed

`daemon()` calls `Session::prune_helpers` once, immediately after recovery. No
periodic pass calls it. `prune_helpers` marks matching helper conversations with
`conversations.archived = 1`. It does not delete a task row, an entry, a media
file, or a report file. The tasks stay readable, as `docs/DECISIONS.md` says.

The settings form does not describe that behavior. `retention_days` says "Delete
helper run records older than this many days" and `retention_runs` says "Keep at
most this many recent helper runs". `tasks_by_kind("helper")` and
`TaskTool::list` load every helper task ever written, and `prune_helpers` itself
reads them all at startup to decide what to archive.

## Wanted

One definition of retention. Either delete the records the form promises, or
change the form and SPEC to say that a helper run is archived and its task and
entries remain. If retention is meant to bound storage, the task and entry rows
need a retention path too. If it is only a view filter, the settings should say
so and the startup-only call should become a regular pass or be removed.

## Verification

Run a daemon with a one-day retention and a helper older than one day. Restart
once. The conversation has `archived = 1`, the `tasks` row still exists, and the
helper entries are still in `entries`. No media or report file is removed. A
second run after another day performs the same archive query again.

## Resolution

Helper retention has one definition and one shape. A nightly `helper.retention`
task calls `Session::prune_helpers`, which deletes the stale helper task rows
(beyond `retention_runs` or older than `retention_days`), clears child parent
links and effects for those tasks, and archives their conversations. The helper
entries and the report file remain part of the record. The startup-only call is
gone, and the settings form's "delete helper run records" wording now matches the
behavior.

Commits: `b270865` (test), `c74fbbb` (fix). Verified by
`helper_retention_deletes_task_rows_and_archives_conversations` and
`the_maintenance_tasks_are_created_once`, which requires the retention task.
