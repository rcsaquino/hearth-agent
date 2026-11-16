# The task tool takes a token budget that can zero the helper's context

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-16 |
| Area | hearth-runtime |

## Observed

The `task` tool advertises a `token_budget` argument
(`crates/hearth-runtime/src/helpers.rs:117`, `"minimum": 1000`) and writes it into
the helper task's input (`helpers.rs:211`). The worker reads it back
(`helpers.rs:435`) and, when it is below the configured window, builds a smaller
`ContextBudget` (`helpers.rs:446`):

```rust
let helper_budget = if token_budget < self.config.budget.context_window {
    ContextBudget::new(
        token_budget,
        self.config.budget.max_output_tokens,
        0.65,
        0.85,
        self.config.budget.keep_recent,
    )
} else {
    self.config.budget
};
```

`ContextBudget::new` subtracts the reply reserve before it computes the usable
window (`crates/hearth-core/src/budget.rs:44`):

```rust
let reserve = max_output_tokens
    .max(RESERVE_FLOOR)
    .saturating_add(RESERVE_EXTRA);
let usable = context_window.saturating_sub(reserve);
```

`RESERVE_FLOOR` is 4,096 and `RESERVE_EXTRA` is 1,024, both at
`crates/hearth-core/src/budget.rs:19`. `max_output_tokens` defaults to 32,000 at
`crates/hearth-config/src/settings.rs:396` and is not set in a default settings
file.

So any `token_budget` below 32,000 + 1,024 = 33,024 gives `usable = 0`. The
argument's stated minimum is 1,000, which is well below that floor. Nothing
compares the requested value to the reserve.

An agent that spawns a helper with, for example, `token_budget: 20000` gets a
budget with `usable = 0`. The agent gate at
`crates/hearth-runtime/src/agent.rs:535` then tests
`prompt.tokens.total > self.budget.usable`. Every prompt is greater than zero, so
the first turn fails at once.

The failure is reported with the message at `agent.rs:540`: "The conversation is
too large for the model window even after shrinking. Start a new chat or let
compaction run." Both claims are false in this case. The conversation is empty,
and compaction cannot recover from a zero limit.

A reproduction in the local store: task `01a1e000-ed3a`, spawned with
`token_budget: 20000`. Its helper conversation held two entries and 443 tokens.
It failed in the same second it started, and its report file contains that
message.

## Why the parameter does not earn that risk

`token_budget` substitutes for `context_window` and nothing else. It moves four
derived numbers: `usable`, `soft`, `hard`, and `keep_recent`. It does not cap
output, because `max_output_tokens` passes through unchanged. It does not cap
time. It does not shrink the identity layer, the tool schema, the rules, or the
guidance, which are fixed segments paid in full on every turn.

A helper also starts a new conversation. On its first turn the only transcript
is the brief. So the ceiling constrains almost nothing that is not already fixed,
while the cost of setting it too low is a total loss of the run and a report that
names the wrong cause.

## Requested

Remove `token_budget` from the `task` tool.

A helper should always run on the configured `ContextBudget`, the same one the
daemon builds for the main agent. A caller that still sends the argument should
not fail; the argument is simply no longer part of the interface.

## Design constraint

`wall_seconds` stays. It bounds a run by time, it fails as `Cancelled` with the
reason `cancelled` rather than as a false size fault, and it is the only bound on
a helper that hangs or loops.

## Suggested approach

One shape that works, not a requirement:

1. Delete the schema entry at `helpers.rs:117`.
2. Delete the argument read at `helpers.rs:175` and the task input entry at
   `helpers.rs:211`.
3. Delete the worker read at `helpers.rs:435` and the `helper_budget` block at
   `helpers.rs:446`, then pass `self.config.budget` to `.with_budget` at
   `helpers.rs:562`.
4. Update `docs/DECISIONS.md` under "M7b helpers", which currently states that
   `token_budget` is enforced as the helper's context-window cap.

`ContextBudget` stays in the `hearth-runtime` import list because `HelperConfig`
holds one.

## Test

A test should fail before the change and pass after. Two assertions cover it:

- `TaskTool::spec()` has no `token_budget` property and still has `wall_seconds`.
- A helper spawned with `token_budget: 20000` — below the reserve — reaches
  `TaskStatus::Done` rather than `Failed`.

`crates/hearth-runtime/tests/s7_helper_pool.rs` already has a scripted provider
and a fake Telegram server, so both fit there.

## Not established

Whether a cost bound is wanted at all. A per-task spend cap in the scheduler would
serve that goal directly, because it measures money rather than context. That is a
larger change and is not part of this one.

## Resolution

Fixed in `7fc1930`, with the failing tests in `57642bc`.

**What changed.** `token_budget` is gone from the task tool schema, the spawn
arguments, the task input, and the worker. `spawn` accepts an old caller that
still sends it and ignores the argument. The helper is built with
`self.config.budget`, the same `ContextBudget` the daemon gives the main agent.
`wall_seconds` is unchanged. `ContextBudget` stays in the import list because
`HelperConfig` holds one. `docs/DECISIONS.md` "M7b helpers" no longer claims the
cap, and a new "A helper runs on the configured budget" entry records why.

**Verification.** `the_task_tool_offers_no_token_budget` fails on `57642bc`
because the schema still had the property, and
`a_helper_survives_a_token_budget_below_the_reply_reserve` fails with
`left: Failed, right: Done` when spawned with `token_budget: 20000`. Both pass on
`7fc1930`. `cargo test -p hearth-runtime --test s7_helper_pool` passes all six
tests, including the existing run, report, ETA, kill, and missing-report cases.
