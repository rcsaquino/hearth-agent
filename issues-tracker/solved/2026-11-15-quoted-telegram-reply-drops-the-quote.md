# A quoted Telegram reply arrives without the quoted message

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-telegram` |

## Observed

The user quoted a message in Telegram and replied to it. The agent received the
typed text only. The quoted body did not arrive, so the reply could not be
resolved against what it referred to. The user had to ask whether the quote was
visible at all.

The ambiguity was not recoverable from inside the turn. A reply of "this reply"
is unresolvable when the thing being pointed at is absent.

## Mechanism

`crates/hearth-telegram/src/types.rs`, `struct Message`. It declares one
reply-related field:

```rust
pub reply_to_message: Option<Box<Message>>,
```

There is no field for `external_reply` and none for a `quote` object.

The struct derives `Deserialize` without `deny_unknown_fields`. That attribute
does not appear anywhere in `crates/hearth-telegram/src/`. Serde therefore
discards unknown fields silently, so a quote carried in a field the struct does
not declare is dropped during deserialisation, before any handling code runs.

Telegram represents a reply to a whole message and a reply to a highlighted span
of text differently. The first arrives as `reply_to_message`, which the struct
does model, and `blocks()` reads it:

```rust
// crates/hearth-telegram/src/poller.rs, blocks()
if let Some(context) = message.reply_to_message.as_deref().and_then(reply_context) {
    blocks.push(json!({"type": "text", "text": context}));
}
```

`reply_context()` at `poller.rs:417` formats `Replying to {author}: "{body}"`
and truncates the body at `REPLY_CONTEXT_CHARS`, which is 500. That path is
tested: `crates/hearth-telegram/tests/s5_inbound.rs:594` for text and `:637` for
a voice message.

The second form is not modelled and not tested.

## Evidence that the modelled path has not fired

From the live store at `~/.hearth/hearth.db`, 2026-11-15:

- `SELECT COUNT(*) FROM entries WHERE kind='user'` — **78**
- `SELECT COUNT(*) FROM entries WHERE kind='user' AND content_json LIKE '%Replying to%'` — **0**
- `SELECT COUNT(*) FROM telegram_updates` — **78**, earliest `processed_at` 2026-11-15 04:10:08

Every user entry in the store arrived without reply context. The
`reply_to_message` path exists, is tested, and has never produced a block in
production. Two readings are possible and this report does not decide between
them: the user's client sends the second form, or something earlier in the
ingest path drops the field.

## Not established

Which update shape the client actually sends. Settling it needs one raw
`getUpdates` body containing a quote, which this report does not have. The
candidate is `external_reply`, which carries a nested `quote` with `text` and
`is_manual`; that is the shape Telegram documents for a highlighted-span quote,
and the one the current `Message` struct cannot hold.

Whether a reply to a whole message behaves correctly. No such reply appears in
the store either, so the tested path is untested against live traffic.

Whether any other transport loses quoting the same way. Only Telegram was
examined.

## Consequence

The agent cannot tell which message is being quoted. A short follow-up that
relies on the quote is answered blind, and the loss is silent: nothing logs the
dropped field, because serde discards it before any code can observe it.

## Resolution

`Message` models `external_reply` with its nested `quote` text. `blocks` reads the
quoted span when there is no `reply_to_message` and emits `Replying to a quoted
span: "..."`, truncated at the same limit as the whole-message path.

Commit: `60f5764`. Verified by `a_quoted_span_arrives_as_reply_context`, which
sends a manual quote and requires its text in the submission blocks.
