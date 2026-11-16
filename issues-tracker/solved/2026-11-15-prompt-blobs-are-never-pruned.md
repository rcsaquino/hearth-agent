# Prompt blobs are never pruned, and the schema would block pruning

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-core`, `hearth-storage` |

## Observed

Every model request saves its full prompt text to `prompt_blobs`. Nothing ever
deletes a row, although the specification says the table is pruned after 30 days.

`crates/hearth-core/src/session.rs:736`, `save_prompt_blob`:

```rust
"INSERT OR IGNORE INTO prompt_blobs (hash, text, created_at) VALUES (?, ?, ?)"
```

`crates/hearth-runtime/src/agent.rs:549` passes the serialized prompt document,
which contains the identity, tools, rules, skills index, guidance, the recall
digest, the full rendered transcript, and the runtime note. The hash is a v5 of
that text, and the text changes every turn because the transcript and the clock
change, so `INSERT OR IGNORE` does not deduplicate consecutive turns.

SPEC 6.6 says "Prompt blobs are pruned after 30 days by default"
(`docs/SPEC.md:933`). A search for `DELETE FROM prompt_blobs` finds nothing, and
neither maintenance task touches the table (`crates/hearth-runtime/src/maintenance.rs`
registers only `backup` and `skill.review`).

The schema also blocks a naive prune. `crates/hearth-storage/migrations/0001.sql:45`:

```sql
prompt_hash TEXT,
FOREIGN KEY (prompt_hash) REFERENCES prompt_blobs(hash)
```

There is no `ON DELETE SET NULL`. With `foreign_keys=ON`
(`crates/hearth-storage/src/lib.rs:80`), deleting a blob that an entry
references fails with a constraint error. A prune must first clear the
referencing `entries.prompt_hash` values or the foreign key must change.

## Consequence

Each turn stores the whole prompt, and the prompt contains the whole transcript
up to that turn, so storage grows with the square of conversation length. A
conversation that reaches the context budget stores megabytes of duplicated
transcript across its turns. The database therefore grows much faster than the
entries themselves, which works against SPEC 14's "Annual database size, 2 GB or
less" target and inflates every nightly snapshot. The Prompt tab only reads the
newest 50 blobs, so the older ones serve no reader.

## Verification

Run a conversation of 20 turns against a fake provider, then query
`SELECT COUNT(*), SUM(LENGTH(text)) FROM prompt_blobs`. There is one row per
request, the row count equals the request count, and the total text length is
roughly the sum of the growing transcripts. Run the daemon across a 30-day
boundary or call the maintenance tasks directly; the row count does not drop.

## Resolution

`Session::prompt_blobs_before` is removed by `prune_prompt_blobs`, which clears the
`entries.prompt_hash` links first (the foreign key has no cascade) and then
deletes the old blobs. The nightly backup handler runs it before the snapshot, so
the stored prompt history is bounded at 30 days.

Commit: `59d4b1a`. Verified by
`an_old_prompt_blob_is_pruned_and_its_entry_survives` in
`crates/hearth-core/tests/s1_usage.rs` and by the backup-handler test, which fails
when the prune call is disabled.
