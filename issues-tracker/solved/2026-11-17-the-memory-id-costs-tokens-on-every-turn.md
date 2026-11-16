# The memory id costs tokens on every turn

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-17 |
| Area | `hearth-memory` |

## Observed

`store::new_id` in `crates/hearth-memory/src/store.rs` makes an id as `m_`
followed by the 32 hex characters of a v4 UUID, so every id is 34 characters.
It is rendered in three places:

- `digest::line` starts every recall line with it, so the automatic digest
  carries one per row, before the kind, subject, age, and statement. Ten rows
  cost about 96 tokens per turn in the part of the prompt that is never cached.
- `digest::rule_line` renders `{id}: {content}` for every rule. Fifteen rules
  cost about 148 tokens of the 800-token rules budget, which is roughly one and a
  half rules displaced by ids.
- `memory_recall` prints the same `digest::line` format, so a 20-row answer
  carries about 192 tokens of ids. It also returns the full statement for one id,
  which is the only way to read a row whose content is clipped by
  `digest::LINE_CHARS` in the digest.

The id no longer earns its place in the digest. Since the content limit became
150 and then 200 characters, `digest::line` shows the whole statement, so
`memory_recall(id)` is not needed to read it. What remains is provenance: the
source, the valid-from instant, the revision, and the successor.

`id` is also charged by `digest::line_cost`, which adds one quarter of the id
length to each row's estimated cost before `rank::fill` decides what fits. That
term disappears with the open issue that bounds recall by count instead of
tokens, so the two changes touch the same function and whichever lands first
rebases the other.

`Uuid::new_v4` is also the one v4 identifier in a repository whose SPEC says IDs
are time-ordered UUIDv7. A truncated v4 cannot sort, so no code may ever use the
id for ordering.

A home with 106 current memories, 36 of them rules, measures the rules cost: the
block renders 22 of 36 rules today and names 14 as omitted. Without the id it
renders 31 and names 5. The id accounts for nine rules in that store.

## Wanted

The id stops being part of the text the model reads except where it is needed to
fetch something.

**The digest and the rules block:**

- A recall line carries no id when the statement is shown whole, which is every
  row written after the 150-character limit.
- A line carries the id when `clip` shortened the statement, because
  `memory_recall(id)` is the only way to read the rest. The id goes at the end of
  the line in that case. The line shape itself, a delimited header, belongs to the
  open issue `2026-11-17-each-memory-line-is-a-delimited-header.md`; this change
  removes the id and leaves the header to that file.
- The rules block carries no id. A rule is read, not addressed; a rule that must
  be replaced or deleted is found with `memory_recall(kind = rule)`.
- `ID_SAMPLE` and `digest::rules_cost` stop using a full-size id to measure the
  rules render, since the render no longer has one.

**The tool output:**

- `memory_recall` keeps one id per row, because that is where addressing belongs.
- New ids are `m_` plus ten characters of Crockford base32: 50 bits, no `I`,
  `L`, `O`, or `U`, so a retyped id is less likely to be wrong and a copied id is
  twelve characters instead of thirty-four. `memory_recall` output drops from
  about 192 tokens to about 70 for 20 rows.
- Tool input is normalised before lookup: trim, accept either case, and accept the
  full id only. No prefix matching, because an ambiguous prefix that deletes the
  wrong row is worse than a longer id.
- Generation retries on a `PRIMARY KEY` conflict. At 100,000 rows, 50 bits gives
  about a 4-in-a-million chance that any pair collides, and the retry makes the
  chance irrelevant.
- Existing 34-character ids keep working. No migration rewrites an id, because
  `superseded_by`, `memory_vectors.memory_id`, and saved transcripts refer to
  them.

## Why

The digest is auto-injected on every turn, so fixed cost per row is paid on every
message. About 90 tokens a turn buys nothing: the statement is already complete in
the line.

The rules block is capped, so an id costs more than its tokens — it takes room
from a rule. Dropping ids there buys about one and a half rules by default.

