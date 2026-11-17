# `memory_recall` filters by kind after the limit

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-memory` |

## Observed

`RecallTool::call` chooses one of `MemoryService::subject`, `search`, or
`recent`, passing the caller's `count` as the SQL limit. It then filters the
returned `Memory` values by the requested `kind`. The filter runs after the
limit, not inside it.

A `memory_recall` with `kind: "rule"` and no subject or query asks for the newest
20 memories and then drops every non-rule. If the store has 20 newer facts, the
result is `no memories matched` even though a live rule exists. The same shape
applies to `subject` and `query`: a kind filter can hide matches that were
pushed past the limit by other kinds. The tool schema offers `kind` as a normal
filter, so the caller has no reason to expect a different result set.

`SaveTool` also calls `store::related` after a write, which is unrelated, but
the recall path is where the mismatch is visible to the model.

## Verification

Write 30 facts with increasing timestamps, then one rule with an older timestamp.
Call `memory_recall` with `{"kind": "rule"}`. It returns no memories. Call it
with the rule's id and the rule is present. The same query with a higher `limit`
returns it.

## Resolution

`store::recent`, `store::by_subject`, and `store::lexical` accept an optional
kind and apply it in SQL before the limit. `MemoryService` exposes
`recent_of_kind`, `subject_of_kind`, and `search_of_kind`, and `RecallTool`
passes its kind into the query. The semantic ids are filtered by kind as well.

Commits: `30e2104` (test), `23b5f8d` (fix). Verified by
`a_kind_filter_applies_before_the_limit`, which stores one old rule behind 30
newer facts and requires `{"kind":"rule"}` to return it.
