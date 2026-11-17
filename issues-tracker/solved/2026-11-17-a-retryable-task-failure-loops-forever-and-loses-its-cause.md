# A retryable task failure loops forever and loses its cause

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-core` |

## Observed

`Session::finish_task` handles `StepOutput::Failed { retryable: true }` by
setting the task back to `pending`, resetting `attempt` to 0, setting
`next_wake_at` one second ahead, and writing no error or result. The `error`
value carried by the step is discarded.

A handler that returns `TaskHandlerError::Retryable` every time therefore retries
forever at roughly one attempt per second. `BackupHandler` is one such handler
when the backup path fails. The only poison check in `recover_tasks` looks at
expired `running` leases, not at repeated retryable failures, and the attempt
reset makes an attempt count useless even if a cap were added later.

`Scheduler::tick` returns after one claimed task and iterates registered kinds in
`BTreeMap` order. A permanently failing `backup` task, which sorts first, is
re-claimed every time it becomes due and can starve reminders and
`skill.review`.

A malformed helper task is another task that never resolves. `HelperPool::run_helper`
returns without finishing when the task has no conversation or no brief. The
task stays `running` with its one-hour lease. No production path calls
`recover_tasks` while the daemon runs, so it can stay running, and counted
against `tasks.max_concurrent`, until a restart.

## Verification

Register a handler that always returns `TaskHandlerError::Retryable`, start the
scheduler, and observe repeated `task.retry` commits. The task stays `pending`,
`attempt` stays 0, and `result_json` stays null. The scheduler log is the only
place the cause appears.

## Resolution

`Session::finish_task` keeps the failed `attempt` instead of resetting it, stores
the step's error in `result_json`, and delays the next claim by one second
doubling to five minutes. The fifth retryable failure settles the task as
`failed` with `{"error": <cause>, "poisoned": true}`. `Scheduler::tick` no
longer returns after one claimed task, so a failing kind cannot delay every kind
that sorts after it. `HelperPool::run_helper` finishes a task with no
conversation or no brief as a permanent failure instead of leaving it running.

Commits: `7d44cfb` (core test), `e441785` (helper test), `bbf03d6` (retry fix),
`9e60b48` (helper fix). Verified by
`a_retryable_failure_keeps_its_attempt_and_records_the_cause`,
`a_retryable_failure_stops_after_five_attempts`, and
`a_malformed_helper_task_is_finished_as_failed`.
