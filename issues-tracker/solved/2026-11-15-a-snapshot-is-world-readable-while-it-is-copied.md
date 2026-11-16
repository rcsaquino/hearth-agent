# A snapshot is readable by other local users while it is being copied

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-storage` |

## Observed

The backup temp file is created by SQLite during `VACUUM INTO` and only chmodded
to 0600 after the copy finishes. For the whole copy, the file is a complete
snapshot of `hearth.db` at whatever mode SQLite creates it with, normally 0644
minus the umask.

`crates/hearth-storage/src/backup.rs:29`:

```rust
let temporary = destination.with_file_name(format!(".{file_name}.tmp"));
let _ = fs::remove_file(&temporary);

let connection = Connection::open(database)?;
connection.busy_timeout(Duration::from_secs(5))?;
let temporary_text = temporary.to_string_lossy().into_owned();
if let Err(error) = connection.execute("VACUUM INTO ?", [temporary_text.as_str()]) {
```

The permissions are applied only after the copy and after the connection is
dropped:

```rust
if let Err(error) = fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600)) {
```

Every other private write in the repository sets the mode when the file is
created (`crates/hearth-runtime/src/init.rs:194`,
`crates/hearth-runtime/src/home.rs:76`,
`crates/hearth-tools/src/media.rs:61`). The existing backup test checks the mode
of the destination after the rename (`crates/hearth-storage/tests/backup.rs:41`),
so the window is not covered.

## Consequence

The snapshot is the whole store: transcript, memory, tasks, usage, and any
metadata the database holds. On a multi-user host, another local account can
read it while it is being written. The window scales with database size.
A reviewer who reads only the destination mode will believe the file is private
for its whole life.

## Verification

With umask 022, snapshot a database large enough that the copy takes a moment,
and `stat` the `.<name>.tmp` file in the destination directory during the copy.
The mode is 0644. After the rename it is 0600.

## Resolution

`snapshot` vacuums into a 0700 staging folder next to the destination, chmods the
file to 0600, then renames it into place and removes the folder. The file is
unreachable to another local user for the whole copy, not only after the rename.

Commits: `1771545` (test), `544d316` (fix). Verified by
`a_snapshot_is_private_while_it_is_copied`, a watcher that polls the destination
folder during the copy and fails on a top-level file readable by group or other;
it saw the old 0644 `.tmp` file on the parent commit.
