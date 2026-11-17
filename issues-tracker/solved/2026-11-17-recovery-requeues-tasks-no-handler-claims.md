# Recovery requeues foreground tasks that no handler claims

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-core` |

## Observed

`Session::recover_interrupted` treats every `running` task as orphaned. Its only
special case is a `generation` task whose submission is still `running`, which it
returns to the queue and retires. Every other running task falls through to a
generic update to `pending`.

That generic update is right for `helper`, `reminder`, `backup`, and
`skill.review`, because those kinds have a claimant. It is wrong for tasks whose
only driver is a foreground caller.

- A generation task whose submission already settled, for example a crash
  between `Session::settle` and `Session::finish_task`, becomes pending. The
  scheduler registry contains only `reminder`, `skill.review`, and `backup`, and
  `Agent::run` creates a fresh generation task only for a queued submission. The
  pending row is never claimed.
- A compaction task becomes pending after a crash between the summary save and
  `finish_task`. `Compactor` is called directly from the agent and idle paths, not
  from the scheduler, so this row is never claimed either.

The rows accumulate in `tasks` and in the running and asleep counts. They also
make recovery reports look successful when the work is actually unreachable.

`Session::recover_tasks` has a second problem in the same area. Its
lease-expiry branch for `running` tasks is unreachable in the daemon because
`recover_interrupted` runs first and leaves no running task behind. That branch
can only be exercised by tests that call it directly.

## Verification

Create a running generation task whose submission is `done`, call
`recover_interrupted`, and inspect the task. It is `pending`, `tasks_by_kind`
finds it, and no registered handler owns the kind. Repeat with a running
`compaction` task, which has no registry entry even while it is healthy.

## Resolution

`Session::recover_interrupted` only returns a task to `pending` for a kind with
a claimant: `helper`, `reminder`, `backup`, or `skill.review`. A foreground
`generation` task whose submission already settled, a `compaction` task, and
any other kind are settled as `failed` with `{"error":"interrupted"}` and a
`task.interrupt` commit, so they do not accumulate as unclaimable pending rows.

Commits: `80123d7` (test), `67101b4` (fix). Verified by
`recovery_does_not_requeue_a_foreground_task`, which recovers a running
generation task with a done submission and a running compaction task and
requires both to be failed with the cause recorded.
