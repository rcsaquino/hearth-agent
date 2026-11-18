# Replying to the bot's own message drops the quote

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-18 |
| Area | `Telegram` |

## Observed

In Telegram I hold a message and choose Reply, which attaches the quoted message
above what I type. Whether the quote reaches the assistant depends on whose
message I quote.

- Replying to a message I sent myself works. The assistant received a line
  reading `Replying to Rey: "Okay let me test something"` above my question, and
  answered correctly.
- Replying to a message the bot sent does not. The assistant received only the
  typed text, with nothing above it, and had no idea which message I meant.

I tested this deliberately, one of each kind, a minute apart. Both were genuine
Telegram replies made the same way on my phone, and the screenshots show the
quote bubble in the chat in both cases. So the difference is not how I replied,
it is who wrote the message being quoted.

What should happen is that the quote reaches the assistant in both cases. The
bot's own messages are the ones I most often want to point at, because they are
the ones that can be wrong.

## Verification

1. Reply to a message the bot sent, with text like "why did you send this?".
2. The reply arrives with no quote context, and the assistant cannot tell which
   message is meant.
3. Reply to your own earlier message. The quote arrives normally.

## Notes

This cost a real answer. I asked for a report about a resent message, quoted the
resent message, and the report was written about the wrong message because the
quote was invisible.

Hard to notice from the assistant's side, since the typed text still reads as
complete English. Nothing signals that anything was dropped.
