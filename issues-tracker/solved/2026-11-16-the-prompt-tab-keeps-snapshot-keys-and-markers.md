# The Prompt tab keeps snapshot keys and snapshot markers

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-16 |
| Area | hearth-cli |

## Observed

The Prompt tab presents itself as a conversation view but still carries the
controls and the vocabulary of the request inspector it was rebuilt from.

Four keys exist for snapshots and raw output rather than for reading a
conversation:

| Key | Effect |
| --- | --- |
| `[` `]` | move the cursor to the previous or next saved request and scroll its marker to the top |
| `r` | replace the transcript with pretty-printed JSON of one exchange |
| `o` | reload saved prompts |

`o` duplicates work the view already does. Prompts reload on entering the tab
(`app.rs:464`) and after every agent event that the view cares about
(`app.rs:1145`).

Snapshot vocabulary appears in two further places beyond the legend:

- A numbered separator line before each request,
  `── 2 · 13:12:04 · 8f2a1b6c · 41.2k tok`, drawn by `Painter::marker`
  (`prompt.rs:966`) with a `▸` prefix on the selected one.
- `snapshot 2 of 2` in the header row, `header_line` (`prompt.rs:313`).

## Mechanism

- The keys are bound in `App::handle_prompt` (`crates/hearth-cli/src/app.rs:962`).
- `older` and `newer` (`prompt.rs:95`, `prompt.rs:106`) move `PromptView::cursor`
  and call `prompt.scroll` to the marker's line, found through
  `PromptCache::marker_line` (`prompt.rs:216`).
- Markers are collected into `PromptCache::markers` during rendering
  (`prompt.rs:1000`), each holding the line index and a second, highlighted copy
  of the separator.
- `draw_transcript` (`prompt.rs:347`) swaps the separator at the selected marker
  for its highlighted twin while copying cached lines into the frame.
- The header derives its numbers from `cursor` and `exchanges.len()`
  (`prompt.rs:313`).

## Open question

Removing the markers removes the only indication of where one saved request
ended and the next began, and this tab is also the audit surface for what
reached the model. The request that produced these changes did not settle
whether markers become a thin unnumbered divider or disappear with the keys.

## Resolution

The snapshot selection is gone. `[`, `]`, `<`, `>`, `r`, and `o` do nothing on the
Prompt tab. `PromptView` lost `cursor`, `selected`, and `raw`; `PromptCache` lost
the marker list and the marker-to-line lookup; `older`, `newer`, `jump_to_cursor`,
`marker_index`, `marker_count`, and `sync_selected` are deleted.

The numbered separator (`── 2 · 13:12:04 · 8f2a1b6c · 41.2k tok`) and its `▸`
selection twin became one faint unnumbered rule before each saved request's new
delta. The open question is settled toward the thin divider, not removal: the
merged conversation still needs a visible boundary between what one request added
and what the next added, and the rule carries no inspector vocabulary. The header
lost `snapshot N of M` and the prefix peer count; it keeps the newest request's
model, session, token total, and prefix hash.

A refresh now replaces the loaded set and leaves the reader's scroll and follow
state alone; there is no saved hash to preserve because there is no cursor. The
`a_paused_reader_keeps_its_place_on_refresh` test pins that.

Commits: `a2054a4` (test), `988fd0e` (change). The test commit failed five tests:
the two marker tests, the raw-mode test, the bracket cursor test, and the header
test, all because the old controls and vocabulary were still present. Verified by
`cargo test -p hearth-cli --lib` and `./scripts/ci.sh`. A tmux capture of the
running TUI shows the bare rule and no snapshot number, hash, or time.
