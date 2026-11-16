# The rules budget leaves room for fewer than two more rules

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-16 |
| Area | hearth-memory |

## Observed

The rules budget is 400 tokens, set in two places that must agree:

- `crates/hearth-core/src/budget.rs:8` — `pub const SEGMENT_RULES: u64 = 400;`
  This one only warns. `TokenCounts::over_budget` (`budget.rs:126`) pushes
  `"rules"` when the segment passes it, and the agent logs a warning.
- `crates/hearth-config/src/settings.rs:420` — `default_rules_max_tokens()`
  returns 400, exposed as `memory.rules_max_tokens`. This one enforces. It is
  passed to `store::insert` and `store::insert_many`, and to
  `digest::rules_body` (`crates/hearth-memory/src/lib.rs:247`).

A pinned rule line costs its content plus a 36-character id prefix. The longest
line a rule can produce is 436 characters, because `RULE_CHARS` caps content at
400 (`crates/hearth-memory/src/digest.rs:26`), and
`hearth_model::estimate_tokens` charges `chars / 4 * 1.10`
(`crates/hearth-model/src/estimate.rs:8`). So one maximal rule costs 120 tokens,
or 30% of the budget.

Four pinned rules in the live store occupy 272 tokens, which leaves 128. At the
68-token average of those four, that is room for one more rule and not two.

## Why 400 is now too small

The budget was sized when a rule was an occasional instruction. It is now the
layer that carries standing rules — the ones that must hold on every turn. The
four rules in use are exactly that shape: identity, authorization, reply style,
and a repository rule. Each new standing rule costs 60 to 120 tokens, so the
budget fills after three or four of them.

The failure is abrupt rather than degrading. A write that would pass the cap is
refused outright (`crates/hearth-memory/src/store.rs:249`):

```
the rules budget is full at {used} of {budget} tokens; replace or forget one of {ids}.
```

So the operator learns at the moment of a write, mid-task, that a rule they need
cannot be saved.

## There is room

The fixed prompt overhead target is `FIXED_OVERHEAD_TARGET = 4_500`
(`crates/hearth-core/src/budget.rs:17`). The newest prompt blob in the live store
reports:

| Segment | Tokens |
| --- | --- |
| identity | 818 |
| tools | 1,944 |
| rules | 272 |
| skills | 246 |
| runtime | 11 |
| guidance | 257 |
| **fixed** | **3,548** |

Everything except rules is already committed. With rules at 800 the fixed
overhead reaches 4,076, still 424 under the target. The rules budget can go to
about 1,200 before the target is breached with the segments as they are.

Against the model window the cost is negligible. `models.context_window` defaults
to 128,000 (`crates/hearth-config/src/settings.rs:392`), where 800 tokens is
0.63%. The live settings use 1,000,000, where it is 0.08%.

The configured value is clamped to `50..4_000`
(`crates/hearth-memory/src/lib.rs:80`), so 800 needs no other change to be
accepted.

## Requested

Raise the rules budget to 800 tokens.

Both numbers must move together. If `memory.rules_max_tokens` alone is raised,
`SEGMENT_RULES` warns on every turn once the block passes 400, and the warning
stops carrying information. If `SEGMENT_RULES` alone is raised, nothing changes
except the silence, because the config value still gates the write.

`docs/SPEC.md` states the number in three places and must follow:

- `SPEC.md:820` — the layer table, `| L3 rules | ... | 400 |`
- `SPEC.md:1097` — "Default budgets are 400 tokens for rules and 300 for the
  digest"
- `SPEC.md:1563` — the settings table, `| memory.rules_max_tokens | 400 | ... |`

## Do not confuse two limits

`SPEC.md:1036` also says "A rule is capped at 400 characters". That is
`RULE_CHARS` in `crates/hearth-memory/src/digest.rs:26`, a character limit on one
rule's content. It is not the token budget and must not change with it.

## Test

`crates/hearth-core/tests/s1_budget.rs:44` asserts `SEGMENT_RULES == 400`. The
test name is `segment_budgets_match_the_spec`, so it exists to keep the constant
and the spec in step. It must be updated with the change, and a new assertion
should pin the config default to the same number so the two cannot drift apart
again.

## Not established

Whether 800 is the right ceiling or a first step. The fixed overhead target
allows up to about 1,200 today, but the identity and tools segments are already
over their own caps (818 against 800, 1,944 against 1,200), so the room above 800
depends on those being brought back inside their budgets first. 800 is the value
that leaves room without depending on that work.

## Resolution

Fixed in `ace1f7d`, with the failing tests in `ad4d627`.

**What changed.** `SEGMENT_RULES` and `default_rules_max_tokens` both read 800.
A third default had the same 400: `MemoryConfig::default` in `hearth-memory`,
used when a caller builds the service without the daemon's settings; it is 800
too. `docs/SPEC.md` moves all four mentions: the L3 layer table, the 7.5 default
sentence, the 7.6 cap, and the settings table. `RULE_CHARS` stays 400 characters,
which the 7.7 text states separately. `docs/DECISIONS.md` records the choice and
the three pinned numbers.

**Verification.** The tests fail on `ad4d627` and pass on `ace1f7d`.
`segment_budgets_match_the_spec` now asserts 800. A new
`the_rules_budget_matches_the_prompt_segment` asserts the settings default is 800
and equals `SEGMENT_RULES`, and `the_memory_default_rules_budget_matches_the_prompt_segment`
does the same for `MemoryConfig`. `hearth-core` s1 budget 8 passed,
`hearth-config` budget settings 11 passed, `hearth-memory` lib 71 passed, and
`hearth-cli` lib 108 passed. The memory benchmark is unchanged: automatic 0.933,
reachable 0.990, digest p95 139 tokens, retrieval p95 15.18 ms.
