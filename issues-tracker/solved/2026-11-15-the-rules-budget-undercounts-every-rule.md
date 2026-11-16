# The rules budget undercounts every rule, so accepted rules leave the prompt

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-memory` |

## Observed

The write-time rules budget measures a placeholder id, while the render measures
the real id. Both sides of the budget check agree with each other and disagree
with the text the prompt actually carries, so a store can accept more rules than
the blocks shows.

`crates/hearth-memory/src/digest.rs:213`, `rule_cost_text`:

```rust
pub fn rule_cost_text(content: &str) -> u64 {
    let line = format!("m_000000000000: {}\n", clip(content, RULE_CHARS));
    hearth_model::estimate_tokens(&line).max(1) as u64
}
```

A real id is `format!("m_{}", uuid::Uuid::new_v4().simple())` (`store.rs`, the
insert path), which is 34 characters. The placeholder is 14. The check
(`store.rs:214`, `check_rules_budget`) sums `rule_cost` for existing rows and
adds `rule_cost_text` for the new one, so both use the short form. The render
uses the real id (`digest.rs:183`):

```rust
let line = format!("{}: {}", rule.id, clip(&rule.content, RULE_CHARS));
let cost = hearth_model::estimate_tokens(&line).max(1) as u64;
```

The two functions are meant to measure the same line. They do not: one uses a
placeholder id and the other uses the stored id.

## Consequence

With the default 400 token budget and short rules, the store admits more live
rules than the render can show. The excess is reported as `N more rules were not
shown`, but a write that was accepted and reported to the model as pinned then
never reaches the prompt. SPEC 7.6 says a rule is refused rather than silently
evicted, and rules are the one memory path that works without retrieval.

## Verification

With the default configuration, write short pinned rules until the store refuses
one. Compare the number of live rules with the number of `m_` lines in
`MemoryService::rules().text`. The store count is larger. The gap grows with the
number of rules, because each line undercounts by roughly five tokens.

## Resolution

Both sides now measure the same line through `digest::rule_line`, and the
write-time cost uses `store::ID_SAMPLE`, a placeholder id with the shape of a
real `m_` plus 32 hex characters. `store::new_id` and the sample share that
shape, and the extra newline in the old estimate is gone.

Commits: `3bff728` (test), `be22628` (fix). Verified by
`every_live_rule_fits_in_the_rules_block`, which found 36 admitted rules and 25
shown lines on the parent commit and now finds them equal.
