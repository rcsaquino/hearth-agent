# The rules block spends tokens its budget never measured

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-16 |
| Area | hearth-memory |

## Observed

`digest::rules_body` fills the block against a token budget
(`crates/hearth-memory/src/digest.rs:176`). It measures each rule line before it
adds it, then appends a notice line after the loop:

```rust
for rule in rules {
    let line = rule_line(&rule.id, &rule.content);
    let cost = hearth_model::estimate_tokens(&line).max(1) as u64;
    if used + cost > max_tokens {
        break;
    }
    used += cost;
    lines.push(line);
}
if lines.len() < rules.len() {
    lines.push(format!(
        "{} more rules were not shown.",
        rules.len() - lines.len()
    ));
}
lines.join("\n")
```

The notice is pushed outside the accounting. `used` never includes it, so the
rendered block can exceed `max_tokens` by the cost of that line.

A notice line runs 28 to 29 characters at any rule count from 1 to 9,999, which
is 8 to 9 estimated tokens at `chars / 4 * 1.10`
(`crates/hearth-model/src/estimate.rs:8`). With one rule line of 65 tokens
against a 200 budget, three lines fit at 195 used, and the block renders at
203.

The existing test accepts the overshoot. At
`crates/hearth-memory/src/digest.rs:312`, `the_rules_block_stops_at_its_budget`
builds twenty rules of 200 characters each, calls `rules_body(&rules, 200)`, and
asserts the result is `<= 240` — 20% above the budget it was given. The
assertion cannot fail for the fault described here, because 240 is above the
worst case.

## Why it matters here and not in the digest

The recall path already does this correctly. `MemoryService::recall_within`
reserves the coverage lines before it fills the statements
(`crates/hearth-memory/src/lib.rs:321`):

```rust
let statement_budget = tokens
    .saturating_sub(digest::coverage_cost(&coverage))
    .max(1);
```

`coverage_cost` (`digest.rs:204`) measures the coverage text separately so the
fill spends only what is left. The rules block has the same shape — lines plus a
trailing notice — but no equivalent reservation.

The overshoot is 9 tokens at most, so this is not a size problem. It is a
correctness problem: the segment budget check in `TokenCounts::over_budget`
compares the rendered block against `SEGMENT_RULES`, and the render is the thing
that overran its own bound. A block that reports "at most 400" can measure 409.

## Requested

The notice line is counted before it is added, so the rendered block never
exceeds `max_tokens`.

Either reserve the notice cost up front, as the recall path does, or measure the
notice and drop rule lines until the block including the notice fits. The
observable requirement is that `estimate_tokens(rules_body(&rules, n)) <= n` for
every input.

## Test

`the_rules_block_stops_at_its_budget` should assert the bound it names rather
than 20% above it: `<= 200`, not `<= 240`. With that assertion the test fails on
the current code and passes after the change.

## Resolution

Fixed in `2227fd5`, with the failing test in `f3b4bdd`.

**What changed.** `digest::fill_rule_lines` now projects the whole rendered block
before it accepts a line: the joining newlines, and the notice line when rules
remain. A line is added only when the block including its notice fits. If the
budget is smaller than the notice itself, the block renders empty rather than
overrunning. `rules_body` and the new `rules_fit`/`rules_cost` share that one
measure.

**A second arithmetic fault surfaced.** The tighter test first exposed that the
write-time budget summed per-line estimates while the render also pays for
newlines. The sum admitted 25 rules at 400 tokens while the joined block measured
406, so `every_live_rule_fits_in_the_rules_block` failed with one rule hidden.
`store::check_rules_budget` now calls `digest::rules_fit` on the same candidate
block the prompt renders, and the refusal message reports `rules_cost`.

**Verification.** The test now uses 228-character rules so the notice pushes a
block that previously fit over 200, and asserts `<= 200`. It fails on `f3b4bdd`
and passes on `2227fd5`. `cargo test -p hearth-memory --lib` passes 70 tests,
including `every_live_rule_fits_in_the_rules_block`. The memory benchmark is
unchanged: automatic recall 0.933, reachable 0.990, digest p95 139 tokens,
retrieval p95 15.18 ms.
