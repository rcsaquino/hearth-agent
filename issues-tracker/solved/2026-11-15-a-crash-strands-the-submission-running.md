# A crash mid-run leaves the submission running forever

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-core`, `hearth-runtime` |

## Observed

A run that is killed between `begin_submission` and `settle` leaves the
submission at `status = 'running'`. No production path moves it back to
`queued`, and no path settles it, so the message is never answered and never
reported.

`crates/hearth-core/src/session.rs`, `begin_submission`:

```rust
"UPDATE submissions SET status = 'running' WHERE id = ? AND status = 'queued'"
```

The only transition out of `running` is `settle`. Recovery handles tasks, not
submissions. `recover_tasks` (`session.rs:2344`) wakes due sleeping tasks and
requeues running tasks whose lease expired, but it does not touch the
submission row:

```rust
"UPDATE tasks SET status = 'pending', lease_owner = NULL, \
 lease_until = NULL, updated_at = ? \
 WHERE id = ? AND status = 'running'"
```

Every discovery path filters on `queued`:

- `next_queued_telegram_submission` (`session.rs:1135`): `WHERE s.status = 'queued'`.
- `queued_submissions` (`session.rs:1106`): `WHERE ... status = 'queued'`.
- `queued_submission_count` (`session.rs:1094`): `WHERE ... status = 'queued'`.

The requeued generation task cannot substitute for the queue. The only claimant
of `kind = 'generation'` is `Agent::run`, which creates its own task after the
submission is picked. The scheduler registry registers `reminder`,
`skill.review`, and `backup` only (`crates/hearth-runtime/src/daemon.rs:441`).

The state is already asserted by the crash suite. `crates/hearth-eval/tests/s7_crash.rs:292`
checks `submission.status == Running`, then the test expires the lease, calls
`recover_tasks`, and re-runs the driver itself, so the missing production path is
not exercised.

## Consequence

After a SIGKILL, an OOM kill, or a panic in the run thread, the user's message
is never answered. There is no notice entry, no `telegram.send` effect to
recover, and no log above `debug`. The submission stays `running` for the life
of the record. This contradicts SPEC 4.2 promise 7 ("One ending per message")
and 4.5 ("Running tasks resume from their last saved checkpoint").

## Verification

In a core test: create a conversation, call `submit_telegram`, then
`begin_submission`, create a `generation` task with
`input_json["submission_id"]`, and claim it. Expire the lease, call
`recover_tasks`, then assert `next_queued_telegram_submission()` is `None` while
`submission_by_id` still reports `Running`. A production recovery should either
return the submission to `queued` (the link is `tasks.input_json["submission_id"]`)
or settle it with a notice.

## Related

`open/2026-11-15-a-crash-wedges-every-chat-behind-a-busy-run.md` is the same
persisted state seen from the daemon: the still-running generation task blocks
later submissions. Fixing one does not fix the other.

## Resolution

`Session::recover_interrupted` runs at daemon start and treats every `running`
task as orphaned, because no run in a new process can own one. A generation task
whose submission is still `running` sends that submission back to `queued` and
retires the task, so the worker picks the message again. The daemon calls it
where it used to call the lease-based `recover_tasks`.

Commit: `c503d8e`. Verified by `an_interrupted_submission_is_queued_again` in
`crates/hearth-core/tests/s1_tasks.rs` and
`an_unclean_restart_still_answers` in `crates/hearth-cli/tests/s8_telegram_daemon.rs`.
Both fail when the daemon goes back to the lease-based recovery.
