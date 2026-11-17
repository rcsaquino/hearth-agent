# A memory write can carry more than one statement

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-17 |
| Area | `hearth-memory` |

## Observed

`memory_save` accepts content with several sentences. `store::validate` checks
empty content, length, NUL, subject length, pin rules, and secrets. It does not
check the number of statements. `extract::sentences` splits sentences for the
deterministic extractor, but the tool path does not.

For example, these two writes succeed as two rows:

```text
User likes apples. He likes to bring it to work.
User likes apples. He buys one every day.
```

Each row repeats `User likes apples`. The store compares whole rows. The exact
check sees different content, and a duplicate predicate that compares whole
rows cannot remove the shared first sentence. The duplicate is inside the row,
not between rows. A merge cannot remove it without dropping the distinct second
sentence.

## Wanted

One durable statement per memory. A write with more than one clear sentence is
refused or reported before it enters the store, with a message that tells the
caller to split it. The extractor already produces one sentence per candidate.

A `related` result must say that a neighbour is not a duplicate. Use
`supersedes` only when the new statement replaces the row, not when it adds a
separate fact. In the example, the correct state is three rows:

```text
the user likes apples
the user brings apples to work
the user buys an apple every day
```

The second and third rows are different behaviors. Both can be true. Neither
should supersede the other.

Existing compound rows are not split automatically. Pronouns such as `He` and
`it` need context, and the store must not rewrite content. They are reported by
the audit instead.

## Why

Shared prefixes become repeated text in every row. The digest spends tokens on
the same claim twice, and a later merge cannot remove the repetition without
losing the distinct second sentence. Preventing new compounds is cheaper and
safer than repairing them after they enter the store.

## Scope

- Do not add a pronoun gate. The specification allows pronouns.
- Do not auto-split existing rows.
- Do not rewrite content.
- Keep the 200-character limit and the other validation.
- Keep the extractor's sentence splitting.

## Test

- Two clear sentences in one write are refused or reported.
- A single statement with an abbreviation such as `Dr. Smith likes apples` is
  not split by mistake.
- Atomic writes for the shared preference and each behavior succeed and remain
  three rows.
- Existing compound rows are reported by the audit and are not changed.
- A `related` result for a new, non-replacing behavior does not recommend
  `supersedes`.

## Not established

- Hard rejection or a soft warning that asks the caller to retry.
- How to detect two independent clauses joined by `and` without a parser.
- Whether imports need an escape hatch for historical compound records.


## Resolution

The kind stays `change`. A write is refused, not warned: hard rejection is the
simplest behaviour that guarantees the store never gains a compound row.

`hearth-memory/src/text.rs` holds one statement splitter. It does not split a
known title or initial (`Dr.`, `e.g.`, `1.5 p.m.`), a decimal, an ellipsis, a
repeated punctuation mark, or a lowercase continuation. The extractor uses the
same splitter, so it can now read a stated statement that begins after an
abbreviation.

`store::validate` counts the statements and returns an `invalid_input` message
that names the count and asks the caller to split the write. The 200-character
limit, NUL check, subject limit, pin rules, and secret check are unchanged.
Existing compound rows are not split or rewritten; the consolidation audit
reports them.

`memory_save` no longer appends `Pass supersedes=...` to a related row. It says
the neighbour is a separate statement, not a duplicate.

Commits: `2b25eef` (test), `d344f2c` (change).

Verified by `two_statements_in_one_write_are_refused`,
`an_abbreviation_does_not_make_a_second_statement`, and
`atomic_statements_for_the_shared_preference_remain_three_rows` in
`crates/hearth-memory/src/store.rs`, by the splitter cases in
`crates/hearth-memory/src/text.rs`, and by
`a_statement_beside_a_related_one_names_the_neighbour` in
`crates/hearth-memory/src/tools.rs`. `compound_rows_are_reported_and_not_split`
in `crates/hearth-memory/tests/consolidate.rs` covers the audit report.
