# Maintenance and the scheduler only start when Telegram is configured

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-runtime` |

## Observed

`daemon()` builds `TelegramRuntime` only when a Telegram bot token is present.
That object owns the poller, the worker, the `Scheduler`, the `HelperPool`, and
the call to `ensure_maintenance_tasks`. The scheduler thread, the nightly backup,
the nightly skill review, and helper workers do not exist in a daemon started
without Telegram.

`Settings::validate_for_run` only requires `models`. A settings file with
identity and models but no `telegram` block passes validation and starts a
daemon. That daemon serves status, but it never schedules a backup, never runs a
skill review, never wakes a reminder, and never prunes helper retention. The
scheduler wake bug already means a running daemon misses due tasks, and this
condition means an unconfigured daemon has no scheduler at all.

If Telegram is a required part of a runnable home, `validate_for_run` should say
so. If it is optional, the scheduler and maintenance should not depend on the
transport.

## Verification

Create a temporary home with a valid `models` block and no `telegram` block.
Start the daemon. `read_status` succeeds, `ensure_maintenance_tasks` has not
created `backup` or `skill.review` rows, and no scheduler or helper thread is
started. The guarded block in `daemon()` is the only construction site for
`Scheduler`.

## Resolution

Telegram stays optional. A daemon without a bot token now builds a maintenance
scheduler from the configured model, registers the backup and skill-review
handlers, calls `ensure_maintenance_tasks`, and runs the scheduler thread. The
reminder handler and helper pool still exist only with Telegram, because both
need a chat to do anything.

Commits: `1baaf2f` (test), `dc0c311` (fix). Verified by
`a_daemon_without_telegram_still_schedules_maintenance`, which starts a
models-only daemon and requires the `backup` and `skill.review` tasks to exist.
