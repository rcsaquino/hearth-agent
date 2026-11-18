# A reply already sent is sent again after a restart

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-18 |
| Area | `Telegram` |

## Observed

I sent two messages to the bot a fraction of a second apart: the first had text
and a photo, the second had only a photo. Both were answered together in one
reply, and that reply arrived in the chat once, at 20:44.

The daemon kept running. Sixteen hours later, at 12:37 the next day, I stopped
it and started it again. On startup the same reply was sent to the chat a second
time, prefixed with the words "Recovered reply:". Nothing had been asked in
between, and the rest of the text was word for word identical to the first copy,
down to the workout tables it reported.

What should happen is that a reply already delivered is never delivered again,
regardless of a restart. Nothing was owed, so nothing should have been resent.

## Verification

1. Send two messages in quick succession so one reply covers both, for example
   a photo with a caption followed by a second photo.
2. Confirm the reply arrives once.
3. Stop the daemon and start it again.
4. The reply arrives again on startup, prefixed "Recovered reply:", with no
   prompt from the user.

## Notes

Two photos sent back to back is ordinary usage, so the pairing is worth testing
deliberately. I hit it on my first day of using the feature.

Not established: whether the gap between the two messages matters, whether a
longer or shorter one avoids it, or whether the age of the reply before the
restart matters. All of those were fixed in this instance.

The "Recovered reply:" prefix suggests the restart path believed the reply had
never been sent, when in fact it had been sent and shown in the chat.

## Related

`a-crash-before-the-delivery-intent-loses-the-answer` made a settled answer
deliverable on startup when no delivery was recorded for it. This looks like the
same recovery deciding that a reply was owed when it had in fact been sent, so
the two should be read together.
