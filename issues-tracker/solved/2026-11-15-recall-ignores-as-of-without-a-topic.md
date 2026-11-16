# `memory_recall` ignores `as_of` when the call names no topic

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-memory` |

## Observed

A recall call with only `as_of` returns current statements. The instant is parsed
and then discarded on that branch.

`crates/hearth-memory/src/tools.rs:332`, `RecallTool::call`:

```rust
let found = if let Some(subject) = optional(arguments, "subject") {
    self.memory.service.subject(&subject, at, count).map_err(failure)?
} else if let Some(query) = optional(arguments, "query") {
    self.memory.service.search(&query, at, count).map_err(failure)?
} else {
    self.memory.service.recent(count).map_err(failure)?
};
```

`crates/hearth-memory/src/lib.rs:438`:

```rust
pub fn recent(&self, limit: i64) -> Result<Vec<Memory>> {
    store::recent(&self.inner.session, None, limit)
}
```

The store already accepts the instant, and the `search` branch passes it to the
same function when the query is trivial (`lib.rs:447`). The recall-by-id branch
also ignores it. The tool description tells the model that `as_of` "Reads what
was true then", and SPEC 7.8 lists an as-of read as one of the four uses.

## Consequence

A historical question answered with today's value, presented as fact. Each
returned line carries only a relative age, so the reply does not show that the
date was ignored. No test covers `as_of` without a subject or query.

## Verification

Write a statement with an old `valid_from`, then a replacement that is current
now. Call `memory_recall` with `{"as_of": "<the old date>"}` and no query or
subject. The replacement is returned. `cargo test -p hearth-memory` passes
today because no test makes that call.

## Related

`open/2026-11-15-recall-by-id-prints-a-raw-microsecond-instant.md` is a separate
gap in the same tool's time handling.

## Resolution

`MemoryService::recent` now takes the instant, and the no-topic branch passes
`at`. The eval caller passes `None`, which is the old behaviour for browsing.

Commits: `e3a1330` (test), `9bad0ac` (fix). Verified by
`a_bare_as_of_read_returns_what_was_true_then`, which writes an old statement,
replaces it, and asserts a bare `as_of` read returns the old one. The test
failed on the parent commit by returning today's statement.
