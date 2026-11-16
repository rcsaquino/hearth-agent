# A recall by id prints a raw microsecond instant

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-memory` |

## Observed

A `memory_recall` call that names an id prints `valid_from` as an epoch
microsecond integer instead of the zoned ISO 8601 stamp used everywhere else.

`crates/hearth-memory/src/tools.rs:303`:

```rust
return Ok(text(format!(
    "{} [{}, {}, {}]: {}\n{}{}",
    memory.id,
    memory.kind,
    subject,
    if memory.pinned { "pinned" } else { "unpinned" },
    memory.content,
    format_args!(
        "source {} since {}",
        memory.source.as_str(),
        memory.valid_from
    ),
    history
)));
```

`valid_from` is an `i64` of UTC microseconds. The model reads
`source user since 1768438800000000`. The same tool struct carries `zone`, and
the transcript tool renders with `self.memory.zone.iso(...)`.

## Consequence

The model cannot turn a 16-digit instant into a date, so a question about when
something became true cannot be answered from the one field that answers it. It
also breaks the invariant that every timestamp shown to the model is ISO 8601 in
`identity.timezone`. `docs/DECISIONS.md` records the zone sweep for the
transcript citation and `as_of`; this stamp is not named there, so it is a gap
rather than an exception.

## Verification

Call `memory_recall` with an id in a fixed `Zone` and assert the reply contains
an ISO 8601 stamp with an offset and no bare microsecond integer. The current
reply contains the integer.

## Resolution

The by-id branch renders `since` with `self.memory.zone.iso(memory.valid_from)`,
so every stamp the tool prints is ISO 8601 with the offset in force.

Commit: `3dc9c29` (fix), after the test `test: stamp a recall by id in the
configured zone`. Verified by `a_recall_by_id_prints_a_zoned_instant` in
`crates/hearth-memory/src/tools.rs`, which asserts a stamp with an offset.
