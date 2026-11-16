# The Skills tab offers edit, new, and history for a read-only library

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-16 |
| Area | `hearth-cli` |

## Current

The Skills tab is meant to expose the library read-only (SPEC 10.2), but it still
offers three writing keys, all handled by `App::handle_skills` in
`crates/hearth-cli/src/app.rs`.

| Key | Effect |
| --- | --- |
| `e` | Opens the editor on a user skill; refuses a learned one with a message. |
| `n` | Starts the new-skill name input, finished by `App::create_user_skill`. |
| `v` | Enters version history; `v` or `esc` leaves it, through `SkillsView::show_history` and `SkillsView::leave_history`. |

All three are advertised: `footer` in `crates/hearth-cli/src/ui/mod.rs` lists
`v history`, `e edit user skill`, and `n new skill`, `help` says
`Skills     e edits your own skill files`, and the empty state in `render` says
`Press n to write one`.

## Desired

The Skills tab offers no way to change the library: no `e`, no `n`, no `v`. It
selects, reads, and scrolls. A skill is read where it lives, and the library is
changed by editing the files or by Hearth's own skill loop.

The keys and the hints that name them go with them, and the empty state stops
pointing at a key that no longer exists.

## Constraint

`EditorKind::Skill` and the `skills/<name>.md` write path exist only behind these
keys. Editing `SOUL.md` and `HEARTH.md` from Settings is a different route and
stays.

## Related

`2026-11-16-the-skills-tab-cannot-filter-by-scope.md` — the empty-state line in
`render` is the same line, so both changes touch it.

## Verification

Open the Skills tab and press `e`, `n`, and `v`. Nothing in the library changes,
no editor opens, and the footer lists only selection, scroll, and quit.

## Resolution

The Skills tab selects, reads, and scrolls. `e`, `n`, and `v` are gone, with the
in-tab new-skill prompt, the version-history view, `EditorKind::Skill`, and the
`skills/<name>.md` write path behind them. `SkillsView.new_name`, `versions`, and
`version`, `create_user_skill`, `show_history`, `leave_history`, and `slug` went
with them. The footer lists selection, scroll, and quit; the help no longer
advertises a skill editor; the empty state points at the folder and Hearth's own
skill loop.

Editing `SOUL.md` and `HEARTH.md` from Settings is a different route and stays.

Commits: `886a740` (test), `2e0655a` (feat), `d54907f` (docs).

Verified by `the_skills_tab_offers_no_writing_keys`,
`the_skills_footer_names_only_reading_keys`,
`the_empty_skills_library_points_at_no_key`, and
`the_help_does_not_advertise_a_skill_editor` in `crates/hearth-cli/src/ui/mod.rs`.
The tests for the removed behavior, `skills_tab_creates_a_new_user_skill` and
`the_skills_tab_reads_a_preserved_version`, were deleted with it.
`./scripts/ci.sh` passes.
