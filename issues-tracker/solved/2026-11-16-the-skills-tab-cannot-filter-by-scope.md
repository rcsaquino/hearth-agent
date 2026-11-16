# The Skills tab cannot filter by scope

| | |
| --- | --- |
| Kind | feature |
| Reported | 2026-11-16 |
| Area | `hearth-cli` |

## Current

The list always shows both directories at once, user skills first and then
learned ones, with the scope printed per row by `render` in
`crates/hearth-cli/src/ui/skills.rs`. There is no filter. `[` and `]` are bound
to nothing in the Skills tab: `App::handle_skills` covers arrows, `v`, `enter`,
`pgup`/`pgdn`, `home`/`end`, `n`, and `e`, and nothing else.

With 50 active skills allowed by `INDEX_MAX_ACTIVE` in
`crates/hearth-runtime/src/skills.rs`, the library is a long undifferentiated
list, and the learned skills — the ones that change on their own — are
interleaved with the ones the person wrote.

## Desired

`[` and `]` cycle the view through three modes: both, user only, learned only.
Both is the default. The mode is visible, so a filtered list cannot be mistaken
for an empty library, and the selection stays on a valid row when the filter
changes.

The footer names the keys, and the empty state says which mode is empty rather
than saying there are no skills at all.

## Constraint

The list is the index the selection refers to: `SkillsView.selected` indexes
`SkillsView.skills`, and the body pane is loaded from the selected row by
`SkillsView::load_selected`. A filter that changes the list without re-resolving
the selection would leave the body showing a skill that is no longer listed.

## Related

`2026-11-16-the-skills-tab-offers-edit-new-and-history-keys.md` — the empty-state
line in `render` is the same line, so both changes touch it.

## Verification

With at least one user skill and one learned skill, press `]` and `[`. The list
cycles both, user only, learned only; the header names the mode; the body always
matches the selected row; and a mode with no skills says so.

## Resolution

`SkillFilter` cycles both, user, and learned. `]` moves forward and `[` moves back.
`SkillsView` keeps the full library in a private `all` and derives `skills` from the
filter, so the visible list stays the index the selection refers to; `show` clamps
the selection and reloads the body, which keeps the body on a listed row.

The header appends `showing both|user|learned`, the footer names `[/] filter`, and
`SkillFilter::empty` gives the empty state its own words per mode.

Commits: `d80a55c` (test), `62df952` (feat), `d5955bc` (docs).

Verified by `the_skills_tab_filters_by_scope`,
`an_empty_filter_names_the_mode_it_is_empty_in`, and
`the_skills_footer_names_the_filter_keys` in `crates/hearth-cli/src/ui/mod.rs`.
The first asserts both rows and `showing both`, each filtered mode with its header
and its body, and that `[` wraps back to learned. `./scripts/ci.sh` passes.
