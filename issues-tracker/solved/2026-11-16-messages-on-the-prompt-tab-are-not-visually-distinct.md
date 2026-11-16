# Messages on the Prompt tab are not visually distinct

| | |
| --- | --- |
| Kind | feature |
| Reported | 2026-11-16 |
| Area | hearth-cli |

## Observed

The Prompt tab renders every message as the same shape: a label flush left, then
its text indented by `BODY_INDENT` (`prompt.rs:28`) under it, separated by a
blank line. The role is carried by the label word and by a colour, and by
nothing else.

```
you
  what is the standing task count
tool call · recall
  {"query":"standing","limit":5}
tool result
  no matches
hearth
  There are no standing tasks.
```

Two messages of different roles therefore have the same edges, the same indent,
the same left boundary, and the same inter-message gap. In a long transcript,
where the reader has scrolled, the boundary between one message and the next is
marked only by the blank line, and a body line and its label are at different
indents but the same weight. Nothing encloses a message.

## Mechanism

- Every entry is drawn through `Painter` (`crates/hearth-cli/src/ui/prompt.rs:857`),
  which appends finished lines to one flat vector. It has no concept of a
  message boundary; a message is just a run of lines.
- `Painter::label` (`prompt.rs:879`) pushes a blank line, then a row of the
  label text with an optional right-aligned cost.
- `Painter::body` (`prompt.rs:902`) wraps the text to the card width minus
  `BODY_INDENT` and prefixes each line with two spaces.
- `push_blocks` (`prompt.rs:808`) dispatches on block type and picks the style:
  `theme::think()` for thinking, `theme::warn()` for tool calls, `theme::good()`
  or `theme::bad()` for tool results depending on `is_error`, and
  `Role::style()` (`prompt.rs:471`) for text.
- The outer frame is one `Block` drawn by `render` (`prompt.rs:223`); there are
  no inner borders.
- Scrolling is line-based against this flat vector: `draw_transcript`
  (`prompt.rs:347`) slices `cache.lines` between `scroll` and `scroll + height`
  and renders the slice as one `Paragraph`.

## Constraint

Anything enclosing a message has to survive the line-based scroll and the cache.
`build_cache` (`prompt.rs:718`) stores finished `Line` values and is
invalidated only when the key changes, so a frame drawn as characters is
possible in the cached document, while a frame drawn as a ratatui `Block` is
not. Inner borders also cost horizontal space at narrow widths; the tab has an
existing test at 80 and 120 columns (`prompt.rs:1605`) and one at narrow widths
(`prompt.rs:1629`).

## Resolution

Each message now has a role bar. The label row opens with `▌` and each body line
with `▏`, both coloured by the role; a blank line closes the message. The bar
continues across the blank separator inside one message, so thinking, text, and a
tool call read as one message rather than as unrelated labels. System layers,
memory, the runtime note, and `no reply saved` are not messages: they keep the
flush-left label and the two-space body, which makes a message structurally
different from the scaffold around it.

The bar costs the two columns the body indent already spent, so the wrap width is
unchanged. The 80-, 120-, and narrow-width tests still pass, including at 12
columns.

Commits: `48ed8c2` (test), `5bc63bb` (feat). The test commit added
`a_message_is_enclosed_by_a_role_bar` and changed the two tests that used
`exact("you")` to the new label form; all three failed on the old flat document.
Verified by `cargo test -p hearth-cli --lib`, `./scripts/ci.sh`, and a tmux capture
of the running TUI, which shows `▌ you` over `▏` bodies and no bar on the system
layers.
