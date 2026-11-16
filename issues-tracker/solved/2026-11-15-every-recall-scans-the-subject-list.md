# Every recall scans and clones the whole subject list

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-15 |
| Area | `hearth-memory` |

## Observed

Each automatic recall reads every distinct subject in the store, builds a
`HashSet` of owned strings from it, and compares each multi-word subject against
the message words. The work is linear in the number of stored subjects and runs
on the turn path.

`crates/hearth-memory/src/store.rs:446`:

```rust
"SELECT DISTINCT subject FROM memories WHERE subject <> '' \
 ORDER BY LENGTH(subject) DESC",
```

There is no `LIMIT`. `crates/hearth-memory/src/lib.rs:577`,
`known_subjects`:

```rust
fn known_subjects(&self) -> Result<HashSet<String>> {
    Ok(self.subject_list()?.into_iter().collect())
}
```

`crates/hearth-memory/src/query.rs`, `analyze`, then splits every known subject
into a `Vec` of parts and compares each part against every word of the message.
The cache is invalidated by `write` and `forget` (`lib.rs:404`, `:428`), so the
first recall after any memory write repeats the full scan.

## Why it is worth changing

Recall runs on every message, and the store is meant to hold years of history.
The documented measurements stop at 200 statements and 4,000 distractors; the
vector scan is already linear from about 10 ms at 4k (`AGENTS.md`, known
residuals), and this scan grows on the same axis without a bound. A 200k store
pays it on the path the 100 ms p95 target covers.

## Verification

Time `MemoryService::recall` on seeded stores of 1k, 10k, and 100k statements
with a fixed query, and report the curve and the number of subjects. The subject
query's cost should be visible as a rising share. A bound (a `LIMIT` plus a
longest-first prefix check, or matching against the raw message instead of the
full subject list) is the smallest shape that keeps the behavior.

## Resolution

Fixed in `aa07a74`, with the failing benchmark in `2a0708b`.

**Measured first.** `crates/hearth-eval/tests/memory_scale.rs` seeds a store with
one distinct subject per statement, then times (a) the subject query alone, (b) a
cold recall on a fresh service, whose subject cache is empty, and (c) a warm
recall. On this machine, before the fix:

| Statements | Subjects | Subject query p95 | Cold recall p95 | Warm recall p95 |
| --- | --- | --- | --- | --- |
| 1,000 | 1,000 | 1.30 ms | 21.91 ms | 19.39 ms |
| 10,000 | 10,000 | 5.27 ms | 87.56 ms | 82.13 ms |
| 100,000 | 100,000 | 124.52 ms | 957.13 ms | 781.52 ms |

The cold/warm gap at 100,000 was about 176 ms, and the query alone accounted for
most of it. The rest of the recall grew on the same axis, so the subject scan was
not the only cost, but it was the one this issue names.

**What changed.** The subject list is capped at `MAX_KNOWN_SUBJECTS` (4,096
distinct names), longest first, with `subject` as the tie-breaker so the kept set
is stable. The list supplements the lexical channel, so a statement about a
dropped short name is still found by its words.

A cap alone was not enough. `ORDER BY LENGTH(subject) DESC` still sorted every
distinct subject on the first recall after a write. Migration `0010` adds the
partial covering index `memories(LENGTH(subject) DESC, subject) WHERE subject <> ''`,
so the query walks the index in its read order and stops at the bound.

**After.** Same benchmark, same machine:

| Statements | Subjects | Subject query p95 | Cold recall p95 | Warm recall p95 |
| --- | --- | --- | --- | --- |
| 1,000 | 1,000 | 1.30 ms | 21.91 ms | 19.39 ms |
| 10,000 | 10,000 | 5.27 ms | 87.56 ms | 82.13 ms |
| 100,000 | 100,000 | 5.12 ms | 623.46 ms | 617.46 ms |

The subject query is now flat from 10,000 to 100,000. The cold/warm gap at
100,000 is about 6 ms, so the scan is no longer a rising share. The remaining
617 ms is the FTS channel and the ranker; that is recorded in
`open/2026-11-15-no-200k-retrieval-measurement.md`, not here.

**Verification.** `cargo test -p hearth-eval --test memory_scale -- --ignored
--nocapture` fails on `2a0708b` with
`the subject query took 124.52 ms at p95 on 100000 statements`, and passes on
`aa07a74`. `the_subject_list_stops_at_its_limit_longest_first` covers the cap and
the order. `cargo test --workspace` and
`cargo clippy --workspace --all-targets -- -D warnings` pass. Migration `0010`
moves `SCHEMA_VERSION` to 10 and is documented in `docs/SCHEMA.md` and
`docs/DECISIONS.md`.
