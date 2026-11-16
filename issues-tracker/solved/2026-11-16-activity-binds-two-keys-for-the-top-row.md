# The Activity tab binds two keys that do the same thing

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-16 |
| Area | hearth-cli |

## Observed

The Activity tab offers `g` for the top of the feed and `home/end` for the ends.
`Home` and `g` take the same action, so the footer advertises one behaviour twice.

The footer at `crates/hearth-cli/src/ui/mod.rs:157`:

```rust
Tab::Activity => theme::hints(&[
    ("↑↓", "select"),
    ("p", "pause"),
    ("g", "top"),
    ("home/end", "ends"),
    ("q", "quit"),
]),
```

The handler at `crates/hearth-cli/src/app.rs:802`:

```rust
KeyCode::Home => {
    self.activity_selected = 0;
    self.activity_paused = true;
}
KeyCode::Char('g') => {
    self.activity_selected = 0;
    self.activity_paused = true;
}
```

The two arms are identical. `Home` selects the first row and pauses; `g` selects
the first row and pauses.

`Home` and `End` were added on 2026-11-15 in `0f9a863`. `g` was already there and
was left in place. The footer gained a second line for the pair, so the same
action now has two hint entries.

## Requested

One key for the top of the feed, and one hint for it.

`End` is not part of the duplication. It selects the newest row and resumes
following, which nothing else in the tab does.

`Home` also is not redundant on its own. The choice is between dropping `g` and
dropping the `Home` half of the pair. Either leaves one route to the top row.

## Constraint

The pair `Home`/`End` reads as a unit to a reader, and it matches the Prompt tab,
which also advertises `home/end` as one hint
(`crates/hearth-cli/src/ui/mod.rs:181`). A footer that says `home/end · ends`
teaches both keys at once. Dropping `Home` while keeping `End` would leave the
footer naming one half of a familiar pair.

`g` has no test in the Activity tab. The tests added in `0f9a863` press `Home`
and `End` only (`crates/hearth-cli/src/ui/mod.rs:412`), so dropping `g` breaks
nothing that is covered. Dropping `Home` would break
`home_and_end_select_the_ends_of_the_activity_feed`, which asserts the `Home`
behaviour directly.

## Not established

Whether `g` is wanted as a habit from other tools. The Memory tab binds `g` to
select the first row (`crates/hearth-cli/src/app.rs:780`), and the Prompt tab
binds `Home | g` to the same action in one arm (`app.rs:982`). A reader who
learns `g` in one tab may expect it in the others, which is an argument for
keeping `g` here and dropping the `Home` half of the footer hint instead.

## Resolution

Fixed in `fdd4ebf`, with the failing tests in `927b844`.

**What changed.** `g` is gone from the Activity tab. `Home` and `End` stay as the
one pair, which matches the Prompt tab's `home/end` hint and keeps the familiar
pair intact. The footer now reads `home/end ends`, and `handle_activity` has one
arm for the top row.

**Verification.** `g_is_not_a_second_route_to_the_top_of_the_activity_feed` fails
on `927b844` with the selection at 0 instead of 1, and
`the_activity_footer_names_one_key_for_the_top` fails because the footer still
contained `g  top`. Both pass on `fdd4ebf`.
`home_and_end_select_the_ends_of_the_activity_feed` still passes, and
`cargo test -p hearth-cli --lib` passes 110 tests.
