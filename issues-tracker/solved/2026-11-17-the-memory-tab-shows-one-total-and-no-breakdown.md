# The Memory tab shows one total and no breakdown

| | |
| --- | --- |
| Kind | feature |
| Reported | 2026-11-17 |
| Area | `hearth-cli` |

## Observed

The Memory tab title is the only count on the tab. `list` in
`crates/hearth-cli/src/ui/memory.rs` builds it from the length of
`MemoryView::filtered`:

```rust
" Memory · {} saved "
```

One number, labelled "saved", for three different states.

`MemoryRecord` in `crates/hearth-runtime/src/inspect.rs` already carries the
distinction. `pinned` is true only for a row that is pinned and still current,
and `retired` is true when `valid_to` is set, so a row is pinned, active
(current and unpinned), or retired. `store_label` in
`crates/hearth-cli/src/app.rs` and `status` in `ui/memory.rs` already render the
three as `★`, `●` and `○`.

The list shows all three interleaved and the title adds them together. A store
with 8 pinned rules, 60 current facts and 400 retired rows reports `468 saved`,
which is true of nothing an operator can act on. The pinned rows cost prompt
room on every turn, the current rows are the working set, and the retired rows
cost nothing and are read only when history matters.

The Overview card is no help. Its `memory` row in `work_card`
(`crates/hearth-cli/src/ui/dashboard.rs`) reads `memory_items` from the status
payload, which `StatusContext` fills from `MemoryService::count`, and that counts
current rows only through `store::count` and its `valid_clause`. So the Overview
reports current rows without saying that is what it is, and never mentions
pinned or retired at all.

## Wanted

The Memory tab says how many memories are pinned, how many are active, and how
many are retired, rather than one combined total.

Exact wording, placement and abbreviation are design decisions. The observable
requirement is that all three figures are visible at once, that they are
distinguishable from one another, and that they account for what the tab is
showing.

Two behaviours worth stating because they are easy to get wrong:

- **The breakdown describes the store, not the loaded page.** Today the title is
  also a filtered count: `MemoryView::filtered` applies the search string before
  `list` takes its length, so typing a search turns the title into a match count
  still labelled "saved". Whatever the new figures say, a search must not
  silently restate them as different numbers. If a match count is wanted while
  filtering, it needs to read as a match count.
- **Retired means retired.** A row that was pinned before its window closed is
  retired, not pinned. `list_memories` already clears the displayed pin for it,
  and the breakdown must agree with the marks the list draws, or the same tab
  will contradict itself.

## Where the numbers have to come from

`MemoryRecord.pinned` and `MemoryRecord.retired` are enough to count the three
groups, but not from the loaded page. `MemoryView::reload` asks
`inspect::list_memories` for a fixed 1000 rows, and that function applies it as a
query `LIMIT`. Counting the returned rows therefore reports at most 1000 and
understates every figure in a large store. The counts need to come from the
store, the way `store::count` already counts the current window, and not from
whatever page the tab happens to have loaded.

The status payload is the existing route for a daemon-side figure, and
`StatusContext` already caches the slow values it computes for five seconds, so a
breakdown computed there costs one query behind that TTL rather than one per
probe. Whether the figures travel in the status payload, in the inspect call the
tab already makes, or both, is a design decision.

## Why

This is the number an operator needs before changing a rule or pruning the store,
and it is currently unavailable anywhere in the TUI. Retired rows are permanent:
nothing is hard-deleted, so they only accumulate, and a total that mixes them
with live rows grows forever while meaning less each month. Pinned rules are the
most expensive rows in the store because they are injected every turn, and the
tab cannot say how many there are.

The `solved/2026-11-16-the-memory-tab-orders-pinned-active-inactive.md` change
taught the list to draw the three states as three ordered groups. The title is
the one place on the tab that still does not know the groups exist.

## Scope

- Do not change what the list shows, its ordering, or its search behaviour.
- Do not hard-delete, archive, or retire anything. This is a display change.
- Do not remove `memory_items` from the status payload; the Overview reads it and
  `crates/hearth-cli/tests/s8_telegram_daemon.rs` asserts it.
- Do not add a new tab or a settings option.
- Do not report a figure computed from the 1000-row page as a store total.

## Test

The render is a string, so the check is a text assertion, as
`memory_tab_shows_saved_memories` in `crates/hearth-cli/src/ui/mod.rs` already
does.

- A store with known pinned, active and retired rows renders all three figures,
  and each equals the count for its state.
- A store with zero retired rows says so, and does not omit the figure or print a
  dash that reads as an error.
- A search that matches a subset does not change the three figures to match
  counts.
- A retired row that was pinned before it closed counts as retired.
- The totals are correct when the store holds more rows than the list loads.

## Not established

- Whether the Overview card should also show the breakdown or keep its single
  current-row figure. The request is about the Memory tab; the Overview is
  mentioned above only to record that it cannot supply this today.
- Whether the figures belong in the tab title, in a footer line, or beside the
  search row. Wording is left to the implementer.


## Resolution

The Memory tab title is now `Memory · N pinned · N active · N retired`. The
figures come from `inspect::memory_state`, which counts through a read-only
connection with `store::breakdown`, not from the 1,000-row page. A search adds a
separate `N match` clause; it never restates the three store figures. A zero
figure is printed as `0 retired`, not omitted. A row that was pinned before its
window closed counts as retired, because the SQL is `valid_to IS NULL AND pinned`
for pinned and `valid_to IS NOT NULL` for retired.

The same counts travel in the daemon status payload under `memory`, beside the
existing `memory_items` current-row figure that the Overview and
`s8_telegram_daemon` read. The audit line reports the last consolidation counts,
and a report candidate carries `⚠` in the list and an `audit` label in the
detail pane.

Commits: `a7931ec` (test), `ddcdf4a` (feat), `7187b1f` (CLI dependency).

Verified by `memory_tab_shows_pinned_active_and_retired_totals`,
`a_search_keeps_the_store_totals_and_adds_a_match_count`,
`a_zero_retired_count_is_still_shown`,
`a_retired_pinned_row_counts_as_retired`, and
`memory_tab_marks_a_report_candidate` in `crates/hearth-cli/src/ui/mod.rs`; by
`memory_state_counts_the_store_not_the_loaded_page`, which writes 1,007 rows
through a 1,000-row page; and by
`status_reports_the_memory_breakdown_and_the_last_audit` for the payload.
`./scripts/ci.sh` passes.
