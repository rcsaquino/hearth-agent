# An exhausted 429 is recorded as a permanent delivery failure

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-telegram`, `hearth-runtime` |

## Observed

When Telegram rate-limits a send, the sender retries twice and then returns the
429 error. The daemon records the effect as `failed`, which is terminal, so the
answer is never delivered later.

`crates/hearth-telegram/src/sender.rs:74` (the rich path, and the same shape at
`:209` in the fallback path):

```rust
Err(error @ TelegramError::Api { code: Some(429), retry_after, .. }) => {
    if attempts >= 2 {
        return Err(error);
    }
    self.sleeper.sleep(Duration::from_secs(retry_after.unwrap_or(1)));
    attempts += 1;
}
```

`crates/hearth-runtime/src/daemon.rs:974` maps every error from `send_reply` to
`mark_effect_failed`:

```rust
Err(error) => {
    if let Err(save_error) = self
        .session
        .mark_effect_failed(&effect_id, json!({"error": error.to_string()}))
```

`effects_awaiting_delivery` lists only `intent` and `unknown`
(`crates/hearth-core/src/session.rs:499`), so the recovered-reply path can never
reach a failed effect.

In the fallback path the error can arrive after earlier chunks were already
delivered. The answer is then truncated on the user's side, with no marker and
no recovery.

## Consequence

A temporary server-side deferral permanently loses the answer. The record keeps
the answer entry, but no delivery is attempted again, and the user sees either
nothing or a partial message. SPEC 8.3 and 8.6 treat rate limiting as a
deferral and an undelivered answer as durable.

## Verification

Point the sender at a fake Bot API server that answers `sendMessage` with
`429` and `retry_after: 0` on every attempt. Deliver one run and assert the
effect status and the set returned by
`effects_awaiting_delivery("telegram.send")`. The effect is `failed` and the set
is empty.

## Resolution

`TelegramError::retryable` marks 429, 5xx, and transport failures as deferrals.
The daemon records those effects as `unknown` instead of `failed`, both on the
live delivery path and on recovery, and `effects_awaiting_delivery` includes
`unknown`, so the answer remains recoverable.

Commit: `2d8266e`. Verified by `a_rate_limited_delivery_is_left_for_recovery`,
which points the daemon at a fake Bot API that answers every rich send with 429
and asserts a recoverable effect remains.
