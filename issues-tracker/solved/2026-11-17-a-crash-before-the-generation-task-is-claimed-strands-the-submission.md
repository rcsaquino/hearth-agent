# A crash before the generation task is claimed strands the submission

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-runtime` |

## Observed

`Agent::run` moves a submission through three separate writes. It calls
`Session::begin_submission`, then `Session::create_task`, then
`Session::claim_task_by_id`. A process death between any two of them leaves a
submission at `status = 'running'` with no running generation task, or with a
pending generation task that nobody claims.

Recovery only sees half of that state. `Session::recover_interrupted` scans
`tasks` rows that are `running` and handles a generation task whose submission is
still running. If the submission is running but the task does not exist yet, or
the task is still `pending`, there is no row for recovery to find.
`next_queued_telegram_submission` filters on `queued` and `recover_tasks` only
touches `asleep` and `running` tasks. Nothing moves the submission back to
`queued`, and nothing settles it with a notice.

The solved `a-crash-strands-the-submission-running` issue covers a claimed
generation task whose `input_json` has a `submission_id`. This is the narrower
window before that link exists, and it is still open.

## Consequence

The message is never answered. It is also invisible to the Telegram worker,
because every queue lookup filters on `queued`, so the person sees a run that
never settles and no notice explains it.

## Verification

Call `begin_submission` on a submitted message in a scratch home, then stop the
process before `create_task`. On restart, `recover_interrupted` returns an empty
report, `next_queued_telegram_submission` returns `None`, and
`submission_by_id` still reports `Running`. The same shape appears if the task
was created but `claim_task_by_id` returned before the claim.

## Resolution

`Session::recover_interrupted` now scans running submissions after it scans
running tasks. A submission with no running generation task is returned to
`queued`, and a pending or sleeping generation task for it is settled as
`failed` with `{"error":"interrupted"}`. A running task still follows the
existing recovery branch.

Commit: `f989362`, with the pending-task test in `0988974`. Verified by
`recovery_requeues_a_running_submission_with_no_claimed_task` and
`recovery_requeues_a_running_submission_with_no_task`; the second case was
added after the fix and passes on its first run.
