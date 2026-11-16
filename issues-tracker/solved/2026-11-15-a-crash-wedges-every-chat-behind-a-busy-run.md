# A still-running generation task wedges every chat behind a busy run

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-runtime`, `hearth-core` |

## Observed

After an unclean restart, the daemon can answer every Telegram message with a
busy loop and never make progress. The worker re-picks the same submission every
100 ms, sends a typing action, receives `Busy`, and starts again.

`crates/hearth-runtime/src/agent.rs:370` rejects a run when any generation task
in the conversation is still `running`:

```rust
if self
    .session
    .active_generation_task(&submission.conversation_id)?
    .is_some()
{
    return Err(AgentError::Busy);
}
```

`crates/hearth-core/src/session.rs:2328`, `active_generation_task`, selects on
status only. It does not compare `lease_until` to the clock:

```rust
"SELECT id FROM tasks WHERE conversation_id = ? AND kind = 'generation' \
 AND status = 'running' ORDER BY created_at, id LIMIT 1"
```

A process killed mid-run leaves the row `running` with a 60 second lease.
`recover_tasks` runs only at daemon start (`crates/hearth-runtime/src/daemon.rs:110`)
and requeues only a row whose lease has already expired. A restart inside the
lease window therefore leaves the row `running`, and nothing ever expires it
again while the process lives. Even after the lease time passes, the row stays
`running` and the check above keeps failing.

The worker loop makes the wedge total. `crates/hearth-runtime/src/daemon.rs:613`
takes the globally oldest queued Telegram submission:

```rust
match self.session.next_queued_telegram_submission() {
    Ok(Some(submission)) => self.run_one(submission),
```

`run_one` starts a typing thread before calling the agent
(`daemon.rs:776`), and on `Busy` the loop sleeps 100 ms and repeats
(`daemon.rs:818`). The first message that reaches this check is a new message in
the crashed conversation: it is queued, `run` returns `Busy`, and the message is
never settled, so it stays queued. From then on it is the globally oldest queued
submission, and every later message in any chat queues behind it. Each iteration
sends at least one `sendChatAction`, which is about ten Telegram requests per
second for as long as the process runs.

## Consequence

After a crash, the next message in the affected conversation returns `Busy` and
stays queued. That single stuck submission then blocks every later message in
every chat, so one crash produces a lasting Telegram outage plus a steady
request stream. The only trace is a `debug` log line. Restarting the daemon
again after the lease has expired requeues the task and clears the wedge, but
the submission it belonged to is then lost (see the related issue).

## Verification

Create a conversation, submit a message, leave a `generation` task `running`
with `lease_until` in the future, then run the daemon worker against it. The
worker should not return `Busy` forever, or the recovery should requeue or
settle the task before the worker starts. `cargo test -p hearth-runtime --test s7_scheduler`
and `cargo test -p hearth-eval --test s7_crash` cover the recovery primitives but
not this daemon loop.

## Related

`open/2026-11-15-a-crash-strands-the-submission-running.md` covers the lost
submission. This file covers the blocking task row.

## Resolution

The same recovery retires the orphaned generation task instead of leaving it
`running`, so `active_generation_task` no longer reports `Busy` for a process
that is gone. The worker resumes with the requeued submission rather than
looping.

Commit: `c503d8e`. Verified by `an_unclean_restart_still_answers`, which seeds a
running generation task with an unexpired lease before starting the daemon; on
the parent commit the daemon never sent an answer.
