# A stop that arrives before the run registers its token is dropped

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-telegram`, `hearth-runtime` |

## Observed

The poller records the `stop` update and advances the offset, then asks the
cancellation registry to stop the chat. If the run has not started yet, no token
is registered, the request returns false, and the stop is lost.

`crates/hearth-telegram/src/poller.rs:141`:

```rust
let active = self
    .stop
    .as_ref()
    .is_some_and(|handler| handler.stop(message.chat.id));
```

`crates/hearth-runtime/src/daemon.rs:569`, `set_cancelled`:

```rust
fn set_cancelled(&self, chat_id: i64) -> bool {
    let Ok(tokens) = self.tokens.lock() else {
        return false;
    };
    let Some(token) = tokens.get(&chat_id) else {
        return false;
    };
    token.store(true, Ordering::Relaxed);
    true
}
```

A token exists only while `run_one` is inside a run
(`daemon.rs:551`, `register`). The worker starts a submission at most every
100 ms when idle, so two messages in one poll batch ("the request", then "stop")
hit the window: the poller drains both before the worker picks the first.

## Consequence

The documented stop gesture does nothing in a window that covers all queue
latency, and the run proceeds to a full answer after the user asked it to stop.
Nothing tells the user the stop was ignored. SPEC 8.5 presents stop as reliable,
and the stop update is consumed, so sending it again is the only recovery.

## Verification

Serve one `getUpdates` batch with a message and then a `stop` for the same chat,
then let the worker run. The first submission settles `done`. The expected
behavior is `Cancelled`, or a recorded pending stop that the run observes as soon
as it registers its token.

## Resolution

The cancellation registry records a stop for a chat with no active run, and
`register` observes it so the run starts cancelled. A new message admission
clears a pending stop, so a stale stop cannot cancel a later message. The poller
calls `admit` before admitting an update.

Commit: `2f3806a`. Verified by `a_stop_in_the_message_batch_cancels_the_run`,
which serves the message and the stop in one batch and requires the submission to
settle `Cancelled`; before the fix it settled `Done`.
