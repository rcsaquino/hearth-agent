# The activity feed has no key for the newest row or the oldest

| | |
| --- | --- |
| Kind | feature |
| Reported | 2026-11-15 |
| Area | `hearth-cli` |

## Requested

Bind `Home` and `End` in the Activity tab.

- `Home` selects the oldest row and pauses the feed.
- `End` selects the newest row and returns the feed to following.

The footer hint bar lists the Activity keys and will need a matching update:
`crates/hearth-cli/src/ui/mod.rs:157`.

## Current state

`App::handle_activity` (`crates/hearth-cli/src/app.rs:782`) matches `Up`,
`Down`, `p` and `g`. It does not match `KeyCode::Home` or `KeyCode::End`.

`g` selects the oldest row and pauses:

```rust
KeyCode::Char('g') => {
    self.activity_selected = 0;
    self.activity_paused = true;
}
```

`p` toggles the pause without moving the selection:

```rust
KeyCode::Char('p') => self.activity_paused = !self.activity_paused,
```

So there is no key that selects the newest row, and no key that resumes
following without also waiting for the next event. Unpausing leaves the
selection where it was until `AppEvent::Live` arrives and moves it, which means
the feed can be unpaused with the viewport still mid-list.

## Precedent

`Home` and `End` are already bound elsewhere, and the prompt view pairs `Home`
with `g` for the same action (`crates/hearth-cli/src/app.rs:936`):

```rust
KeyCode::Home | KeyCode::Char('g') => ui::prompt::scroll_top(&mut self.prompt),
KeyCode::End | KeyCode::Char('G') => ui::prompt::scroll_bottom(&mut self.prompt),
```

The Skills tab binds both to its scroll offset at `app.rs:857`. Adding the same
pair to Activity is consistent with both.

## Related

`open/2026-11-15-activity-viewport-leaves-the-last-row.md` covers the viewport
losing a selection that is already at the end of the list. `End` is partly a
workaround for that fault, but the fault is separate and remains.

## Resolution

`Home` selects the oldest row and pauses, `End` selects the newest row and resumes
following, and the footer lists the pair. A paused feed whose selection is the
last row now follows a new event instead of leaving it behind; the centring
behaviour for an older selection is unchanged.

Commit: `0f9a863`. Verified by `home_and_end_select_the_ends_of_the_activity_feed`
and `a_paused_feed_keeps_its_last_row_selected` in `crates/hearth-cli/src/ui/mod.rs`.
