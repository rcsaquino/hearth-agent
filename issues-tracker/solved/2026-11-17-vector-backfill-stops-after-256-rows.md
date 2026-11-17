# Vector backfill stops after 256 rows

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-memory` |

## Observed

`MemoryService::index_pending_vectors` fetches pending rows with
`store::pending_vectors(..., 256)` and embeds that one batch. It does not loop,
and the daemon calls it once at startup and once when a downloaded model becomes
active. There is no periodic pass and no signal that rows remain.

A home with more than 256 memories and a newly installed model therefore gets
vectors for the newest 256 rows. Every older row has no vector. Hybrid recall
silently treats those rows as absent from the semantic channel, so the store's
recall behavior depends on how many times the daemon has restarted. The
`memory.embeddings` setting promises the local hybrid path, and the daemon logs
one backfill count as though the work completed.

## Verification

Write 300 memories with embeddings disabled, attach a fake `EmbeddingProvider`,
and call `index_pending_vectors` once. The returned count is 256, and
`store::pending_vectors` still reports 44 rows for the same model and dimension.
The daemon logs the first count and exits the background thread.

## Related

`store::prune_vectors` runs in the same startup block and deletes vectors for
other models, so the remaining rows are not covered by a later model-specific
pass either.

## Resolution

`MemoryService::index_pending_vectors` now repeats the 256-row batch until no
pending row remains, and stops if an embed fails or a batch writes nothing.

Commits: `f645bbf` (test), `aa48cfa` (fix). Verified by
`the_backfill_indexes_every_pending_memory`, which seeds 300 facts, backfills
once, and requires 300 vectors and no pending row.
