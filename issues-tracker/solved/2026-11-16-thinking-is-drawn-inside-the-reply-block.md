# Thinking is drawn inside the reply block

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-16 |
| Area | `hearth-cli` |

## Current

A thinking block becomes a `Part` of the assistant `Block`, alongside the reply
text and tool calls. `parts_of` in `crates/hearth-cli/src/ui/prompt.rs` maps a
`thinking` block to a part labelled `thinking` with `Fold::Thinking`.

So thinking shares the block's border, its `HEARTH` label, its `turn N · time`
meta line, its background, and its accent with whatever it reasons about. The
same happens in the live turn, where `live_lines` emits thinking through
`body_block` above the streaming answer.

Thinking is also excluded from the decisions the block header makes: `spoken`
filters thinking out before the label and error state are chosen by
`response_block`, so a block labelled `HEARTH` can open with thinking, and a
block labelled `TOOL CALL` can contain thinking as well. `Fold::Thinking` only
opens together with the chat fold in `Fold::open`, so thinking cannot be opened
independently of the message it belongs to.

## Desired

Thinking is its own block: its own header, its own border, its own fold, and its
own position in the transcript, separate from the reply and from tool calls. It
opens and closes on its own.

This applies to a saved answer and to the turn being written. The reasoning is a
different kind of content from the answer — `parts_of` is also where thinking is
kept out of what is sent back — and the view should not make it look like part
of the reply.

## Documented elsewhere

The ordering tests assume thinking shares the transcript with the reply rather
than owning a block of its own: `thinking comes before the answer it precedes`,
`thinking comes before the text it precedes`, and
`enter_folds_thinking_with_the_chat`.

## Verification

Open the Prompt tab on a turn that contains thinking and a reply. Thinking has
its own header and border, folding it does not fold the reply, and folding the
reply does not hide the thinking. The same holds for a live turn while it
streams.

## Resolution

`derive` splits each message with `split_thinking`. Thinking becomes its own
`THINKING` block through `thinking_block`, with its own header, gutter, turn, and
request time; the spoken blocks keep their `HEARTH`, `TOOL CALL`, `YOU`, or
`TOOL RESULT` label without it. `response_block` and `message_block` no longer
receive thinking, so a `HEARTH` block cannot open with it and a `TOOL CALL` block
cannot contain it. `parts_of` no longer maps a thinking part.

`t` folds every thinking block and Enter folds the chat.
`PromptView.thinking_collapsed` is a second fold flag, the prompt cache key carries
it, and the live turn paints the same block through the same `Painter`, so saved
and streaming thinking look alike. This reverses the earlier one-key decision now
that thinking is a block of its own.

Commits: `703c49f` (test), `7cd37f5` (feat), `d46d253` (docs).

Verified by `thinking_is_its_own_block` and `thinking_folds_on_its_own_key` in
`crates/hearth-cli/src/ui/prompt.rs`, and by the updated
`the_live_turn_is_a_streaming_block_after_the_saved_chat`, which asserts that
Enter leaves live thinking open and `t` previews it without touching the reply.
The old `enter_folds_thinking_with_the_chat` was replaced by the new fold contract,
as the issue anticipated. `./scripts/ci.sh` passes.
