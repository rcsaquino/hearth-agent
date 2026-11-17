# A sleeping task is never woken while the daemon runs

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-runtime` |

## Observed

A reminder scheduled for later, the nightly backup, and the nightly skill review
are all saved as `asleep` tasks with a `next_wake_at` instant. `create_sleeping_task`
and the `StepOutput::Sleep` branch of `Session::finish_task` are the two writers.
The scheduler never turns a due sleeping task into a pending one.

`Scheduler::tick` asks `Session::claim_task_for_kind` for work, and
`claim_task_matching` selects only `status = 'pending'`. The only transition from
`asleep` to `pending` is `Session::recover_tasks`, and `daemon()` calls that once,
before it starts the scheduler thread. No periodic pass calls it again.

A reminder created for `delay_seconds: 60` therefore stays asleep until the next
daemon restart. A recurring reminder that ran once sleeps forever after its first
`StepOutput::Sleep`. The nightly `backup` and `skill.review` tasks never run in a
daemon that stays up across their wake time, and they run late only if the daemon
happens to restart after it.

The scheduler tests call `Session::recover_tasks` by hand before
`claim_task_for_kind`, so they exercise a production path that never runs.

## Verification

A scheduler over a temporary home was given one `wake-audit` sleeping task due
one second in the past and a counting `TaskHandler`. `Scheduler::run` ran for
500 ms. The handler was not called, the task stayed `Asleep`, and the scheduler
reported no error. The same result follows from the query in
`claim_task_matching` and the single `recover_tasks` call in `daemon()`.

## Related

`docs/SPEC.md` 5.4 says a missed wake-up runs on startup, which assumes the
wake-up happened while the process was running. 5.5 and 9.6 rely on the same
mechanism for compaction and skill review. The behavior is also what makes
`a-reminder-interrupted-at-send-is-lost` hard to observe in production.

## Resolution

`Session::claim_task_matching` and `Session::claim_task_by_id` now treat a
sleeping task whose `next_wake_at` has passed as claimable. The claim moves it
straight to `running`, so the scheduler no longer needs a periodic
`recover_tasks` pass to wake it. `recover_tasks` still handles startup.

Commits: `e35ee58` (test), `1b9abc5` (fix). Verified by
`a_due_sleeping_task_is_woken_while_the_scheduler_runs`, which starts the real
scheduler over a sleeping task one second in the past, waits for the handler,
and requires the task to finish.
