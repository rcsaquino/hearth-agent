# The log file is never rotated

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-runtime` |

## Observed

`init_logging` opens `logs/hearth.log` in append mode and writes through it for
the life of the daemon. Nothing rotates, truncates, or prunes it.

`crates/hearth-runtime/src/lib.rs:45`:

```rust
let logs = home.join("logs");
std::fs::create_dir_all(&logs)?;
let path = logs.join("hearth.log");
let mut options = std::fs::OpenOptions::new();
options.create(true).append(true);
```

The subscriber writes one JSON line per event to that file. No maintenance task
touches it: `crates/hearth-runtime/src/maintenance.rs` registers only `backup`
and `skill.review`, and `daemon.rs` prunes helpers, not logs.

SPEC 3.3 says of `logs/`: "Safe to delete? Yes. They are rotated automatically."
`QUICKSTART.md:150` also points at the file as a diagnostic.

## Consequence

A long-running daemon grows one file without bound. Every run, turn, tool call,
and Telegram poll event adds a line, and the file never shrinks. On a small disk
the file can fill the filesystem, at which point SQLite writes fail and the
record stops accepting saves. The documented claim that deletion is safe is
misleading: deleting the file under a running daemon leaves the open descriptor
writing to an unlinked inode.

## Verification

Run the daemon in a temporary home with `logging.level = "debug"` and a loop of
requests, and watch `logs/hearth.log` grow past several megabytes. Wait across
the nightly maintenance window or inspect `logs/` after a restart. No rotated or
numbered file appears, and the original keeps its size plus the new writes.

## Resolution

The log writer owns the file, its path, its current size, and a limit. A write
that would cross the limit renames `hearth.log` to `hearth.log.1` and reopens the
file, keeping one previous copy. The limit is 10 MiB.

Commit: `a2a9c5e`. Verified by `a_full_log_rotates_into_one_previous_copy`, which
fails when rotation is disabled.
