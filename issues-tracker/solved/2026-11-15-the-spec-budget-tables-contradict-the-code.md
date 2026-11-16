# The spec's budget tables contradict the current design

| | |
| --- | --- |
| Kind | chore |
| Reported | 2026-11-15 |
| Area | `docs` |

## Requested

One authoritative number for each budget, and no table that describes a design
that was removed. The specification currently disagrees with itself and with the
code in several places, so a reader cannot tell which value is in force.

## The contradictions

- Turn limit. SPEC 5.1 says "There is no turn limit. A tool loop runs until the
  model stops asking for tools or the user cancels." Appendix A still lists
  `Turn limit | 120 | 5.1`, and §13 says the Overview shows a turn limit. The
  code has no turn limit (`crates/hearth-runtime/src/agent.rs` has no counter or
  cap), and the Overview has no such metric. The removal is deliberate (commit
  `9681d62`, which rebuilt memory and refined the terminal surface, removed the
  limit), so the table and the Overview sentence are stale.

- Recall budget. SPEC 6.1's layer table says L6 is 900 tokens. SPEC 7.5 says
  "Default budgets are 400 tokens for rules and 300 for the digest, with at most
  12 statements." Appendix A says "900 tokens, 6 items". The code uses 300 and
  12 (`crates/hearth-config/src/settings.rs:424`,
  `crates/hearth-core/src/budget.rs:14`), and `docs/DECISIONS.md:1688` records
  the 300 default. 6.1 and Appendix A are stale.

- Standing memory and decay. Appendix A lists `Standing memory | 1,200 tokens`,
  `Memory decay | 5% per 90 days, floor 15`, and `Archive threshold | 3 years
  unused and importance below 20`. SPEC 7.7 says "There is no decay and no
  automatic archival", the `memories` table has no importance column
  (`crates/hearth-storage/migrations/0008.sql`), and nothing in
  `crates/hearth-memory` implements decay.

- Scheduled memory pass. Appendix A lists both "No scheduled memory pass |
  03:30 in `identity.timezone`" and a backup/skill-review schedule. The line is
  a leftover from the old nightly consolidation. `maintenance.rs` registers only
  `backup` at 03:00 and `skill.review` at 04:30.

## Why it is worth doing

The spec is the binding product definition. An operator who reads Appendix A
believes there is a runaway guard at 120 requests and a 900-token recall budget.
A maintainer tuning the prompt from 6.1 will over-count. `AGENTS.md` requires a
deviation to be recorded and the draft to be bumped when behavior changes.

## Verification

Read the cited SPEC lines against the cited code and migration. Each pair
disagrees on the printed line, with no intervening decision that reconciles
them.

## Resolution

SPEC 6.1's L6 row now carries 300 tokens and Appendix A one recall row of
"300 tokens, 12 items", matching `budget.rs` and the settings defaults. The
stale rows are gone, not restated: turn limit, standing memory, memory decay,
archive threshold, and the scheduled memory pass. The Overview sentence no
longer claims a turn limit.

Commit: `3960ab1`. Verified against `crates/hearth-core/src/budget.rs`,
`crates/hearth-config/src/settings.rs`, `crates/hearth-runtime/src/maintenance.rs`,
and `crates/hearth-storage/migrations/0008.sql`.
