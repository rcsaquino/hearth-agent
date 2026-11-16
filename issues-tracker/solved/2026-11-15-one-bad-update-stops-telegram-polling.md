# One undecodable update stops polling for every chat

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-telegram` |

## Observed

`getUpdates` decodes the whole batch into `Vec<Update>` in one call. One update
that does not fit the declared shapes fails the batch, so `poll_once` returns
before admitting any update in it, and the offset is not advanced. The next poll
fetches the same batch again.

`crates/hearth-telegram/src/api.rs:192`:

```rust
serde_json::from_value(result).map_err(|error| TelegramError::Protocol {
    method: "getUpdates".to_owned(),
    detail: format!("invalid update list: {error}"),
})
```

`crates/hearth-telegram/src/poller.rs:96`:

```rust
let updates = self.client.get_updates(offset, self.poll_timeout, &["message", "callback_query"])?;
```

The types declare required fields. `crates/hearth-telegram/src/types.rs:7`:

```rust
pub struct Update {
    pub update_id: i64,
    #[serde(default)]
    pub message: Option<Message>,
}
```

`Message` requires `message_id` and `chat`, and `Chat` requires `id` and `type`.
There is no per-update tolerance and no fallback that skips a bad element.

The daemon's poll loop treats the protocol error as transient, backs off, and
retries (`crates/hearth-runtime/src/daemon.rs:623`), so it never gets past the
bad batch.

## Consequence

No message from any chat is admitted until the offending update ages out of
Telegram's retention window, which can be a day. The only transport is down, and
the log shows a repeating protocol error rather than a skipped update. The
trigger is upstream shape drift (a new update kind or a changed required field),
so it is infrequent, but the failure has no per-update isolation.

## Verification

Serve a fake `getUpdates` body containing two updates where the second has a
`message` with no `chat`. Call `poll_once` and assert the first update's sender
is admitted. Today the decode fails for the batch and nothing is admitted; repeat
the call and the same batch fails again.

## Resolution

`get_updates` decodes the batch one element at a time. An element that does not
fit the declared shapes is logged and replaced by its `update_id` with no
message, which the poller counts as ignored and which still advances the offset,
so the rest of the batch is admitted and the bad update ages out.

Commit: `9e3f283`. Verified by `one_undecodable_update_does_not_stop_the_batch`,
which serves a good update and one without a chat and asserts one admission and
offset 63.
