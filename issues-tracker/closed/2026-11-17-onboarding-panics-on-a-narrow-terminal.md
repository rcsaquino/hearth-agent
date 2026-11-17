# Onboarding panics on a narrow terminal

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-cli` |

## Observed

`ui::render` dispatches `Screen::Onboarding` to `ui::onboarding::render` before
it checks the minimum size. The onboarding model list computes its label width as
`area.width as usize - 4` before calling `short`. A terminal narrower than four
columns makes the subtraction underflow. Debug builds panic; release builds wrap
to a huge width and render a wrong line.

The dashboard has a `resize` screen for a small terminal, but onboarding has no
equivalent guard. The first-run wizard is the one screen a new user sees, and a
small window or a terminal multiplexer pane can reach this path.

## Verification

Render onboarding at the `Step::Model` step with a ratatui `TestBackend` of
width 3. The debug test process panics on the subtraction. The existing UI tests
use the default 80x24 backend and do not cover a narrow onboarding frame.

## Closure

Not reproducible. `ui::onboarding::render` returns before it lays out any step
when the frame is narrower than 70 columns or shorter than 20 rows. The model
list and the `area.width as usize - 4` subtraction in `render_step` are only
reached through that function, after the guard, so a three-column frame draws
the enlargement message instead of underflowing. The guard has been in
`ui/onboarding.rs` since the terminal application replaced the CLI.
