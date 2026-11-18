# Photos sent together arrive as separate messages

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-18 |
| Area | `Telegram` |

## Observed

When I select several photos in Telegram and send them at once, Telegram delivers
them as one album, but the assistant receives them as separate messages. Each one
starts its own turn, so the assistant sees two or three unrelated prompts and can
answer each one separately.

Today I sent two photos together with a question, "Does this warrant ERCP?".
The first arrived as one message with the text and the first photo. The second
arrived about a second later as a second message with just the photo. The
assistant answered the first and then answered the second on its own, as though
I had sent the picture again with nothing else to say.

What should happen is that an album is delivered to the assistant as one message,
with all of its photos in the order I sent them and the caption attached once.
One album should produce one turn and one reply.

## Verification

1. Send two or three photos together, with a caption or a follow-up line.
2. The assistant receives one message per photo instead of one message per album.
3. The assistant replies more than once for what the user sent as a single
   message.

## Notes

This is worse than cosmetic. The split changes what the model is answering: a
question about "this" becomes a question about one of three images with no way to
tell which. It also multiplies the replies, and it multiplies any tool calls made
per reply, so a single send can write the same file twice or run the same lookup
twice.

Related to the delivery bug in
`a-reply-sent-once-is-delivered-again-after-a-restart`. Two submissions sharing
one answer, which is what I reported there, is the shape that arises when
separate messages are answered together. Grouping on the way in would remove that
shape entirely.

Grouping needs a short wait: an album's members arrive within a few hundred
milliseconds of each other, so collecting them for a moment before admitting the
turn is enough. Not established what the right window is, or what happens to a
caption that arrives on the second photo rather than the first.
