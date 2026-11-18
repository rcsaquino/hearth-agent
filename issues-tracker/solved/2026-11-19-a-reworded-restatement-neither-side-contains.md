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

## Resolution

The store now asks the only actor that can tell a restatement from a changed
value, at the moment the pair costs a recall line.

- The digest renders both rows and follows them with one line,
  `similar: m_a/m_b | if they say one thing, memory(forget) one`. The two ids are
  addressable, the strongest-overlap pair is named first, and at most two pairs
  are named. The line changes nothing by itself. The model reads both statements
  and closes the one it judges redundant with `memory` and `action = forget`.
- The floor is 0.45 overlap about one non-empty subject, and it sits in a measured
  gap. The fixture's template pairs, which are different facts, stop at 0.429. The
  reported restatement is at 0.476, and a changed value is at 0.60 to 0.71. The
  line fires on 10 of the benchmark's 195 questions, and the benchmark now gates
  that at 20.
- The line sits where the duplicate already spends tokens. A pair that is never
  recalled never renders and never needs repair.
- The digest now also collapses a pair the write gate can prove is one statement,
  rather than only an exact fingerprint. Before, a reworded copy the gate refuses
  on write was still rendered twice until the nightly pass closed it.
- No model call was added to a memory operation. `docs/SPEC.md` 6.5 and 7.7 still
  hold, and the agent already reads the digest every turn.

What is still not automatic, and is now the design rather than a gap: the store
does not close the pair by itself. It cannot, because the words do not carry the
answer. The pair stays current until the agent reads the line, and a pair that is
never recalled changes nothing.

Commits: `8c16a60` (test), `5f3723e` (fix), and the docs in the same commit set.

Verified: `a_similar_pair_the_store_cannot_prove_names_both_rows`,
`a_near_duplicate_is_shown_once_and_counted`,
`two_different_facts_about_one_subject_get_no_notice`, and
`the_same_words_under_two_subjects_are_both_shown`. A real recall of the reported
pair rendered both rows plus
`similar: m_f7nk2tvvt4/m_zz1t9sk320 | if they say one thing, memory(forget) one`.
`./scripts/ci.sh` and `./scripts/acceptance.sh` pass, digest tokens stay at 97 p95,
and the memory benchmark's notices are 10 of 195.
