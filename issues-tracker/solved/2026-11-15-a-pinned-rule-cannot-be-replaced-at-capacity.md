# A pinned rule cannot be replaced when the rules budget is full

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-memory` |

## Observed

At the rules budget, a pinned write that names an existing rule as `supersedes`
is refused, even though replacing a rule should free the space it uses.

`crates/hearth-memory/src/store.rs:513`, `insert_in`:

```rust
if item.pinned {
    check_rules_budget(writer, &item.content, rules_budget)?;
}
```

The `supersedes` target is resolved later, at `store.rs:590`:

```rust
let supersedes = supersedes.or(earlier.map(|earlier| earlier.id));
if let Some(previous) = &supersedes {
    let closed = writer.execute(
        "UPDATE memories SET valid_to = ?, superseded_by = ?, pinned = 0 \
         WHERE id = ? AND valid_to IS NULL",
```

`check_rules_budget` (`store.rs:214`) sums `rule_cost` over every live pinned
rule and adds the new rule's cost. It has no parameter that could exclude the
row being replaced, so the old rule is counted and the new one is added.

The refusal then tells the model to do what it just did:

```text
the rules budget is full at {used} of {budget} tokens; replace or forget one of {ids}.
```

## Consequence

A rules block that fills up becomes write-only by deletion. The model retries
the replacement, gets the same error, and either gives up or spends another turn
forgetting a rule. SPEC 7.6 says an explicit replacement is the way to change a
rule and that the block is refused, not silently evicted; the accepted
replacement path does not exist at capacity.

## Verification

Set `rules_max_tokens` to 400. Write two pinned rules that fill it. Then write a
third pinned rule that names one of them in `supersedes`. Assert the write
succeeds and the replaced rule is no longer live. Today it returns
`Code::RulesFull`.

## Resolution

`check_rules_budget` takes the row being replaced and excludes it from the used
total, since the same transaction closes it. The refusal message lists only the
rows that actually hold space.

Commits: `f7592c3` (test), `4378367` (fix). Verified by
`a_pinned_replacement_fits_at_capacity`, which filled the block, replaced one
rule with another of the same cost, and saw the replacement accepted and the old
row retired.
