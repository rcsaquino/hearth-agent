# A panic in the storage writer closes every later write

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-storage` |

## Observed

`Storage::open` starts `writer_loop` on a dedicated thread. The loop calls each
queued `Job` with no `catch_unwind` and no replacement thread. The job wraps
`run_transaction`, so a panic in any closure that builds or applies a write
unwinds the writer thread.

When that thread dies, the receiver is dropped. The next `Storage::write` still
holds a sender, but `Sender::send` fails and the call returns
`StorageError::Closed`. Every later write returns the same error. The daemon
process stays alive, keeps the home lock, and still serves reads and status, but
no conversation, task, memory, or effect can be saved.

`Storage::read` already handles the equivalent fault for the read pool. A
`PooledReader` guard returns its connection on unwind, so four panics do not
shrink the pool. The writer has no guard and no recovery.

## Consequence

A programming fault in one write closure becomes a permanent, silent write
outage. A restart is the only recovery, and the daemon has no path that decides
to restart itself. The failure is visible only as `storage writer is closed` in
later errors.

## Verification

Run a write closure that panics, then run a second ordinary write from a new
thread. The second returns `Closed` and never reaches SQLite while the process
remains alive. The `a-panicking-read-poisons-the-pool` issue documents the read
side of this contract, and its resolution does not cover the writer.

## Resolution

`Storage::write` runs the write closure under `catch_unwind`. A panic rolls the
transaction back as it unwinds, the caller receives `StorageError::Panicked`,
and the writer thread stays alive for the next job.

Commits: `068ae86` (test), `d130017` (fix). Verified by
`a_panicking_write_does_not_close_the_writer`, which panics one write and then
requires a following write to commit its row.
