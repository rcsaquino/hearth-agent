| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-18 |
| Area | `skills` |

## What happens now

I add skills of my own to the skills folder and they work. I load them, they run,
they do real work. The Skills tab in the terminal never changes: the row keeps
saying `(unused)` forever.

I noticed it today. I loaded one of my own skills, which is the whole point of
having it, and the Skills tab still listed it as unused. That was not a display
glitch on a stale screen; reopening the tab shows the same thing. The row cannot
move, because the count behind it is never recorded for a skill I wrote.

The status makes sense for a skill the assistant learned and has not run yet.
For a skill that has clearly been used it is simply wrong, and it makes the tab
useless for the skills I care most about, since those are the ones I write
myself.

## What should happen

Loading a skill counts as using it, whoever wrote the skill. After I load a skill
of mine, its row should show a use, not `(unused)`.

## Notes

I know an earlier report about this ended with the placeholder being kept for
user-written skills on purpose: a skill I wrote can never gain a success rate, so
it kept showing `(unused)`. That reasoning holds for the *rate* but not for the
*count*. A skill can be used without anybody measuring whether it succeeded. The
tab conflates the two, and the result is that the row claims something untrue
about a skill that has demonstrably been run.

So the narrow fix is to keep no rate for a user skill while still recording that
it was used. Showing something like "used, no rate" is honest. `(unused)` is not.

## What is not established

Whether the count is worth persisting for skills I wrote, given they are read-only
files that I own and could edit by hand. If recording a count would mean writing
back into a file I manage, a session-level count that resets is a reasonable
answer, as long as the tab stops saying unused after a use.

## Resolution

A load of a skill counts as a use for either scope, and a user skill that has
been loaded no longer reads `(unused)`.

- `record_use` now resolves through `find` instead of `find_learned`, so it counts
  a skill the user wrote. A learned skill still writes its counters into its own
  frontmatter, because the review pass archives on them.
- A user skill stays read-only. Its count goes to `skills/.uses.json`, a hidden
  ledger keyed by skill name, written through a temporary file and renamed at mode
  0600. `load_all` overlays it on user skills only, so `uses`, `last_used`, and the
  top-skills count mean the same thing in the daemon and in the tab. A missing or
  unreadable ledger counts nothing and warns; the library never fails over a lost
  count.
- The Skills tab's rate column keeps `(unused)` at zero uses, keeps the
  percentage for a learned skill, and shows `used` for a user skill with a count.
  The word sits in the same right-aligned eight-character field, so the column
  still lines up. No rate is invented for a file Hearth cannot write an outcome
  into, which keeps the earlier decision about the rate and only changes the
  count.

Commits: `e705087` (test), `b77c9e0` (feat), `f0dfd3c` (docs).

Verified: `a_user_skill_counts_its_own_loads_without_being_rewritten` drives the
`skill` tool twice and asserts the count is 2 and the user's file is byte-for-byte
unchanged. `a_loaded_user_skill_stops_saying_unused` renders the tab and asserts
the row reads `used` with no percentage, in the same column as a learned skill's
`100%`. A real home showed
`{"writing":{"uses":2,"last_used":"2026-11-18T16:50:49.791916974Z"}}` after two
loads. `./scripts/ci.sh` passes.
