| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-18 |
| Area | `memory` |

## What happens now

The same fact gets stored twice when it is written twice in different wording,
and both copies stay. They show up as separate lines whenever memory is recalled,
so one fact takes two slots out of the fixed number of lines I get to see.

Two examples from my own memory right now:

- Kya's nut allergy is stored twice. One copy says "Kya has a nut allergy
  including pistachio; verify ingredients rather than assuming a food is safe."
  The other says "Kya, Sir's partner, has a nut allergy including pistachio, so
  ingredients must be verified rather than assumed safe." Same fact, same
  subject, both current.
- Moving to this harness is stored twice, and the two copies differ only in
  phrasing.

An exact copy of a statement is already handled. Writing the identical words is
ignored, and the recalled memory hides identical duplicates and says how many it
hid. What escapes is the same fact said differently, which is the common case,
because any rewrite changes the words.

There is also a maintenance pass that notices these pairs and reports how many
it found, but it does not do anything about them. It counts them and leaves them.

## What should happen

One live statement per fact. When a new statement says what an existing one says
in different words, one of them should stop being current, the way an exact copy
already does.

The count of near duplicates that the maintenance pass produces should lead to
repair rather than only to a number in a report.

## Why it matters

The memory I see each turn is capped at a fixed number of lines. Duplicates spend
that budget on repetition instead of on things I need to know. A handful of
duplicates is untidy. Enough of them and the recalled memory is mostly restating
two or three facts while dropping the rest.

## What is not established

Whether the maintenance pass deliberately stops at reporting, and whether closing
a near duplicate automatically would risk retiring a statement that only looks
similar. That trade is the implementer's call, but the report should not be the
end of it.

## Resolution

Partly fixed, and the remainder is split into
`2026-11-19-a-reworded-restatement-neither-side-contains.md`.

What changed:

- The store closes a near duplicate when its own write gate would already have
  refused the second write: the same statement under one subject, or one statement
  whose words are all in the other. The nightly pass now does that repair instead
  of only counting the pair, keeps the fuller wording, and reports `near_closed`.
  Pairs it cannot prove are reported and left current.
- A write that contains an earlier statement replaces it, so the fuller wording is
  the row that stays current. The shorter restatement is refused with the fuller
  row returned. Before this, a fuller write was refused as a duplicate, which kept
  the thinner wording and dropped the detail the issue complains about.
- The save result no longer says "This is a separate statement, not a duplicate."
  It names the pair and the repair: "If the two say the same thing, forget one."
- The keyword extractor that produced both example pairs is gone
  (`solved/2026-11-18-the-keyword-memory-extractor-only-writes-duplicates.md`), so
  new reworded copies are no longer manufactured.

Why the reported pair itself is still stored twice: the store cannot tell it from
two different facts. The pair scores 0.476 by token overlap. The value changes it
must keep apart score 0.60 to 0.71. There is no threshold between them, so an
automatic close would either miss the restatement or retire a changed value. That
residual is the new open issue above.

Commits: `226578c` replace on containment, `c99aeff` close near duplicates in the
pass, `1f080c3` neutral related hint, `659e702` boundary test, `d5b9143` extractor
removal, `66759ce` docs.

Verified: `./scripts/ci.sh`, `./scripts/acceptance.sh`, and the memory benchmark
(170 stored rows before and after, 5 duplicates refused, 15 supersessions kept,
automatic recall unchanged at 0.933). Focused tests:
`a_near_duplicate_the_write_gate_refuses_is_closed_by_the_pass`,
`a_near_pair_keeps_the_fuller_wording`,
`a_statement_that_contains_an_earlier_one_replaces_it`,
`a_statement_an_earlier_one_contains_keeps_the_earlier_row`,
`a_changed_value_about_one_subject_is_not_a_duplicate`,
`a_reworded_restatement_the_store_cannot_prove_is_left_current`, and
`a_statement_beside_a_related_one_names_the_neighbour`.
