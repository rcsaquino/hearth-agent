# The hidden-memory note counts the store, not the cut

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-17 |
| Area | `hearth-memory` |

## Observed

When the digest shows fewer memories than the store holds, `digest::coverage_lines`
adds one of two lines:

```
more: alice 3, or board 8 | memory_recall(subject) for the rest
earlier: alice 2 | memory_recall(as_of) for what was true then
```

The `more:` numbers come from `Coverage.current`, which `MemoryService::coverage`
fills from `store::subject_count` for **up to three subjects the message named**.
They are totals for the subject in the store minus what the digest showed, not
counts of what retrieval found and dropped.

That has three consequences:

- **No named subject, no note.** A keyword question that fills the digest and
  drops fifty matching rows says nothing, because `coverage` has no entity to
  count.
- **The number can overstate relevance.** `alice 3` says the store holds three
  more rows about alice. It does not say those rows matched the question; some
  may have been nowhere near the pool.
- **The note never says the cut is arbitrary.** With a pool of `rank::POOL` (128)
  and `memory_recall` taking `recall_max_items` (10), the cut is the normal case.
  The line reports the store's size, not whether the eleventh row was as good as
  the tenth.

`earlier:` has the same store-count semantics, but superseded rows are excluded
from candidates by `valid_clause`, so a store count is the only thing that can
produce it.

The benchmark's `reachable` measure in `crates/hearth-eval/tests/memory_bench.rs`
calls `service.search` itself, so it measures whether a search can find the
expected row, not whether a note would have prompted one. The known misses are
paraphrase (0.85 hybrid, 0.65 lexical), which a note cannot fix.

## Wanted

The `more:` line describes the cut, not the store.

- **Build it from dropped candidates.** After ranking and diversity, the fill has
  a list it did not show. Group the dropped ones that were close to the cut by
  subject, name up to three, and give the count of dropped candidates for each:
  `more: or board 12, alice 9 | memory_recall(subject) for the rest`.
- **Only say it when the cut hid something close.** The line appears only when
  the best dropped candidate is within a small margin of the weakest shown (start
  at 10 percent of the weakest shown score, tune from the measurement below). If
  every dropped candidate is clearly weaker, render nothing: the digest holds the
  best evidence, and a note about the rest is noise.
- **Fall back when there is no subject.** If the close dropped candidates have no
  subject, render `more: 1 match | memory_recall(query)`, which is actionable
  because the question text is the handle the model already has.
- **One note, not two.** At most one `more:` line. Do not add a second generic
  line, a total, or a "showing 10 of 60" count.
- **`earlier:` is unchanged.** It reports history for a named subject and keeps
  its current meaning and format.
- **The format is unchanged.** A plain line, no bullet, no id, the same `|` hint,
  so it stays consistent with the digest line format in
  `2026-11-17-each-memory-line-is-a-delimited-header.md`.

## Why

The note exists to tell the model that the answer may be incomplete. A number
about the store cannot do that: a store with fifty rows about a subject says
nothing about whether any of them match the question. A number about the cut can:
if the eleventh candidate ties the tenth, the item cap is the only reason the
model does not see it, and the note names the subject to ask about.

It also closes the silent case. A keyword question that drops strong matches
currently gets no note at all, and that is exactly when a second call would help.

The cost is bounded by construction: silence when the cut is clean, one line when
it is not, three subjects at most, and no ids.

## Measurement

The benchmark can label this without a model call.

For each question, compare `Recall.items` with `service.search(&question, at, 20)`
and classify the expected statement:

- **shown** — in the digest.
- **cut** — in the top 20 but not in the digest.
- **absent** — not in the top 20.

Then check the note against that label: it must fire on the `cut` class and name
the expected row's subject. It must stay quiet on the `shown` class. Print the
three counts and the note's fire rate, and keep them as the acceptance record.

