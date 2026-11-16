# An unused skill says (unused) instead of a dash

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-16 |
| Area | `hearth-cli` |

## Observed

`rate` in `crates/hearth-cli/src/ui/skills.rs` returns `—` when
`hearth_runtime::skills::Skill::uses` is zero, and a percentage otherwise.
`render` in the same file places that string in a four-character right-aligned
field, so an unused row shows a padded dash where its rate would be.

Both a user skill and a never-run learned skill take that branch. A user skill
never leaves zero because `record_use` and `record_outcome` resolve only learned
skills through `find_learned`; a learned skill leaves zero on its first counted
use.

The dash is documented as the intended display in the Draft 0.37 note at the top
of `docs/SPEC.md`, in the 9.6 skills rules, and in the DECISIONS entry "An
uncounted skill has no rate". `an_unused_skill_shows_no_success_rate` in
`crates/hearth-cli/src/ui/mod.rs` pins the exact dashed row text.

## Wanted

The Skills tab writes `(unused)` in the rate column when `Skill.uses` is zero,
for both scopes. A counted skill keeps its percentage. The rate field widens so
`(unused)` fits, and percentage rows stay right-aligned in the same column, so
the list still lines up.

`Skill::success_rate` and the sort and archive logic that read it do not change.
No skill becomes counted or uncounted; this is only the word the tab uses for a
skill with no counted use.

## Why

An em dash is a blank whose meaning lives only in the documentation. In the tab
it can read as missing data, a divider, or a glyph that failed to render, and the
Skills tab has no legend that explains it. `(unused)` states the fact in place
and keeps the distinction the dash was chosen for: a skill with no counted use
must not read as a skill that failed every run. `0%` stays wrong for the same
reason it was wrong before.

## Scope

- The dash goes from every unused row, including a user skill, which can never
  gain a rate.
- A learned skill shows its percentage after its first counted use, as it does
  today. A user skill stays `(unused)`.
- The `0%` exclusion stays. Only the placeholder text and the column width
  change.

## Documents

- Update the Draft 0.37 note at the top of `docs/SPEC.md`, bump the Draft, and
  update the 9.6 Skills tab rule.
- Update the DECISIONS entry "An uncounted skill has no rate" to record
  `(unused)` and the wider column.
- Leave `2026-11-16-an-unused-skill-shows-a-zero-percent-success-rate.md` in
  `solved/` as the record of the dash choice. This issue replaces that display
  decision, not the reason for it.

## Test

`an_unused_skill_shows_no_success_rate` in
`crates/hearth-cli/src/ui/mod.rs` fails after the change unless its dash
assertions are updated. Rename it to match the new wording and make it assert:

- A user skill and a never-run learned skill both show `(unused)`.
- Neither row shows `0%`.
- After a successful use, the learned skill shows `100%` while the user skill
  still shows `(unused)`.

Keep the check on the rendered rows rather than the exact padding, so the wider
column is free to change, but confirm the percentage and `(unused)` land in the
same column.

## Resolution

`rate` in `crates/hearth-cli/src/ui/skills.rs` returns `(unused)` when
`Skill.uses` is zero and a percentage otherwise. The rate field is eight
characters wide, and the name and scope are padded to the widest row, so a
percentage and `(unused)` end at the same edge. `Skill::success_rate`, the sort,
and the archive logic are unchanged.

Commits: `18f826d` (test), `56a1c5f` (feat), `b203827` (docs).

Verified by `an_unused_skill_shows_unused` in `crates/hearth-cli/src/ui/mod.rs`,
which renders a user skill and a never-run learned skill as `(unused)`, rejects
`0%`, then records a successful use and asserts the learned row reads `100%`
while the user row stays `(unused)` and both rates share a column edge.
`./scripts/ci.sh` passes.
