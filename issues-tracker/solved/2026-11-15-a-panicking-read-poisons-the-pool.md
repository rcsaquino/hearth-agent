# Four panics in a read closure hang every later read

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-storage` |

## Observed

`Storage::read` removes a connection from the pool, runs the closure, and only
returns the connection on the normal path. A panic in the closure unwinds past
the return, so the pool loses a connection permanently. After four panics the
next read blocks on the condition variable forever.

`crates/hearth-storage/src/storage.rs:81`:

```rust
let mut readers = self.readers.lock().map_err(|_| StorageError::Poisoned)?;
while readers.is_empty() {
    readers = self
        .reader_ready
        .wait(readers)
        .map_err(|_| StorageError::Poisoned)?;
}
let connection = readers.pop().expect("the pool is non-empty");
drop(readers);
let outcome = run(&Reader {
    connection: &connection,
});
if let Ok(mut readers) = self.readers.lock() {
    readers.push(connection);
    self.reader_ready.notify_one();
}
outcome
```

There is no guard object and no replacement path. `Storage::open` creates exactly
four read connections (`storage.rs:31`). The writer side is not affected: a
panic there still surfaces as `StorageError::Closed` through the channel.

## Consequence

A panic inside any read mapping closure, and there are many across
`hearth-core` and `hearth-memory`, shrinks the pool for the life of the process.
The daemon stays alive and holds the lock, but after four panics every read
blocks with no error and no timeout, so it stops answering Telegram. A single
panic also abandons the worker thread, so the symptom is silent.

## Verification

In `crates/hearth-storage/tests/s2_storage.rs`, wrap four
`storage.read(|| panic!())` calls in `catch_unwind`, then call a fifth read from a
watchdog thread. It never returns. The expected behavior is either a fresh
connection or a `Poisoned` error, not a permanent block.

## Resolution

`Storage::read` now checks out the connection into a `PooledReader` whose `Drop`
pushes it back and notifies the condvar. A panic in the closure unwinds through
that drop, so the pool keeps all four connections.

Commits: `db7feb9` (test), `6d7c405` (fix). Verified by
`a_panicking_read_returns_its_connection`, which panics four reads and then
requires a fifth read to finish within five seconds.
