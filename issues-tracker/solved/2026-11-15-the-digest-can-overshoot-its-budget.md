# The digest can exceed its configured token budget

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-memory` |

## Observed

The statement fill subtracts a fixed 32-token reserve for the coverage lines,
then appends those lines without measuring them. Two coverage lines can cost
more than the reserve, and a long subject makes one line cost more than a whole
digest.

`crates/hearth-memory/src/lib.rs:319`:

```rust
let statement_budget = tokens.saturating_sub(digest::COVERAGE_RESERVE).max(1);
let items = rank::fill(
    diverse,
    statement_budget,
    config.recall_max_items as usize,
    digest::line_cost,
);
let coverage = self.coverage(&parsed.entities, at)?;
let digest = digest::recall_body(&items, &coverage, now);
```

`crates/hearth-memory/src/digest.rs:204`:

```rust
/// Tokens reserved for the coverage lines and the block wrapper.
///
/// The fill spends the rest on statement lines, so the rendered digest cannot
/// overshoot its configured budget.
pub const COVERAGE_RESERVE: u64 = 32;
```

`coverage_lines` (`digest.rs:130`) renders up to three subjects for the current
count and three for the earlier count, each with a fixed skeleton of 38 and 46
characters. The example line in SPEC 7.5 is 88 characters, about 24 tokens, so
the pair is about 48 and already over the 32-token reserve. A subject may be up
to `SUBJECT_LIMIT` (120 characters), so three long subjects can make one line
about 110 tokens.

## Consequence

The digest sits in the uncached position, so every overshooting token is paid on
every turn. The configured `recall_max_tokens` is not an upper bound: a store
with long subject names can carry a digest several times the budget while
`TokenCounts::over_budget` only logs a warning. The comment above the constant
states the opposite invariant.

## Verification

Seed a store whose matched statements have three long subjects (near the
subject limit), recall with a small `recall_max_tokens`, and assert
`estimate_tokens(&recall.digest) <= recall_max_tokens`. The rendered digest is
larger. `cargo test -p hearth-memory` currently measures the fixture, whose
subjects are short, so it does not reach the case.

## Resolution

`recall_within` measures the coverage lines before the fill
(`digest::coverage_cost`), reserves exactly that, and then drops the
lowest-ranked statement and, if needed, the longest coverage subject
(`digest::trim_coverage`) until the rendered digest fits. `COVERAGE_RESERVE` is
gone.

Commits: `2dbd31f` (fix) after its test commit. Verified by
`the_digest_never_exceeds_its_budget`, which showed 147 tokens against a 120
budget on the parent commit, and now also checks a 60-token budget.
