# Retrieval has two targets and no measurement at the larger one

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-15 |
| Area | `hearth-eval`, `docs` |

## Observed

SPEC 14 states "Memory retrieval, 200k items | 80 ms or less". SPEC 7.9 states
"Retrieval latency, p95 | 100 ms". The executable gate uses the looser number
and the largest store it measures is 4,200 rows.

`crates/hearth-eval/tests/memory_bench.rs:28`:

```rust
/// Retrieval may take this long at the 95th percentile on a personal store.
const TARGET_MS: f64 = 100.0;
```

The fixture is 200 statements, and the noisy-store test adds 4,000 distractors
(`memory_bench.rs:521`). `AGENTS.md` records that a 200k store was not
re-measured after the memory rebuild, and that the vector scan is linear from
about 10 ms at 4k.

## What should happen

One target, one measured store size, and a stated scaling assumption for the
sizes that are not run. Today a 25 percent regression against the section 14
target passes the gate, and any regression that only appears past about 4k rows
passes too. A reader quoting "80 ms at 200k items, measured not aspirational"
(SPEC 14) is quoting a number the suite never produces.

## Verification

`rg 'TARGET_MS|200_000|4_000' crates/hearth-eval` shows the only store sizes are
200 and 4,200 and the only latency constant is 100. Run the benchmark and read
the reported p95 and store size; neither the 200k population nor the 80 ms
threshold appears.

## Resolution

There is one target and one measured store: 100 ms at p95 on the 200-statement
fixture plus 4,000 distractors, quoted by SPEC 7.9, SPEC 14, and the benchmark
constant, with the linear scaling note beside it. The 200k store remains
unmeasured, so that remainder is a new file:
`open/2026-11-15-no-200k-retrieval-measurement.md`.

Commit: `f0c6fcd`. Verified by reading the three numbers and by
`cargo test -p hearth-eval --test memory_bench`.
