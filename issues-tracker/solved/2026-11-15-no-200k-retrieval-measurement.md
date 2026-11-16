# Nothing measures retrieval on a 200k store

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-15 |
| Area | `hearth-eval`, `docs` |

## Observed

The retrieval target now names the store the suite actually builds: 4,200 rows
and 100 ms at p95 (SPEC 7.9 and 14, `crates/hearth-eval/tests/memory_bench.rs`
`TARGET_MS`). The larger store is unmeasured. The vector scan is linear from
about 10 ms at 4,000 rows (`AGENTS.md`, known residuals), so a 200,000-row store
is an extrapolation, not a result.

This is the remainder split from
`solved/2026-11-15-retrieval-has-two-targets-and-no-200k-measurement.md`.

## What should happen

Measure a 200,000-row store once and record the p95 and the machine, or state in
SPEC 14 that the number is extrapolated and by what factor. An ignored benchmark
that builds the store is acceptable as long as its result is written down where a
reader can find it.

## Verification

`rg '200_000|200k' crates/hearth-eval` finds no store at that size. The reported
p95 in `cargo test -p hearth-eval --test memory_bench` comes from a 4,200-row
store.

## Resolution

Measured in `70d1d1d` and recorded in `4cdce2e` and `4e67c99`.

**What was built.** `crates/hearth-eval/tests/memory_bench.rs` gains
`the_200k_store_is_measured`, ignored by default. It seeds 200,000 noise rows
with distinct subjects and the committed fixture, then times the 195 fixture
recalls on the lexical path. Run it with:

```text
cargo test -p hearth-eval --test memory_bench the_200k -- --ignored --nocapture
```

**Result.** On the reference host (Intel Core i3-7100T, two cores and four
threads, 7.6 GB RAM, Linux 7.3.4 x86_64), 200,170 stored rows:

| Measurement | Value |
| --- | --- |
| Retrieval p50 | 9.1 ms |
| Retrieval p95 | 221.3 ms |
| Retrieval p99 | 937.3 ms |

The p95 is the lexical channel when a fixture question shares a word with the
noise. It is not the subject list: the subject query is 5.2 ms at p95 on the same
store with 200,000 distinct subjects (`memory_scale`). The 4,200-row gate is
unchanged and still measures about 15 ms.

**What is still extrapolated.** The hybrid vector scan at 200,000 rows was not
measured, because backfilling 200,000 vectors is not a routine check. `SPEC.md`
14 now states the factor: linear extrapolation from about 10 ms at 4,000 rows is
about 500 ms at 200,000, so the 100 ms gate is not claimed for the hybrid path at
that size. `AGENTS.md` known residuals and `docs/DECISIONS.md` say the same.

**Verification.** `rg '200_000|200k' crates/hearth-eval` now finds the store at
that size. The benchmark reports the numbers above. `cargo test --workspace`
passes, and the ignored test is the artifact a later reader re-runs.
