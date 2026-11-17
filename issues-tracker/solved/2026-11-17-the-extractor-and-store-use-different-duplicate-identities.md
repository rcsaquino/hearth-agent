# The extractor and the store use different duplicate identities

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-memory` |

## Observed

One durable fact can become two current rows after it passes through the deterministic extractor. An automatic extract can also replace a better row with a weaker one.

The following outcomes follow from `extract::candidates` and `store::insert`:

- `I prefer dark mode.` becomes `Preference`, subject `user`, through
  `extract::from_preference`.
- `Remember that I prefer dark mode.` becomes `Fact`, subject empty, with the
  same content through `extract::from_remember` and `extract::leading_subject`.
- `I prefer dark mode please.` is a near match of the first row. The second row
  has an empty subject, so `store::insert` skips the same-subject merge and
  writes a new current row.

The first two writes do not create two rows. The exact check misses because the
kind differs, but the later fingerprint-only lookup in `store::insert` finds the
earlier content and closes it. That path ignores kind, so the subjectless `Fact`
replaces the subjectful `Preference`. When the content also changes by one word,
the fingerprint-only lookup misses and the empty subject skips the near check.
Two current rows remain.

Other shapes that follow from the same code:

- `My favorite color is blue.` reaches `extract::from_possessive` before
  `extract::from_preference`, so it becomes a `Fact` with subject
  `favorite color`. A remembered form or a model write can classify the same
  preference as `Preference` with subject `user`.
- `My sister is Alice.` and `Alice is my sister.` compose the same relation in
  opposite word orders. Their fingerprints differ, so exact dedup misses them.
  Only the near check can join them.
- `the user prefers dark mode` and `the user likes dark mode` do not merge.
  `store::content_tokens` has no stemming or synonym folding, so `prefers` and
  `likes` stay distinct even when the subject is the same.

## Mechanism

- `extract::from_remember` rewrites first person to `the user ...`, then asks
  `extract::leading_subject` for the subject. `leading_subject` returns empty
  when the first word is `the`, so every remembered first-person statement is
  subjectless.
- `extract::from_preference` names the subject `user`.
- `extract::candidates` runs `from_possessive` before `from_preference`, so
  `my favorite ...` is classified as a fact before the preference classifier
  sees it.
- `extract::from_relation` and `extract::from_possessive` produce the same
  relation in different orders.
- `store::insert` checks near duplicates only when `item.subject` is non-empty
  and only against current rows with the same `subject`, within `MERGE_WINDOW`.
- `store::same_statement` compares raw `content_tokens`. It does not fold
  contractions, verb forms, spelling variants, or synonyms.
- `store::insert`'s fingerprint-only `earlier` lookup closes a current row
  regardless of kind. Its only guard is for a pinned row. A subjectless `Fact`
  can therefore close a subjectful `Preference` with the same words.
- `store::related` returns `None` for an empty subject, so the model gets no
  neighbour hint on a subjectless write.

## Expected

- One surface fact produces one current row, whichever trigger caught it.
- An automatic extract cannot replace a subjectful, non-`Fact`, or pinned row
  with a weaker one.
- A near duplicate is refused or reported consistently, not only when both rows
  carry the same non-empty subject.
- A negated statement is not a duplicate of its affirmative form.

## Scope

- No model call in the write path.
- Preserve history: close rows, do not delete.
- Do not rewrite existing content.
- Do not weaken the secret, length, or pin checks.

## Test

- Every surface form of one preference yields the same candidate and leaves one
  current row.
- Both relation orders yield one current row.
- A subjectless remembered statement does not close a subjectful preference.
- A synonym or spelling variant with the same subject is refused.
- `the user prefers dark mode` and `the user does not prefer dark mode` remain
  two current rows.
- A duplicate older than `MERGE_WINDOW` is still found, or the limitation is
  stated.

## Not established

- Whether a material addition such as `in the editor` is a duplicate or a
  related refinement. The current containment rule refuses it and drops the
  detail; reporting it may be safer.
- Whether the store should compare content when both subjects are non-empty and
  different. The current specification says no.


## Resolution

The extractor and the store now share one duplicate decision.

- `extract::from_remember` names the subject `user` when the rewrite starts with
  `the user`. `from_preference` runs before `from_possessive`, so `my favourite
  ...` is a preference. Both relation orders compose `Alice is the user's
  sister`.
- `store::content_tokens` folds common US and UK spellings and the preference
  verbs `prefer`, `like`, and `love`. The fold is comparison-only; stored content
  and the fingerprint keep the caller's words. `same_statement` refuses to call a
  negation a duplicate of its affirmative.
- The near check reads every row about a non-empty subject, however old, plus the
  newest 64 subjectless rows. A subjectless write is compared against the newest
  64 current rows. The old `MERGE_WINDOW` limit no longer hides an older
  same-subject duplicate.
- A write with `Source::Extract` returns the earlier row instead of closing a
  pinned row, a subjectful row when the extract has no subject, or a non-`fact`
  row with a `fact`.
- `store::related` reads the recent current rows when the subject is empty, and
  `memory_save` reports a neighbour as a separate statement without recommending
  `supersedes`.

Commits: `c52f281` (test), `dad3b14` (fix).

Verified by the new cases in `crates/hearth-memory/src/store.rs` and
`src/extract.rs`: `both_relation_orders_produce_the_same_candidate`,
`a_remembered_preference_names_the_same_subject_as_a_stated_one`,
`a_favourite_is_a_preference_not_a_fact`,
`a_synonym_about_the_same_subject_is_refused`,
`a_spelling_variant_about_the_same_subject_is_refused`,
`a_negation_is_not_a_duplicate_of_its_affirmative`,
`an_older_duplicate_is_found_beyond_the_subjectless_window`,
`an_automatic_extract_cannot_weaken_a_stored_row`, and
`a_subjectless_write_still_reports_a_related_neighbour`, plus the tool test
`a_statement_beside_a_related_one_names_the_neighbour`. The memory benchmark
still passes: automatic 0.933, recoverable 0.990, duplicate failures 0, secret
leaks 0. A subjectless write still reads only the newest 64 rows, which is the
one remaining window.