If the note cannot be made to fire on `cut` without also firing on `shown`, the
margin is wrong or the idea has no value; close the issue rather than ship a note
that always fires.

## Scope

- Do not change selection, scoring, ranking, diversity, or the item cap.
- Do not add a bare total, a "showing X of Y", or an ellipsis on the last row.
- Do not change `earlier:` or the `as_of` path.
- Do not add a setting for the margin. Use a named constant with a comment until
  the benchmark shows a value that must be configurable.
- Do not put ids in the note; it names subjects and a tool call only.
- Do not count duplicate or kind-crowding drops as cut candidates if their shown
  twin already represents them. Start with candidates dropped by the item cap.

## Test

- A digest whose eleventh candidate is within the margin of the tenth renders
  `more: … | memory_recall(subject)`, naming the dropped candidates' subjects and
  their dropped counts.
- A digest whose eleventh candidate is below the margin renders no `more:` line,
  even when fifty candidates were dropped.
- Close dropped candidates with no subject render `more: 1 match |
  memory_recall(query)`.
- At most three subjects, alphabetical, and the counts are dropped candidates,
  not store rows.
- The note never changes the ten selected rows.
- `earlier:` output is identical before and after.
- The benchmark prints the `shown` / `cut` / `absent` counts and the note's fire
  rate for the fixture; the digest p95 stays under the 300-token gate.

## Documents

- SPEC 7.4 (the digest and the coverage line) and Appendix A, plus the Draft bump.
- DECISIONS: the move from store counts to dropped-candidate counts, the margin,
  the fallback text, and the benchmark label that decides whether the margin is
  right.
- The `digest::coverage_lines` doc comment, which currently describes the line as
  reporting what the store holds.

## Not established

- Whether the model acts on the note. Nothing in the repository measures that
  without a live provider call, so the issue claims an accurate note, not a
  better answer.
- Whether a 10 percent margin is right. The `cut` label decides it, and the issue
  is closable if no margin separates `cut` from `shown`.
- Whether `earlier:` should also become candidate-based. Superseded rows are not
  candidates by construction, so its store count is the only source.

## Closure

**Declined.** The proposed note was built and measured on the committed fixture,
and no margin separates the `cut` class from the `shown` class.

The first implementation built the note from the candidates the item cap dropped
after `rank::diversify`. On the 195 questions it fired on 0 of the 11 `cut`
questions and stayed quiet on all 182 `shown` questions. The cause is visible in
the digest: every `cut` question had 2 to 5 shown rows, not 10, so the item cap
was not the binding bound. The expected row had already been dropped by
`rank::diversify` for kind crowding or duplicate similarity, not by `rank::fill`.

The second implementation included the distinct rows `rank::diversify` dropped
for kind crowding, which is the broader set the "After ranking and diversity"
paragraph describes. It was measured at four margins:

| Margin | Cut note fired | Cut note named the subject | Shown quiet |
| --- | --- | --- | --- |
| 10% | 10 of 11 | 10 of 11 | 57 of 182 |
| 25% | 10 of 11 | 10 of 11 | 53 of 182 |
| 50% | 11 of 11 | 11 of 11 | 48 of 182 |
| 75% | 11 of 11 | 11 of 11 | 47 of 182 |

The 4,000-distractor run moves by one question in each class and does not change
the result. A note that fires on 125 of 182 `shown` questions is noise, and
raising the margin makes it worse, because the dropped rows sit on the same
subjects as the shown ones. The issue's own acceptance rule says to close rather
than ship a note that cannot separate the two classes.

The existing `more:` line keeps its store-count meaning. Its imprecision is the
known residual this issue described: it reports what the store holds about a
named subject, not what the cut hid. A future design needs a dropped-candidate
signal and a score that separates a close drop from a weak one; the measurement
above says the current scoring does not provide the second.

The `memory_bench` classification (`shown`, `cut`, `absent`) and the temporary
broader `rank::diversify` return were the measurement harness, not shipped code.
