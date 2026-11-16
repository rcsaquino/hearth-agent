# The Overview standing count reads a key the daemon never sends

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-16 |
| Area | hearth-cli |

## Observed

The `memory` row of the Overview "Work & memory" card shows:

```
memory   12 items · 0 standing
```

even when pinned rules exist.

## Mechanism

`status_value` (`crates/hearth-runtime/src/ipc.rs:193`) publishes the pinned rule
count as `rules_bullets`. `work_card`
(`crates/hearth-cli/src/ui/dashboard.rs:184`) reads `standing_bullets`, which the
daemon never sends, so `unwrap_or(0)` wins every time.

The regression test at `crates/hearth-cli/src/ui/mod.rs` built its fixture with
the same invented key, so it passed against a payload shape the daemon does not
produce.

## Resolution

`work_card` reads `rules_bullets`, and the test fixture now uses the key the
daemon publishes. The render assertion is unchanged, so it fails against the old
`standing_bullets` read.

Commits: `c892e75` (test), `181b8e8` (fix).

Verified by `cargo test -p hearth-cli`, and the fixture now matches the key
asserted in `crates/hearth-cli/tests/s8_telegram_daemon.rs`.
