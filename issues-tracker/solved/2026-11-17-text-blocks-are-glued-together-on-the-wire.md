# Consecutive text blocks are glued together on the wire

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-model` |

## Observed

`openai::text_blocks` collects `ContentBlock::Text` and file notes and joins them
with the empty string. `OpenAiCompatibleProvider::body` uses that result for a
system or user message that has no image. The prompt model keeps message blocks
separate, and `TelegramPoller::blocks` emits a quoted reply as one text block and
the actual message as a second.

A reply to a Telegram message therefore reaches the provider as
`Replying to Alice: "the old text"What does this mean?`, with no whitespace
between the quote's closing quote and the new message. A message with a file note
gets the note appended directly to the preceding text for the same reason.

The model can usually parse the result, but the boundary is real information. It
is also inconsistent with the audit document, which stores the blocks separately.

## Verification

Build a `ChatRequest` with one user message holding two text blocks, `alpha` and
`beta`, and call `OpenAiCompatibleProvider::wire_body`. The content is
`alphabeta`. The same shape appears by replying to a Telegram message and
inspecting the saved prompt document's transcript versus the provider request.

## Resolution

`openai::text_blocks` joins text blocks and file notes with a newline, so the
wire body keeps the boundary the prompt model and the audit document already
store.

Commits: `bc63ba5` (test), `a6ded04` (fix). Verified by
`consecutive_text_blocks_keep_their_boundary`, which sends two text blocks and
requires the wire content to be `alpha\nbeta`.
