# Automatic recall is counted from the first five digest rows

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-eval` |

## Observed

`memory_bench::run_fixture` builds a `shown` list with
`recall.items.iter().take(5)`. It uses that list for the `automatic` metric, the
precision calculation, and the supersession stale-row check. The digest can hold
`memory_recall.max_items` rows, which defaults to 10, so an expected row at rank
6 through 10 is in the digest but is scored as an automatic miss. The
supersession check can also miss a stale row shown at those ranks.

The same `shown` list is the only input to precision@5. That metric should stay
at five; the recall metric should use every row the digest rendered.

## Measurement

The committed fixture reports:

- `take(5)`: automatic 0.928, person 0.950
- every digest item: automatic 0.933, person 1.000
- precision@5 stays 0.928

The one-question difference is not noise. It is a person question whose row is
rendered at a position after the fifth, because `recall.items` has ten slots and
`rank::fill` keeps the top ten. The current 0.928 is therefore a lower bound on
the property SPEC 7.9 calls automatic recall, not the property itself.

## Verification

Change the automatic and supersession checks to use `recall.items` without
`take(5)`, keep precision@5 on the first five, and rerun
`cargo test -p hearth-eval --test memory_bench -- --nocapture`. The two numbers
separate as above.

## Related

The closed `the-hidden-memory-note-counts-the-store` issue measured the digest
cut with the same five-row assumption. Any future digest measurement should
state which rows count as shown.

## Resolution

`run_fixture` scores automatic recall and the supersession check against every
row `recall.items` rendered. Precision@5 still uses the first five rows. The
committed fixture now reports automatic 0.933 and precision@5 0.928, which
matches the measurement in the issue. `docs/SPEC.md` 7.9 and the AGENTS residual
record the corrected number.

Commits: `8b62c37` (test), `50d48b3` (fix). Verified by
`every_rendered_digest_row_counts_as_automatic`, whose expected row ranks sixth
in a six-row digest, and by `the_memory_benchmark_meets_its_targets`.
