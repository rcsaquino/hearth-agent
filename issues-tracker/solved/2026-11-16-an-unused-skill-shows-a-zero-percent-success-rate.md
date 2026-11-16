# An unused skill shows a 0% success rate

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-16 |
| Area | `hearth-cli` |

## Observed

The Skills tab prints a percentage for every row. `render` in
`crates/hearth-cli/src/ui/skills.rs` formats each label with the skill name, its
scope, and `skill.success_rate() * 100.0` to zero decimals.

Any skill that has never been used shows `0%`. That covers every user skill, and
every learned skill before its first run.

## Mechanism

`Skill::success_rate` in `crates/hearth-runtime/src/skills.rs` returns `0.0`
when the skill has never been used, and the ratio otherwise.

For a user skill the counter can never leave zero, because both counters resolve
the skill through `find_learned`:

- `record_use`, which increments `uses`
- `record_outcome`, which increments `successes` or `failures`

`find_learned` never returns a user skill, so `uses` and `successes` stay at zero
for the life of the file.

A learned skill is different only in that it can eventually leave zero. Until its
first run it renders the same `0%`.

## Consequence

`0%` is a statistic, not a blank. It reads as a measured failure rate on a skill
that has never been tried, and it sits beside a learned skill whose percentage is
real. A person cannot tell "never used" from "used and always failed".

The misleading case is strongest for a newly learned skill, since that is a row
the person did not write and cannot judge from memory: the first thing the tab
says about it is that its success rate is zero.

The scope column already distinguishes the two kinds, so the number adds nothing
for a user skill and misinforms for both.

## Desired

A skill with no outcomes shows no rate: an em dash, `—`, in place of the
percentage. A rate appears only where a use has been counted.

This applies to both scopes, and the row keeps its alignment either way.

The distinguishing value is already at the render site. `SkillsView.skills` is a
`Vec<hearth_runtime::skills::Skill>`, which carries `uses`, `successes`, and
`failures`, so the render can tell an unused skill from a failed one.

No behaviour depends on the zero: user skills are never archived, and the
archive order in `SkillLibrary::index` only sorts learned skills that the index
already holds.

## Verification

Add a user skill and let Hearth learn one that has not run. Both rows show `—`.
Run the learned skill successfully and its row shows `100%`. A user skill
continues to show `—`.

## Resolution

`rate` in `crates/hearth-cli/src/ui/skills.rs` returns an em dash when
`Skill.uses` is zero and a right-aligned percentage otherwise, so the row keeps its
column alignment. `Skill::success_rate` is unchanged: the index cap still sorts
learned skills by it, and a computed zero is the right value there.

Commits: `1164a40` (test), `96080b3` (fix), `b4eaec0` (docs).

Verified by `an_unused_skill_shows_no_success_rate` in `crates/hearth-cli/src/ui/mod.rs`,
which renders a user skill and a never-run learned skill as dashes, then records a
successful use and asserts the learned row reads `100%` while the user row stays a
dash. `./scripts/ci.sh` passes.