The tool output is where an id is actually used, and a 12-character id is not
worse at that job. Crockford base32 is the alphabet for the job: it excludes the
four characters that a model or a person confuses when copying, and it decodes
case-insensitively, which the existing hex ids do not.

## Scope

- Do not change what an id means: stable for the row, opaque, never reused, and
  the target of `supersedes` and `superseded_by`.
- Do not use a rowid, an ordinal, or a per-turn alias. A short integer that
  changes meaning after a compaction is a deletion waiting to hit the wrong row.
- Do not put the id in the coverage line (`memory_recall(subject) for the rest`
  names a subject, not an id) and do not remove the coverage line.
- Do not change the digest's kind, subject, age, or statement text. The only
  render change is where the id appears, and only for a clipped row.
- Do not change the content limit, the subject limit, or the retrieval channels.
- Do not add prefix matching to `memory_recall` or `memory_forget`.

## Test

- A digest line for a statement within the content limit contains no `m_` token;
  a line for a statement above the limit ends with its id, and
  `memory_recall(id)` returns the full text.
- A rules block contains no id, and a rule can still be replaced with
  `memory_save(supersedes = ...)` after `memory_recall(kind = rule)` returns it.
- `memory_recall` output for 20 rows carries 12-character ids, and the rendered
  digest token estimate for ten rows drops by the id's share. Measure the text,
  do not assert a hard number.
- A generated id is `m_` plus ten lowercase Crockford characters and is
  accepted by `memory_recall` in either case and with surrounding whitespace.
- A forced id collision retries and saves; two rows never share an id.
- A row written with a 34-character id is still found by `memory_recall(id)` and
  can still be superseded.
- The tests that count digest rows by `starts_with("m_")` in
  `crates/hearth-runtime/tests/s6_agent.rs` and `crates/hearth-memory/src/lib.rs`
  count rows by the leading kind token from
  `2026-11-17-each-memory-line-is-a-delimited-header.md`, so they still assert the
  number of rows and not the number of ids.

## Documents

- SPEC 7.2 (the id row), 7.4 (the digest format), 7.6 (the rules render),
  Appendix A, and the Draft bump.
- DECISIONS: a new entry for the id shape, the Crockford alphabet, the no-prefix
  rule, and where the id is rendered.
- The `memory_recall` schema text, the `ID_SAMPLE` constant, and the
  `line_cost`/`rules_cost` doc comments that mention the id.

## Not established

- Whether any caller needs the id in the digest for provenance. The source and
  valid-from instant are in the `memory_recall(id)` output, and nothing else
  reads them from the digest.
- Whether ten Crockford characters are the right length. Eight would save four
  more characters per id at a 40-bit bound; ten keeps the collision chance far
  below the value of a shorter line.
- Whether the id should become v7 like the rest of the system. Truncation makes
  the version irrelevant for ordering, and nothing reads the id's order.

## Resolution

`store::new_id` returns `m_` plus ten Crockford base32 characters (50 bits) from a
v4 UUID. `store::normalize_id` trims and lowercases before `get`, `forget`, and a
`supersedes` target; the lookup is exact, never a prefix. `store::reserve_id`
checks the generated id inside the write transaction and retries a taken id up to
eight times. Existing 34-character ids are not rewritten, and `get`, `forget`, and
`supersedes` still work on them. The digest carries an id only on a clipped row,
the rules block carries none, and `memory_recall` ends every row with its
12-character id. The `memory_recall` schema names the shape and says case does not
matter. The row-count tests use the leading kind token instead of `starts_with("m_")`.

Commits: `69def54` (test), `14b51f3` (feat), `8894484` (docs).

Verified with `a_new_id_is_short_and_crockford`, `a_taken_id_is_retried`,
`an_id_is_read_case_insensitively_and_trimmed`, `a_prefix_is_not_an_id`,
`a_legacy_id_is_still_found_and_replaced`, the tool tests for recall and forget by
id in either case with surrounding whitespace, the digest clip test, and the rules
test. `./scripts/ci.sh` and the memory benchmark pass.
