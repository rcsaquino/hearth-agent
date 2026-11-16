# Long blocks cannot be collapsed

| | |
| --- | --- |
| Kind | feature |
| Reported | 2026-11-16 |
| Area | hearth-cli |

## Observed

Every block is printed in full. A tool result that returns a large file, a long
thinking block, or a page of JSON occupies the transcript for hundreds of lines
and pushes the surrounding conversation off the screen. There is no way to read
past one without scrolling through it.

The only collapse that exists is for the whole system layer group, bound to `s`
(`app.rs:969`), which switches `PromptView::show_system` and makes `push_system`
(`prompt.rs:773`) print layer names and token costs instead of text. Per-block
collapse does not exist, and there is no selection the reader can move.

## Mechanism

- `push_blocks` (`crates/hearth-cli/src/ui/prompt.rs:808`) always calls
  `Painter::body` with the full text for `text`, `thinking`, `tool_call`
  (arguments), `tool_result`, `file`, and `image` blocks.
- The painter appends all wrapped lines unconditionally:
  `Painter::body` (`prompt.rs:902`) loops over `wrap_text` output with no limit.
- `PromptView` (`crates/hearth-cli/src/app.rs:109`) has no selected-block field
  and no expanded set. Its `selected` field is documented as the snapshot marker
  nearest the top of the viewport, so it is a scroll position, not a cursor over
  content.
- `derive` (`prompt.rs:479`) walks the saved requests and turns them into `Entry`
  values; each `Entry::Message` holds the raw block array (`Entry`, `prompt.rs:413`).
  Blocks therefore already exist as addressable units before they are painted.
- Arrow keys currently scroll: `Up`/`k` and `Down`/`j` call `scroll_up` and
  `scroll_down` (`app.rs:966`).

## Consequence for the cache

`CacheKey::new` (`prompt.rs:181`) holds exchanges, width, `raw`, `show_system`,
and zone. An expanded set is not part of the key, so the cache has no way to
represent the same document with one block open and another closed. Adding a
selection or an expanded set without extending that key leaves the view
rendering stale lines.

## Resolution

A message block whose wrapped body exceeds 12 lines starts folded to its first
line and a faint stub, `… N lines hidden · enter to open`. `n` and `p` move a
block cursor over the long blocks and Enter opens or closes the block under it.
The selection paints the label row in `theme::selected()`.

`Tab` was the first choice and was rejected during implementation because the
application binds Tab and BackTab to switching sections; `n` and `p` are free on
this tab and the footer names them. System layers keep the `s` group fold and are
not part of the cursor.

Block identity is the prompt hash plus the message index and the block index, so a
refresh that appends a newer request leaves an older block's key and open state
intact. The open-key set is in `CacheKey`; the selected key is not, because the
draw swaps in a prebuilt emphasized label row. Moving the cursor or toggling a
fold therefore rebuilds only when a fold actually changes.

Commits: `2bc2396` (collapse tests), `c74e120` (highlight test), `4e1f92b`
(feat). `2bc2396` failed on the old always-full body; `c74e120` failed because `n`
selected nothing. Verified by `cargo test -p hearth-cli --lib` and
`./scripts/ci.sh`. A tmux capture of the running TUI shows the folded tool result
with the `30 lines hidden` stub, the highlighted `▌ tool result` label after `n`,
and the full body after Enter.
