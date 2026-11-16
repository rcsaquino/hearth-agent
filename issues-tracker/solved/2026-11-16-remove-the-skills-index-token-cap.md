# The skills index token cap and its automatic archiving are removed

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-16 |
| Area | hearth-runtime |

## Wanted

Remove the token cap on the skills index. An active skill is indexed whatever its
description costs.

## Why

The cap silently destroys work. `SkillLibrary::index`
(`crates/hearth-runtime/src/skills.rs:234`) walks the active learned skills and,
for any skill that would push the rendered index past the cap, calls
`archive_for_index` (`skills.rs:513`), which sets the status to `Archived`,
rewrites the file, and appends a `CHANGELOG.md` line reading "archived by index
cap". The only visible trace afterwards is that entry.

The consequence is that the index is not a window onto the library but a mutator
of it. A skill stops being offered because its neighbour's description was long,
not because it failed or was unwanted. There is no undo short of editing the file
back to `active`.

The cap no longer protects anything either. Long descriptions are written
deliberately, and the value of an accurate one-line description is that the model
finds the skill without loading it. Truncating the list to stay under 300 tokens
trades that away.

## What exists today

- `INDEX_MAX_TOKENS` (`skills.rs:17`) is 300.
- `INDEX_MAX_ACTIVE` (`skills.rs:19`) is 50.
- `index()` applies both. User skills are appended first, and learners over the
  cap are archived and skipped rather than truncating the list silently.
- `SEGMENT_SKILLS` (`crates/hearth-core/src/budget.rs:11`) is 300, reported by
  `TokenCounts::over_budget` (`budget.rs:129`). That is a warning only — the agent
  logs `prompt segment over budget` at `agent.rs:549` and continues.
- `docs/SPEC.md:832,1453,2017` state the 300-token and 50-active caps.

## Scope

This asks for the **token** cap only.

`INDEX_MAX_ACTIVE` is a different limit — a count of entries, not a cost — and
this issue does not ask for it to change. The archiving it performs is the same
mechanism, so if it is meant to stay, a worker should confirm that archiving
remains reachable for the count limit while the token path no longer archives.

## Constraints

- The user-owned skills stay read-only. Only learned skills are archived today,
  and that asymmetry does not change.
- The prompt still reports the skills segment. Removing the cap does not remove
  the measurement, and the segment stays in the budget table.

## Test

Observable requirements:

- A library whose index exceeds 300 tokens indexes every active skill and
  archives none of them for that reason.
- The existing tests that assert the opposite must be replaced, not extended:
  `crates/hearth-runtime/tests/skills.rs:118`,
  `the_index_archives_low_success_learned_skills_over_the_cap`, and
  `crates/hearth-runtime/tests/s6_skills.rs:172`,
  `the_prompt_index_archives_the_overflow`. Both build 60 skills and require
  `archived > 0`.
- `crates/hearth-core/tests/s1_budget.rs:46` pins `SEGMENT_SKILLS == 300` and
  stays correct if only the index cap is lifted.

## Resolution

`INDEX_MAX_TOKENS` and the token branch in `SkillLibrary::index` are gone. Every
active skill is indexed up to `INDEX_MAX_ACTIVE` (50). Only the count cap retires
a learned skill, keeping the lowest success rate first and then the least
recently used; user skills stay read-only. `SEGMENT_SKILLS` stays 300 as a prompt
warning, and `TokenCounts::over_budget` still names `skills`.

The two tests that required archiving over the token cap are replaced. The new
tests build 15 and 40 skills with long descriptions, assert the rendered index is
over 300 tokens, assert every skill is listed, and assert none is archived. A
third test keeps 60 short skills and asserts the count cap still archives ten,
with the zero-success skills first.

Commits: `eb30e01` (test), `5166724` (change).

Verified by `cargo test -p hearth-runtime --test skills --test s6_skills` and
`./scripts/ci.sh`.
