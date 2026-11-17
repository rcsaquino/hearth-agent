# The effect worker has no production effect to run

| | |
| --- | --- |
| Kind | chore |
| Reported | 2026-11-17 |
| Area | `hearth-runtime` |

## Observed

`EffectWorker::execute` handles only `file_write`. `EffectWorker::run_once` skips
every `telegram.send` intent, because delivery is performed inline by
`TelegramWorker::deliver`, `ReminderHandler::deliver`, and
`helpers::send_notice`. No production code creates a `file_write` intent. Every
production `EffectIntent::new` call uses `telegram.send`.

The daemon still starts an effect worker thread that polls
`Session::pending_effects` every 100 ms, skips each telegram intent, and has
nothing else to execute. `FileWrite`, its `FileWrite` re-export, and the
`file_write` recovery branch exist for a producer that is not in the tree. The
crash driver and tests use the worker, so the dead code is exercised only there.

## Wanted

Either remove `EffectWorker`, `FileWrite`, the daemon thread, and their tests, or
give the file-write effect a real producer and use it from a current feature.
`recover_started` still has a purpose for telegram intents; keep that part only
if it is separate from the dead executor.

## Verification

`grep` for `EffectIntent::new` in `crates` outside tests returns three
`telegram.send` sites and no `file_write`. `EffectWorker::execute` has one arm.
The worker's run loop on an ordinary home queries only telegram intents and
returns `processed = 0` on every pass.

## Resolution

Removed `EffectWorker`, the daemon thread and join, the `FileWrite` tool, the
`file_write` recovery branch, the crash-driver `effects` mode, and the crash test
that exercised it. The daemon still marks a `telegram.send` effect that was
running at a crash as `unknown` through `recover_started_effects`; any other
started effect is failed as unsupported. `scripts/acceptance.sh` no longer names
the removed `during_effect` point, and `docs/SCHEMA.md` drops `file_write` from
the example kinds.

Commit: `632a8f4`. The crash suite, the tool suite, the scheduler suite, and clippy
are green.
