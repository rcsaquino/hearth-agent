# A durable answer can miss delivery when task bookkeeping fails

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-runtime` |

## Observed

A run saves the answer, settles the submission `done`, then finishes the
generation task. If the last step fails, the run returns an error and the
delivery step never executes.

`crates/hearth-runtime/src/agent.rs:694`:

```rust
let answer_entry_id =
    self.save_assistant(&submission, &prompt_hash, &thinking, &text, &[])?;
let settled = self.session.settle(
    &submission.id, SubmissionStatus::Done, Some(&answer_entry_id))?;
self.settle_absorbed(&absorbed, SubmissionStatus::Done, Some(&answer_entry_id), None)?;
self.session.finish_task(
    &claimed.id, &self.config.owner,
    StepOutput::Terminal { outcome: TerminalOutcome::Done, .. },
)?;
return Ok(RunOutcome { .. });
```

The `?` on `finish_task` (also on `settle_absorbed`) discards the outcome. The
daemon only calls `deliver` on `Ok` (`daemon.rs:791`). No `telegram.send`
intent was written before the failure, so recovery has nothing to find:
`recover_delivery` reads effects, and none exists.

## Consequence

The answer is stored and settled but never sent, and the person sees nothing.
The failure leaves a `warn` log and a `done` submission whose answer is not
reachable. SPEC 8.6 promises that an undelivered answer stays durable and is
delivered later; without the effect row, that promise cannot be kept.

## Verification

Force `finish_task` to fail after a successful answer. One route is to change
the task's status or owner between the run and the finish call, which makes
`finish_task` return `TaskNotRunning` or `LeaseLost` (`session.rs:2155`). Then
assert the submission is `Done` with an answer entry and that `effects` holds no
`telegram.send` row for that submission. A storage error in the same window
produces the same result.

## Related

`open/2026-11-15-an-exhausted-429-is-recorded-as-permanent.md` is another path
where a durable answer is not delivered. The delivery handoff is the shared
subject, not the mechanism.

## Resolution

After the answer entry is saved and the submission is settled, a failure in
`settle_absorbed` or `finish_task` is logged instead of aborting the run. The
answer is durable and the daemon's delivery follows the `Ok` outcome, so the
message is still sent.

Commit: `4d8c140`. The full `s6_agent` and daemon suites pass. A test that fails
only the final `finish_task` needs a seam that does not exist: the agent
heartbeats the task immediately before the answer is saved, so any mid-run
sabotage fails that heartbeat first. The change is the removal of the two `?`
operators; the outcome is exercised by the existing success path.
