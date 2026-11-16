# Two benchmark gates pass when their population is empty

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-15 |
| Area | `hearth-eval` |

## Observed

The contradiction and duplicate gates compare a count with itself, so both hold
when the fixture has no rows of that kind. Nothing asserts a lower bound on the
population.

`crates/hearth-eval/tests/memory_bench.rs:456`:

```rust
assert_eq!(report.duplicate_failures, 0, "a duplicate became a new row");
assert_eq!(
    report.supersession_correct, report.supersession_count,
    "a replaced statement answered a current question"
);
```

`supersession_count` is incremented only for questions whose category is
`"contradiction"` (`memory_bench.rs:306`), and `duplicate_failures` only for
memories with `duplicate_of` set (`:356`). The only population assertions are
`questions == 195`, `fixture_memories == 200`, and `spot_check == 20`
(`:447`). Category labels and `duplicate_of` values are not pinned.

## What should happen

The gates should fail if the fixture stops carrying the population they measure.
Relabeling the 15 contradiction questions as another category, or replacing the
five duplicate memories with ordinary ones, keeps every total the same and turns
two rows of SPEC 7.9 into measurements of nothing while the suite stays green.
The same shape appears at the noisy-store test, which asserts
`stored_rows >= 4_000` without asserting that any fixture statement was still
deduplicated.

## Verification

Relabel the fixture's `"category": "contradiction"` entries as `"direct"` and
run `cargo test -p hearth-eval --test memory_bench
the_memory_benchmark_meets_its_targets`. It passes with `supersession 0/0`. The
expected behavior is a failure naming the missing population.

## Resolution

The benchmark now pins the population each gate measures: 15 contradiction
questions and 5 duplicate statements. Relabelling the contradictions or dropping
the duplicate markers fails the gate instead of reporting 0/0 as success. The
noisy-store test asserts the duplicate population survives the distractors.

Commit: `c23da71`. Verified by relabelling the fixture's contradiction questions
to `direct` and re-running; the gate fails naming the missing population.
