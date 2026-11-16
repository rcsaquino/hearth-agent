# The Memory tab orders pinned, then active, then inactive

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-16 |
| Area | `hearth-cli` |

## Observed

`MemoryView::filtered` in `crates/hearth-cli/src/app.rs` returns records in the
order `inspect::list_memories` returned them. `list_memories` in
`crates/hearth-runtime/src/inspect.rs` orders by current pinned first, then by
`updated_at` descending and `rowid` descending.

The `pinned` and `retired` flags come from the query. `pinned` is true only for a
row that is pinned and still current, and `retired` is true when `valid_to` is
set. So the order has two groups, not three: every current unpinned row and every
retired row share the second group. A recently updated retired row can therefore
sit above a current fact.

The Memory tab already distinguishes the states with `status` and `store_label` in
`crates/hearth-cli/src/ui/memory.rs`: `★` for pinned, `●` for current, `○` plus
`(retired)` for a closed window. The list order does not follow those marks.

`MemoryView::filtered` filters the records without reordering them, and
`MemoryView::selected_record` indexes the filtered view, so filtering preserves
the interleaving.

## Wanted

The list reads in three groups, in this order:

1. Pinned: current rows with `MemoryRecord.pinned` true.
2. Active: current rows that are not pinned and not retired.
3. Inactive: retired rows, where `MemoryRecord.retired` is true.

Within each group, keep the current recency order: `updated_at` descending, then
`rowid` descending. A row that was pinned before its window closed is inactive,
not pinned; the query already clears the displayed pin for it.

The order survives filtering. Typing in the search box narrows the rows but never
moves an active row below an inactive one. The 1,000-row read in
`MemoryView::reload` applies the group order before the limit, so a full list
cannot fill with inactive rows and hide a current statement behind them.

## Why

The status marks say the three states are different, but the list contradicts
them: a hollow `○` row can appear above a live `●` row, which makes the closed
statement look more current. Pinned rules are the strongest standing memory and
belong at the top; current facts and commitments come next; closed windows are
history and belong at the bottom.

The grouping also matches how the records are read. "What is true now" excludes
closed windows, and an as-of read is the way to reach them. A list that mixes
history into the live rows makes the operator scroll past statements that no
longer hold.

## Scope

- This is a list order change only. The `pinned` and `retired` flags, the status
  glyphs, the `(retired)` label, filtering, and the detail pane stay as they are.
- Do not sort by content, kind, subject, or status text. The only keys are the
  group and then the existing recency order.
- Memory ranking, recall, and skill ordering are not part of the change.
- The 1,000-row limit is applied by `list_memories`, so the order the query
  returns must be the grouped order. Sorting only the already-limited vector in
  `MemoryView::filtered` cannot bring back an active row the limit dropped.

## Test

Add a Memory tab test that writes one of each: a current pinned rule, a current
unpinned fact, and a fact with a closed window whose `updated_at` is newer than
the active fact's. Recency alone would put the inactive row above the active one;
the desired order puts it last.

Assert the order of the records, either on the `list_memories` result or on
`MemoryView::filtered`, and assert that the rendered Memory tab uses it. The
existing `memory_tab_marks_a_replaced_rule_as_retired` covers the glyph and the
`(retired)` label and stays; it does not cover order.

## Documents

- Update the Memory row in the 2.3 application table in `docs/SPEC.md` to name the
  order, bump the Draft, and mention the grouping in the header note.
- Record the grouping and the recency keys in `docs/DECISIONS.md` near the
  existing Memory tab decision.

## Resolution

`inspect::list_memories` orders by one `CASE` group key: current pinned rows,
current unpinned rows, then rows with a closed window. Within each group it keeps
`updated_at DESC, rowid DESC`. The group key is in the query, so the 1,000-row
limit sees the same order and filtering in `MemoryView::filtered` preserves it.

Commits: `ea31953` (test), `edabf0e` (feat), `0f8f4f7` (docs).

Verified by `list_memories_groups_pinned_active_then_inactive` in
`crates/hearth-runtime/tests/inspect.rs` and
`memory_tab_groups_pinned_then_active_then_retired` in
`crates/hearth-cli/src/ui/mod.rs`. Both write a current pinned rule, a current
fact, and a newer retired fact, then assert the retired row is last in the query
and in the rendered tab. `./scripts/ci.sh` passes.
