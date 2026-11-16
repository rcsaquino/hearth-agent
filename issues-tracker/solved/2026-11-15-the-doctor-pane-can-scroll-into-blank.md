# The Doctor pane can be scrolled into blank and stays there

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-cli` |

## Observed

The Doctor tab's scroll offset has no upper bound, and the render skips that many
lines without clamping. Pressing Down past the end shows an empty pane, and the
offset survives the next check, so a short report remains invisible until the
user presses Up the same number of times.

`crates/hearth-cli/src/app.rs:805`:

```rust
KeyCode::Enter if !self.doctor.running => {
    self.doctor.running = true;
    self.doctor.live = false;
    self.doctor.output = None;
    self.workers.spawn_doctor(self.home.clone(), false);
}
...
KeyCode::Up => self.doctor.scroll = self.doctor.scroll.saturating_sub(1),
KeyCode::Down => self.doctor.scroll += 1,
```

`crates/hearth-cli/src/ui/doctor.rs:34`:

```rust
let lines: Vec<&str> = output.lines().skip(app.doctor.scroll).collect();
```

Neither the Enter arm nor the `l` arm resets `scroll`. The Skills pane clamps
(`crates/hearth-cli/src/ui/skills.rs:90`,
`app.skills.scroll.min(max_scroll)`), so the pattern exists elsewhere.

## Consequence

The tab that runs the integrity, schema, lock, settings, and credential checks
can display nothing with no indication that output exists. A user who scrolls
while waiting and then runs a new check sees a blank screen and may conclude the
check failed silently.

## Verification

Set `doctor.output` to a three-line string and `doctor.scroll = 50`, render to a
`TestBackend`, and assert the text does not appear. Then send Enter to start a
new check and assert `doctor.scroll == 0`. Today the text is absent and the
scroll survives.

## Resolution

The Doctor render clamps its offset to the line count and both check keys reset
the offset to zero, so a scroll past the end cannot hide a short report.

Commit: `4204eee`. Verified by `a_scrolled_doctor_pane_stays_on_its_text`, which
sets an offset of 50 on three lines and requires the text on screen, then presses
Enter and requires offset 0.
