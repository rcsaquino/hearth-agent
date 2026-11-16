# Skill history has no surface, though the spec requires one

| | |
| --- | --- |
| Kind | feature |
| Reported | 2026-11-15 |
| Area | `hearth-cli`, `hearth-runtime` |

## Requested

The TUI Skills tab should expose the preserved versions of a learned skill, so a
revision can be read or compared after the fact.

SPEC 9.6 requires "The TUI Skills tab exposes the library read-only: bodies,
history, and counts", and says "Every version is kept under
`skills/learned/.history/`". The tab currently shows a list, one body, and
`stats`, which covers bodies and counts but not history.

## Current state

`crates/hearth-runtime/src/skills.rs:424` defines `history(name)` and
`version_body(name, version)`. Neither has a non-test caller:

- `history` is called only from `crates/hearth-runtime/tests/skills.rs` and the
  skill-review tests.
- `version_body` has no caller at all.

`crates/hearth-cli/src/app.rs:823`, `handle_skills`, matches `Up`, `Down`,
`Enter`, `PageUp`, `PageDown`, `Home`, `End`, `n`, and `e`. None lists or loads
a version. An earlier CLI exposed `hearth skills list|show|history|diff|stats`
(`docs/DECISIONS.md:874`); the current binary has no subcommands
(`crates/hearth-cli/src/main.rs:24`), and the TUI that replaced it has no version
view.

## Why it is worth doing

Every revision writes a copy under `skills/learned/.history/<name>/vN.md`
(`skills.rs:363`), so the data already exists. Without a reader, a revision is
unrecoverable in practice: the user cannot see what a learned skill did last
week, and the specification's history requirement is unmet.

## Verification

`grep -rn "version_body\|\.history(" crates --include=*.rs | grep -v tests`
returns nothing. A browser through the Skills tab has no key that lists versions,
and the body pane shows only the current file.

## Resolution

The Skills tab binds `v` on a learned skill to its preserved versions. The header
shows the version list, Up/Down reads a version through
`SkillLibrary::version_body`, and Esc returns to the current body. User skills
have no history.

Commit: `4de7dc9`. Verified by `the_skills_tab_reads_a_preserved_version`, which
saves two revisions, opens history, reads `Revision 0` with Up, and returns to
the current body on Esc.
