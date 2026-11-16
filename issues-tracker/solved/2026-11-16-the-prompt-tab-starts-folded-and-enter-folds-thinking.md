# The Prompt tab starts folded and Enter folds thinking

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-16 |
| Area | `hearth-cli` |

## Observed

The Prompt tab has two fold states. `PromptView` in `crates/hearth-cli/src/app.rs`
holds `collapsed` and `thinking_collapsed`, both false by default. `handle_prompt`
maps `t` to `toggle_thinking` and Enter to `toggle_fold`.

`Fold::open` in `crates/hearth-cli/src/ui/prompt.rs` gives thinking its own state:
`Fold::Thinking` reads `thinking_collapsed`, while the chat sections read
`collapsed`. The agent's answer reads neither and never folds. The live turn
follows the same split through `live_lines`: live thinking folds on `t`, and the
live reply stays open.

The footer in `crates/hearth-cli/src/ui/mod.rs` names `t thinking`. The behaviour
is recorded in the Draft 0.40 note and the 6.6 bullets in `docs/SPEC.md`, and in
the DECISIONS entry "Thinking owns a block and a fold key".

## Wanted

One fold state, folded at the start:

- The first render shows every foldable section as its five-line preview with the
  hidden-count stub. Thinking, including live thinking, starts previewed.
- Enter toggles the whole view. From the folded start it opens every foldable
  section, thinking included; the next Enter folds them again.
- The agent's answer never folds, and the live reply stays open. Enter folds the
  transcript around them, not the text the person is reading.
- `t` is removed. It becomes an ordinary unbound key in the Prompt tab, and the
  footer no longer names thinking.
- The default applies when the app starts. A refresh, a new exchange, or a tab
  switch does not reset the fold the person chose.

The five-line preview, the hidden-count stub, follow, scroll, and the cache
behaviour stay as they are. Short sections still show in full and get no stub.

## Why

One key is simpler than two. Thinking is part of the transcript, and the person
has to remember which fold hides which lines; the two keys move apart even though
the desired reading state moves together. Folding the thinking with the chat also
means the first screen is a compact outline of the prompt rather than a wall of
context, and Enter reveals it in one press.

This intentionally reverses two earlier choices, while keeping the newer block
structure:

- The separate `t` key and `thinking_collapsed` flag from "Thinking owns a block
  and a fold key" go. Thinking stays its own block; it no longer owns a separate
  fold key.
- The start-unfolded choice from "The Prompt tab folds as a whole" goes. The view
  now starts folded, so the first screen is the conversation outline rather than
  the whole prompt.

## Scope

- Keep thinking as its own `THINKING` block, with its own header, gutter, turn,
  and request time. Only its fold control changes.
- Keep the answer and the live reply exempt from folding.
- Remove the second fold flag and its cache-key member. A saved or live thinking
  block reads the same collapsed state as the chat sections.
- Do not change the five-line preview, the stub, follow, scroll, or the cached
  line set. `t` must not scroll, follow, or change any other tab.
- The footer keeps one fold hint and loses the thinking hint.

## Test

On the first render of a saved chat with a long tool result, a long context
layer, and a thinking block:

- The tool result shows its first five lines and a hidden-count stub.
- The thinking block shows its first five lines and a stub.
- The answer is fully visible.
- The folded view contains no sixth line from those sections.

Press Enter and assert the whole tool result, the whole context layer, and the
whole thinking block are visible. Press Enter again and assert they return to
their previews. In both states the answer stays visible.

Send `t` and assert the rendered view does not change. Check the footer names
`fold` and not `thinking`.

For the live turn, start a stream with long thinking and text. The folded start
shows the thinking preview and the full live reply. Enter opens the thinking;
Enter folds it again; the reply stays visible throughout.

Existing tests that assume an open start need to flip their first assertion:

- `enter_folds_and_opens_the_whole_chat` becomes the folded-start test above.
- `thinking_is_its_own_block` must open the view before looking for its last
  thought, or assert the preview first.
- `thinking_folds_on_its_own_key` is replaced by the Enter test; `t` moves into
  the ignored-key test beside `n`, `p`, `e`, and `c`.
- `a_short_section_survives_a_fold` asserts the folded start, then Enter opens.
- `a_saved_conversation_is_a_chat_of_labelled_blocks` has an "identity is open
  until the chat folds" assertion and comment that no longer hold; open the view
  first or adjust them to the folded start.
- `prompt_tab_shows_the_whole_system_layers` in
  `crates/hearth-cli/src/ui/mod.rs` must open the view before asserting the whole
  identity and the tool argument descriptions.
- `the_live_turn_is_a_streaming_block_after_the_saved_chat` follows the live
  behaviour above.
- `the_prompt_footer_is_the_only_key_legend` in `crates/hearth-cli/src/ui/mod.rs`
  stops expecting `thinking`.

## Documents

- Bump the Draft and add a note at the top of `docs/SPEC.md`.
- Update the 2.3 Prompt row, the 6.6 fold bullets, the footer sentence, and the
  live-thinking sentence.
- Add a DECISIONS entry that records the one-key fold and the folded start, and
  says which earlier entries it supersedes. Keep the thinking-block decision from
  Draft 0.40.

## Resolution

`PromptView` has one `collapsed` flag, default true. `thinking_collapsed`,
`toggle_thinking`, and the `t` binding are gone, and the cache key carries one
fold state. `Fold::Thinking` remains only as the thinking ink; `Fold::open`
treats it like `Fold::Chat`. The first render previews every foldable section,
thinking included, and Enter opens or folds them together. The answer and live
reply stay open. The footer keeps `fold` and drops `thinking`.

Commits: `1996d06` (test), `ac761f3` (feat), `6ab3421` (docs).

Verified by `the_chat_starts_folded_and_enter_opens_it`,
`thinking_starts_folded_and_enter_toggles_it`,
`a_short_section_survives_a_fold`, and
`the_live_turn_is_a_streaming_block_after_the_saved_chat` in
`crates/hearth-cli/src/ui/prompt.rs`; by
`prompt_tab_shows_the_whole_system_layers`,
`prompt_arrows_scroll_the_document_and_end_follows_the_tail`, and
`the_prompt_footer_is_the_only_key_legend` in `crates/hearth-cli/src/ui/mod.rs`;
and by `unbound_prompt_keys_change_nothing`, which now includes `t`.
`./scripts/ci.sh` passes.
