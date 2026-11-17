# The quickstart and readme describe read-only surfaces as editable

| | |
| --- | --- |
| Kind | chore |
| Reported | 2026-11-17 |
| Area | `docs` |

## Observed

`README.md` says the TUI lets you "manage your own skills" and
`QUICKSTART.md` says "Skills can create or edit your own skills" and that the
Skills tab "browses learned skills and can create or edit your own". Draft 0.38
made the Skills tab read-only. `SkillsView` selects, filters, and scrolls; the
`handle_skills` key map has no create, edit, or history key. The solved
`the-operator-docs-describe-surfaces-that-are-gone` issue corrected an earlier
version of these lines, and a later Draft changed the code again.

`QUICKSTART.md` also says the Prompt tab has "`[`/`]` for older snapshots" and
uses `f` to resume following the tail. The Prompt tab has been one merged
conversation since Draft 0.29. `ui::prompt` binds Up, Down, PageUp, PageDown,
Home, End, Enter, and `f`; there is no snapshot list and no `[` or `]` handler.

## Wanted

Describe the Skills tab as read-only and remove the snapshot key instructions
from the quickstart. Point the reader at editing the files under `skills/` or
using Hearth's own skill loop, which is the current way to change the library.
If the version view returns, document it then.

## Verification

Open the Skills tab and press `n`, `e`, or `v`; nothing opens. Neither key is
bound in `handle_skills`. Open the Prompt tab and press `[` or `]`; the view does
not change and neither key is in `handle_prompt`.

## Resolution

`README.md` now says the TUI inspects learned skills instead of managing them.
`QUICKSTART.md` says the Skills tab is read-only and points at the files under
`skills/` and Hearth's own skill loop. The Prompt tab instructions drop the
removed `[`/`]` snapshot keys and keep `f` for following the tail.

Commit: `366b12e`. A docs-only change has no behavior test; `scripts/check-docs.sh`
passes.
