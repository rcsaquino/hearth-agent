# The daemon still imports the previous memory layout on every start

| | |
| --- | --- |
| Kind | chore |
| Reported | 2026-11-17 |
| Area | `hearth-memory` |

## Observed

`crates/hearth-memory/src/legacy.rs` reads `memory/long-term.sqlite` and
`memory/standing.md`, writes the rows through `MemoryService`, and renames the
directory to `memory.imported`. `daemon()` calls `hearth_memory::import_legacy`
after it builds the service and before it starts Telegram. The crate re-exports
`LegacyImport` and `import_legacy`, and the module has its own tests for the old
row shape, bullets, and partial imports.

This code exists only to carry a home from an earlier build that kept memory
beside the database. The current spec says memory is one table in `hearth.db`.
The call stats two paths on every start, retries forever when a partial import
leaves the files, and logs a warning that an operator cannot act on. The
`memory/` layout is not documented as a current path.

## Wanted

Remove the import module, its re-export, the daemon call, and its tests. If any
supported home still has the old files, that is a one-time operator migration
outside the product, or an explicit refusal that names the files, not a silent
startup path. Keep `Source::Import` only if another current path uses it.

## Verification

`grep` finds the call only in `daemon()`, and the module doc itself says it reads
the store this release replaced. Delete the module and the call and the current
test suite does not lose a current behavior; only the legacy tests reference it.

## Resolution

Removed `crates/hearth-memory/src/legacy.rs`, its `LegacyImport` and
`import_legacy` re-exports, and the `daemon()` call. The current spec says memory
is one table in `hearth.db`; an operator with the old `memory/` files needs a
one-time migration outside the product. `Source::Import` stays because the
memory benchmark uses it to replay the fixture.

Commit: `50e6f51`. The change removes code and the build, memory tests, and daemon
tests remain green.
