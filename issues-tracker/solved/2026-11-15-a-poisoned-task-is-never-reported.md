# A poisoned run fails the task and tells nobody

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-core` |

## Observed

When a task crashes twice, recovery marks it `failed` with a result of
`{"error":"poisoned"}` and stops. It writes no notice entry, settles no submission, and queues no
effect.

`crates/hearth-core/src/session.rs:2379`, the poison branch of `recover_tasks`:

```rust
if attempt >= 2 {
    let result = serde_json::to_string(&serde_json::json!({
        "error": "poisoned"
    }))
    ...
    "UPDATE tasks SET status = 'failed', result_json = ?, \
     terminal_at = ?, lease_owner = NULL, lease_until = NULL, \
     next_wake_at = NULL, updated_at = ? \
     WHERE id = ? AND status = 'running'"
```

The only consumer of the count is a log line at
`crates/hearth-runtime/src/daemon.rs:120`. Nothing reads a task result whose
`error` is `poisoned`. The submission linked through
`tasks.input_json["submission_id"]` stays `running`, which is the state described
in `open/2026-11-15-a-crash-strands-the-submission-running.md`.

SPEC 4.5 requires the opposite: "A request that kills the process twice is
marked `poisoned` (blocked from retrying) and fails the task with a
user-visible notice instead of looping forever."

## Consequence

A run that repeatedly kills the process is retried no further, but the person
sees nothing: no answer, no notice, and no recovery. The record shows a
submission that never ended. `crates/hearth-core/tests/s1_tasks.rs:420` fixes the
task-row shape and asserts nothing about a notice, so the missing notice is not
an intended contract.

## Verification

In a core test, claim a generation task with a submission, expire it, recover,
then claim and expire again and recover a second time. Assert the task is
`failed`, the submission is `Unanswered`, and the conversation history ends with a
`notice` entry. Today the first two hold only for the task and the third is
absent.

## Resolution

A second interruption of the same submission now settles it `Unanswered`, writes
a `notice` entry that says the request stopped the process twice and asks for it
to be sent again, and records the task as `poisoned`. The person is told instead
of the message disappearing.

Commit: `c503d8e`. Verified by `a_second_interruption_is_reported_to_the_person`,
which runs recovery twice and asserts the Unanswered submission and the notice
text.
