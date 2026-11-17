# Unused Cargo dependencies and features

| | |
| --- | --- |
| Kind | chore |
| Reported | 2026-11-17 |
| Area | `Cargo.toml` |

## Observed

- `crates/hearth-memory/Cargo.toml` depends on `tracing`, but the crate has no
  `tracing` use. The daemon logs the import and vector messages, not the memory
  crate.
- `crates/hearth-config/Cargo.toml` has a `hearth-core` dev-dependency with no
  use in the crate's tests.
- `crates/hearth-memory/Cargo.toml` enables the `uuid` `v5` feature. The crate
  uses `Uuid::new_v4` for memory ids; no v5 call exists.
- `crates/hearth-memory/Cargo.toml` enables the `rusqlite` `functions` feature.
  The crate has no scalar function registration; the bundled feature is enough.

## Wanted

Remove each unused dependency or feature and let `cargo build --workspace` and
the test suite prove the graph. A dependency that only a removed path used should
not remain in the manifest.

## Verification

`cargo tree -p hearth-memory` shows `tracing` and both optional features. A
search for `tracing::`, `Uuid::new_v5`, and scalar-function registration in
`crates/hearth-memory` is empty. Removing them leaves the compile and tests
green because no source references them.

## Resolution

Removed the direct `tracing` dependency, the `uuid` `v5` feature, and the
`rusqlite` `functions` feature from `hearth-memory`, and the unused
`hearth-core` dev-dependency from `hearth-config`. `cargo build --workspace`,
the `hearth-memory` tests, and the `hearth-config` tests are green.

Commit: `658b927`. The manifest change has no behavior test; the build and the
existing suites are the check.
