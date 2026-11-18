| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-19 |
| Area | `memory` |

## What happens now

The same fact can still be stored twice, in two current rows, when the two
wordings reword each other instead of one containing the other.

The nut allergy pair from the earlier report is the example:

- "Kya has a nut allergy including pistachio; verify ingredients rather than
  assuming a food is safe"
- "Kya, Sir's partner, has a nut allergy including pistachio, so ingredients
  must be verified rather than assumed safe"

Neither wording contains the other, so the store writes both and leaves both
current. The nightly pass does not report the pair either, because it reports only
pairs it can prove. The save-time hint named the pair only if a new statement was
written next to it.

Merging this pair automatically was measured and rejected, not skipped for time.
The two above overlap by 0.48. The value changes the store must keep apart, such
as "the primary database is SQLite" against "the primary database is Postgres",
overlap by 0.60 to 0.71. Any word-overlap threshold low enough to merge the first
pair also merges the second, and merging a changed value retires a statement.

## What should happen

One live statement per fact, including a reworded restatement. The store needs a
judgement it does not have: are these words one fact, or two?

## What is not established

Where the judgement should come from. Three shapes are open:

- Let the agent close the pair, which the product already allows: the save-time
  hint names a related row and says "if the two say the same thing, forget one",
  and the `memory` tool has `forget` and `supersedes`. The gap is that nobody
  looks until a write happens near the pair.
- Give the repair path one model call. The product currently says no memory
  operation calls a model, so this needs an explicit decision.
- Surface the pair on a grid the operator can see and act on, without pretending
  the store knows which row is right.

Until one is chosen, the reported pair stays current twice. This is the residual
of `solved/2026-11-18-the-same-fact-is-kept-twice-in-different-words.md`.
