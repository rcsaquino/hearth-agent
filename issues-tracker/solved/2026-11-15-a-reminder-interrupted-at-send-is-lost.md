# A reminder interrupted at the send boundary is never delivered

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-runtime` |

## Observed

A reminder whose `telegram.send` effect is still `started` when the daemon stops
is marked `unknown` and then finished as delivered. Nothing retries or reports
it.

`crates/hearth-runtime/src/scheduler.rs:194`, `ReminderHandler::deliver`:

```rust
EffectStatus::Started => {
    self.session
        .mark_effect_unknown(
            &effect.id,
            json!({"message": "reminder delivery was interrupted"}),
        )
        .map_err(TaskHandlerError::from_core)?;
    return Ok(EffectStatus::Unknown);
}
```

`advance` (`scheduler.rs:251`) maps `Unknown` to `"unknown"` and, for a
non-repeating reminder, finishes the task:

```rust
EffectStatus::Done => "sent",
EffectStatus::Unknown => "unknown",
...
Ok(StepOutput::Terminal {
    outcome: TerminalOutcome::Done,
    result: json!({"delivery": delivery, ...}),
})
```

The answer-recovery path in `daemon.rs` explicitly excludes reminders
(`effect.payload.get("reminder") != Some(true)`), so the unknown effect is never
drained. There is no notice to the user either.

## Consequence

A reminder that crashes between `begin_effect` and the send is silently lost.
SPEC 16.2 says "A scheduled reminder survives a restart and fires once", and the
at-most-once policy says an uncertain action is "marked unknown and reported".
Here it is neither delivered nor reported: zero times, with a `Done` task and an
`unknown` effect left in the record.

## Verification

Create a one-shot reminder task, make it due, and have the fake sender stop the
process after `begin_effect` but before `send_reply` returns. Restart the daemon
and let recovery run. The task finishes `done`, the effect stays `unknown`, and
no message is sent. The expected behavior is either a delivery or a visible
notice that the reminder could not be sent.

## Resolution

When a reminder's send is uncertain, the task still finishes at-most-once and the
effect stays `unknown`, and `advance` now writes a `notice` entry to the
conversation naming the reminder and asking for a reschedule if it did not
arrive.

Commit: `a0c2ce0`. Verified by `an_interrupted_reminder_leaves_a_notice`, which
starts the effect, recovers, steps the handler, and asserts the Unknown effect
plus the notice entry.
