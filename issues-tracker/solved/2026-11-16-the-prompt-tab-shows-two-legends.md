# The Prompt tab shows two legends

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-16 |
| Area | hearth-cli |

## Observed

The Prompt tab has a key legend inside its card and a second key legend in the
application footer. No other tab has an in-card legend. The two disagree about
which keys exist.

Inside the card (`prompt.rs:400`, drawn at `prompt.rs:296`):

```
[ ] snapshots · f follow · s system · r raw · o refresh
```

In the footer (`crates/hearth-cli/src/ui/mod.rs:177`):

```
↑↓ scroll · pgup/pgdn page · home/end top/tail · ? help · q quit
```

The in-card legend omits the scrolling keys and the footer omits every Prompt
tab key except scrolling. A reader who needs a key that is not on the screen
they are looking at has to read both.

## Mechanism

- `hint_line()` (`prompt.rs:400`) builds the in-card row from `HINT_KEYS`
  (`prompt.rs:32`), a four-entry table of follow, system, raw, and refresh, with
  a hard-coded `[ ]` prefix for the two bracket keys.
- `render()` (`prompt.rs:223`) reserves `hint_rows` at `prompt.rs:244` and draws
  the row at `prompt.rs:296`, inside `card.inner(area)`.
- The footer is drawn by `ui::render` from a match on the active tab
  (`ui/mod.rs:177`), the same match that provides the legend for every other
  tab.
- The footer keeps one row whether or not the in-card row exists, so removing
  the inner legend returns a body row to the transcript.

## Resolution

The card no longer draws a key legend. `prompt::render` lays out the header and the
transcript only, and the row the hint line owned belongs to the transcript. The
footer is the one legend for every tab, and the Prompt row names its keys there.
At this commit the row was `↑↓ scroll · pgup/pgdn page · [ ] snapshots · f follow ·
s system · r raw · q quit`; the later snapshot and block work replaced the last
five entries with `n/p block · ↵ toggle` and kept the tab-specific keys in the
footer.

The footer row is one row whether or not a card legend exists, so the transcript
gained the freed line at every height.

Commits: `c490bd6` (test), `d42d657` (fix). Both new tests failed on `c490bd6` for
the expected reasons: the card still printed `[ ] snapshots · ...` and the footer
did not contain `snapshots`, `follow`, `system`, or `raw`. Verified by
`cargo test -p hearth-cli --lib` and `./scripts/ci.sh`, and by a tmux capture of
the running TUI at 80 and 120 columns, where the footer is the only legend.
