# Every IO failure is reported to the model as retryable

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-tools` |

## Observed

Every `std::io::Error` converted into a tool error is marked retryable, including
deterministic failures such as invalid UTF-8, permission denied, and a name that
is too long. The flag is shown to the model verbatim.

`crates/hearth-tools/src/tool.rs:74`:

```rust
impl From<std::io::Error> for ToolError {
    fn from(error: std::io::Error) -> Self {
        Self::temporary("io", error.to_string())
    }
}
```

`temporary` sets `retryable: true` (`tool.rs:46`), and `to_json` (`tool.rs:63`)
puts that field in the tool result. The agent returns the JSON to the model
(`crates/hearth-runtime/src/agent.rs:805`).

A `files read` of a PNG gives `{"code":"io","message":"stream did not contain
valid UTF-8","retryable":true}` because `Workspace::read_text` uses
`fs::read_to_string` (`crates/hearth-tools/src/workspace.rs:73`). The same answer
covers permission denied, `ENAMETOOLONG`, and a vanished file.

## Consequence

The model re-issues a call that cannot succeed. Nothing in the loop retries
automatically, so the cost is wasted turns against the available context, not a
duplicate side effect. For a binary file there is no alternative call that would
work either, so the model spends turns discovering that.

## Verification

Assert that `ToolError::from(std::io::Error::from(ErrorKind::InvalidData))` is
not retryable, and that `files read` of a binary file returns a non-retryable
error. Both fail today. `cargo test -p hearth-tools` passes without touching the
classification.

## Resolution

`ToolError::from(io::Error)` now checks the kind: interrupted, would-block,
timed-out, connection, broken-pipe, unexpected-eof, storage-full, and resource-busy
errors stay retryable; everything else is a caller error. The daemon test that
expected three attempts for a workspace that is not a directory encoded the old
classification and now expects one attempt.

Commits: `e3a9473` (test), `340d048` (fix), `481668e` (corrected daemon test).
Verified by `only_a_transient_io_error_is_retryable` and
`reading_a_binary_file_is_not_retryable`.
