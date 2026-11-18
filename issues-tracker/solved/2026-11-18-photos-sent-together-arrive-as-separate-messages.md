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

## Resolution

Telegram delivers an album as one message per member, all sharing a
`media_group_id`, and the poller admitted every update separately. Each photo
therefore became its own submission and turn.

The poller now stages every album member in `telegram_albums` and
`telegram_album_members`, in the same write as the update row and the offset. The
group's ready time is 1 s after its last member. The poller keeps polling while
the group waits, and a later poll submits the due group as one submission. A
staged group survives a restart. The submission keeps the first member's request
id, includes a repeated caption once, and adds each photo or file in message
order.

A caption on the second photo works. A member that arrives after the 1 s window
still starts its own submission. Telegram bounds an album at 10 members, so the
window is bounded in practice.

First fix: `79b6fb8` grouped the batch inside one poll, with `1384f70` as its
test. The durable queue replaces that wait in `16d7b19` and `c60aa7a`, with
`e615ef9` as the failing poller tests and `4598763` as the restart proof.

Verified: the same-batch test failed with two admitted submissions and passes
with one submission and two image blocks in order. The split-poll test failed
with one image and passes with two. The full `hearth-telegram` suite passes.
