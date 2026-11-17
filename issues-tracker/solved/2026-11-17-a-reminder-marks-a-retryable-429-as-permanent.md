# A reminder marks a retryable send failure as permanent

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-runtime` |

## Observed

`TelegramSender::send_reply` retries a `429` a bounded number of times and then
returns the `TelegramError`. `TelegramError::retryable` reports that error as
retryable, because a rate limit is a deferral. The reminder path does not read
that classification.

`ReminderHandler::deliver` matches every `Err(error)` from `send_reply` by
calling `mark_effect_failed` and returning `TaskHandlerError::Permanent`. The
scheduler records the task as failed and never tries again. `advance` is only
called on an `Ok` status, so the branch that writes the "could not be confirmed"
notice does not run either. A one-shot reminder is lost, and a repeating
reminder stops repeating.

`helpers::send_notice` has the same error branch for helper ETA and settle
notices. Those notices are not restored by the answer delivery path either,
because `recover_delivery` ignores payloads marked `reminder` and helper notices
are not retried.

The solved `an-exhausted-429-is-recorded-as-permanent` issue fixed this for the
answer delivery path. The reminder and helper notice paths still classify by
the fact that the result was an `Err`, not by the retry policy.

## Verification

Point a `ReminderHandler` at a fake Bot API server that answers every
`sendMessage` with `429` and `retry_after: 0`. Step a due reminder. The effect is
`failed`, the task is `finished` with `TerminalOutcome::Failed`, and no notice
entry is written. The expected result is a deferral or a visible failure, not a
silent permanent loss.

## Resolution

`ReminderHandler::deliver` and `helpers::send_notice` now classify a send
failure with `TelegramError::retryable`. A rate limit marks the effect
`Unknown` and returns a retryable task failure; a permanent error still marks
the effect `Failed`. The next reminder step turns the unknown delivery into a
visible notice instead of ending the task as a silent permanent failure.

Commits: `030d879` (test), `3c6ed1f` (fix). Verified by
`a_rate_limited_reminder_is_not_marked_permanent`, which answers three sends
with `429`, requires a `Retryable` step error and an `Unknown` effect, then
requires the retry to finish with a notice naming the reminder.
