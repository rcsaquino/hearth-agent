# The activity viewport leaves the last row when a new event arrives

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-cli` |

## Observed

In the Activity tab, the selection follows new events only while the feed is
unpaused:

```rust
// crates/hearth-cli/src/app.rs, AppEvent::Live
if !self.activity_paused {
    self.activity_selected =
        crate::ui::activity::row_count(&self.events).saturating_sub(1);
}
```

The viewport is a separate decision, in `list_start()`
(`crates/hearth-cli/src/ui/activity.rs:181`):

```rust
pub(crate) fn list_start(selected: usize, total: usize, height: usize) -> usize {
    if total > 0 && selected + 1 == total {
        (selected + 1).saturating_sub(height.max(1))
    } else {
        selected.saturating_sub(height.saturating_sub(1) / 2)
    }
}
```

The window is bottom-anchored only when the selection is the final row. For any
other row it centres on the selection.

## Mechanism

Every keypress in the feed pauses it. `Up`, `Down` and `g` each set
`activity_paused = true` in `App::handle_activity`
(`crates/hearth-cli/src/app.rs:782`). None of them move the selection to the end
of the list.

While paused, `AppEvent::Live` still appends:

```rust
self.events.push_back(value);
while self.events.len() > MAX_EVENTS {
    self.events.pop_front();
}
```

The selection does not move. The row count grows by one. The condition
`selected + 1 == total` that held a moment ago no longer holds, so
`list_start()` switches from the bottom-anchored branch to the centring branch.
The highlighted row jumps from the bottom of the pane to its middle, and the
list shifts under the reader.

Each further event repeats the effect for as long as the selection stays one
short of the end.

A reader who presses `Down` to reach the newest row does not escape this. The
press pauses the feed, so the very next event moves the window again.

## Requested behaviour

When the selected row is the last row, the viewport stays with it. An incoming
event must not move the window away from a selection already at the end of the
list, whether or not the feed is paused. The selection should follow the newest
row in that case.

When the selection is not the last row, current behaviour is correct: the reader
is inspecting something older and the window should stay where it is.

## Not established

Whether the centring branch for a paused feed is deliberate. No comment in
`list_start()` or `handle_activity()` explains it.

No test covers the paused case. The tests at
`crates/hearth-cli/src/ui/activity.rs:437` cover the bottom-pinned branch
(`list_start(19, 20, 6)`), one centred case (`list_start(10, 20, 6)`), and the
empty cases only.

## Consequence

The feed is not stable to read. A reader who scrolls up to inspect an event
loses their position as new events arrive, and the only way back to the newest
row is a key that pauses the feed and so reproduces the same drift.

## Resolution

`Home` selects the oldest row and pauses, `End` selects the newest row and resumes
following, and the footer lists the pair. A paused feed whose selection is the
last row now follows a new event instead of leaving it behind; the centring
behaviour for an older selection is unchanged.

Commit: `0f9a863`. Verified by `home_and_end_select_the_ends_of_the_activity_feed`
and `a_paused_feed_keeps_its_last_row_selected` in `crates/hearth-cli/src/ui/mod.rs`.
