# The working indicator looks out of place

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-16 |
| Area | hearth-cli |

## Observed

The Prompt tab shows a single animated braille glyph and the word `working` on a
line of its own, inside the card, under the transcript and above the key hints.
It reads as detached from the conversation it is describing. It does not sit
with the message being written, and nothing else on the screen moves, so the one
moving element draws the eye away from the text.

The indicator also appears when the agent is not working. `busy` is
`app.prompt.loading || running_tasks(app) > 0`. `prompt.loading` is the state of
a background reload of saved prompts, not the state of a turn, and the view
reloads on entering the tab (`app.rs:464`) and after every agent message
(`app.rs:1145`). Opening the tab therefore animates `working` while nothing is
running.

## Mechanism

- Frames are a fixed list of eight braille characters, `FRAMES`
  (`crates/hearth-cli/src/ui/prompt.rs:30`).
- `working_line()` (`prompt.rs:377`) builds one line: the frame in
  `theme::accent_style()`, then `working` in `theme::muted()`.
- The frame index comes from `spinner_frame()` (`prompt.rs:394`), which divides
  the elapsed time since a process-wide `OnceLock<Instant>` initialised on first
  call. The phase is tied to when the process started, so the glyph position at
  the moment a turn begins is arbitrary.
- `FRAMES[spinner_frame()]` is evaluated during the draw, so the animation
  advances only because `run_loop` (`app.rs:1514`) redraws on every pass of a
  100 ms poll.
- The row is laid out separately from the body: `busy_rows` is computed at
  `prompt.rs:243` and the line is drawn into `busy_area` at `prompt.rs:293`,
  outside the scrolled region and outside the cached document.

## Constraints

The document is cached as finished lines by `build_cache` (`prompt.rs:718`) and
invalidated by a key of exchanges, width, `raw`, `show_system`, and zone
(`CacheKey::new`, `prompt.rs:181`). Anything that animates inside the transcript
cannot be part of that cache without invalidating it every 100 ms. The existing
indicator avoids this by living outside the cache.

## Verification

The terminal cannot be judged from source. Capture the running TUI and read the
rendered cells: run the binary under `tmux` with a real `TERM` and capture with
`tmux capture-pane -e -p`, which preserves characters and colours. `TERM` in a
non-interactive shell here is `dumb`, so a tmux session is the only observed
way to get a real `TERM` for the TUI. `ffmpeg` and ImageMagick `import` are installed if
a raster image is wanted.

## Resolution

The detached `⠋ working` row is gone. The spinner is now the gutter glyph on the
first label of the live turn, so it sits on the message being written:
`⠋ hearth` over `▏` body lines. The phase comes from the live turn's own
`started` instant, set when `turn_started` arrives, instead of a process-wide
`OnceLock<Instant>`, so the first frame is the first frame of the turn.

`busy` is gone. `prompt.loading` and `status.tasks_running` no longer animate
anything: a background reload is not a turn, and a scheduled task is not this
conversation. `a_refresh_alone_does_not_animate_working` pins both cases.

The spinner lives in the live region, which is drawn outside the cached document,
so it does not invalidate the cache every 100 ms. When the reader pauses, the live
turn stays below the viewport and the title still says `paused`.

Commits: `e0da201` (test), `da99603` (feat). The test commit failed because the
live turn was not drawn and the old loading-based line still appeared. Verified by
`cargo test -p hearth-cli --lib`, `./scripts/ci.sh`, and a tmux capture of the
running TUI, which shows the live label with the spinner in the gutter and no
`working` row.
