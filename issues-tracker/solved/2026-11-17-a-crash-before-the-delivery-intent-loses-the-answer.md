# A crash after settling an answer can lose it before delivery

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-runtime` |

## Observed

`Agent::run` saves the final assistant entry, settles the submission `done`, and
finishes the generation task. It returns a `RunOutcome` to
`TelegramWorker::run_one`, and only then does `TelegramWorker::deliver` create
the `telegram.send` effect with the answer text.

A process death between the return from `Agent::run` and the effect insert leaves
a settled submission with a saved answer and no effect row. On restart,
`EffectWorker::recover_started` has nothing to mark unknown, and
`recover_delivery` reads `telegram.send` effects and finds none. The answer is
durable and unreachable.

The solved `a-finished-answer-can-miss-delivery` issue removed two `?` operators
inside the live process so a bookkeeping failure does not skip `deliver`. It does
not cover a process death in the handoff, because no delivery record is written
inside the answer or settle save.

## Verification

Seed a run that saves and settles, stop the process immediately after
`finish_task`, and restart. `submission_by_id` reports `Done` with an answer
entry, and `effects_awaiting_delivery("telegram.send")` is empty. A crash driver
can also stop at the save boundary and let the next process read the record.

## Related

The `deliver` path is also skipped when a failure notice is settled but
`finish_task` returns an error, because `settle_failure` still uses `?` on the
task bookkeeping. The two faults share the same missing invariant, that the
outcome and its delivery intent be written together or be derivable from the
record.

## Resolution

A settled Telegram answer is now derivable from the record.
`Session::done_answers_without_delivery` returns recent `done` submissions that
have an answer entry and no `tg:<submission>` effect, and the worker derives the
delivery intent from the answer before it starts polling. The worker also
retries every pending non-reminder delivery once at startup. A failure of the
absorbed-settle or task bookkeeping in the cancel and failure paths is logged
instead of skipping the returned notice.

Commit: `7adc613`, with the test in `37e3e4b`. Verified by
`the_daemon_delivers_an_answer_whose_effect_was_never_written`, which settles a
submission with an answer and no effect, starts the daemon, and requires the
answer to be sent and the derived effect to reach `Done`.
