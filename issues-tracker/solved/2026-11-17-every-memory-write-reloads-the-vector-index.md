# Every memory write reloads the whole vector table on the next recall

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-memory` |

## Observed

`MemoryService::write` and `write_many` call `invalidate_vectors` after any row
that was created. `forget`, `index_pending_vectors`, and `prune_vectors` do the
same. The next `semantic_channel` call sees an empty `VectorCache` and reloads
`store::vectors(model, dims)`, which selects every vector for that model, joins
`memories`, and materialises the bytes. `retrieval::top_vectors` then filters to
current rows in Rust, so retired vectors are loaded and discarded as well.

Extraction runs after every answered turn and writes memories. A conversation
that writes one memory per turn therefore invalidates the cache after each
answer and reloads the full table before the next recall. At the 200,000-row
scale the vector payload is about 77 MB at 384 INT8 dimensions, plus the join
and allocation, paid on the turn path. The cache only helps between writes.

`invalidate_subjects` is called from the same paths, so the bounded subject list
is also reloaded after every write. That list is capped at `MAX_KNOWN_SUBJECTS`,
but it is still a full scan of distinct subjects.

## Verification

Attach a fake embedder, write one memory, recall, then write another memory and
recall again. The second recall reloads `store::vectors`. The committed
benchmark seeds the store once and does no writes between questions, so it does
not expose this pattern. A benchmark that writes one memory per question would.

## Related

The AGENTS residual says the hybrid vector scan at 200,000 rows is unmeasured.
This reload makes the measurement a per-turn allocation cost, not only a scan
cost.

## Resolution

`MemoryService::write` and `write_many` no longer empty the vector and subject
caches. A created row adds its vector to a loaded cache for the same model and
dims, marks an explicitly replaced row not current, and adds a new subject in the
same longest-first order. `forget` removes the row from the loaded vector cache.
`store::by_ids` keeps only current rows, so a stale current flag cannot leak a
replaced statement. Backfill and prune still refresh the cache because they run
at startup.

Commits: `78fafe9` (test), `39d6ba2` (fix). Verified by
`a_write_extends_the_vector_cache`, which loads two vectors, writes a third, and
requires the loaded cache to hold three rows without another recall.
