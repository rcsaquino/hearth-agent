# The decision log still documents a turn limit that does not exist

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-16 |
| Area | docs |
| Build | 1d380d9 |

## Observed

`docs/DECISIONS.md:341` states, under "2026-11-08: M2b-3 tools, spill, and the
tool loop":

> The agent turn limit counts provider requests. Reaching it saves a notice,
> settles the submission unanswered with reason `turn_limit`, and fails the task.

The code does not do this, and the limit is not wanted.

## Why the claim is false

**There is no limit in the loop.** `crates/hearth-runtime/src/agent.rs:461` opens
`'turn: loop {`. The only increments are at `agent.rs:438` and `agent.rs:667`,
both guarded by `if !calls.is_empty()`. No count is compared to any threshold.

**The setting was removed.** `agent.max_turns` is not a field in
`crates/hearth-config/src/settings.rs`. The tests that pin its absence:

- `crates/hearth-config/tests/budget_settings.rs:48`,
  `agent_has_no_turn_limit_setting`
- `crates/hearth-runtime/tests/s6_agent.rs:565`,
  `a_long_tool_loop_is_not_cut_short_by_a_turn_limit`, which scripts 125 tool
  rounds and requires them all to complete
- `crates/hearth-cli/src/settings_form.rs:1998`,
  `identity_has_no_soul_file_row_and_no_turn_limit_row`

**The `turn_limit` reason is no longer produced.** A search for the string in
`crates/` finds it in no source file. It appears only in
`crates/hearth-core/tests/s1_session.rs:680`, where the test calls
`session.settle_with_error(..., Some(json!({"reason": "turn_limit"})))` by hand
to check that a settle with an error round-trips. Nothing in the runtime emits
that reason.

**`Unanswered` is still reachable, by another route.** `Agent::settle_failure`
(`agent.rs:942`) settles a submission as `SubmissionStatus::Unanswered` with a
called-supplied reason. Its only caller uses it for the context budget gate at
`agent.rs:535` with reason `budget`. So the status is live; the `turn_limit`
reason that the decision log ties to it is not.

## Why it matters

`docs/DECISIONS.md` opens by saying it "records interpretations, deviations, and
approved decisions that affect the build", and that `SPEC.md` remains binding. A
reader — human or agent — who trusts the entry will look for a limit that is not
there, or will assume a runaway loop is bounded when it is not.

The consequence is not only confusion. The only bound that actually stops a
non-converging loop today is the context gate at `agent.rs:535`, which fails the
submission with the message at `agent.rs:540` — "The conversation is too large
for the model window even after shrinking." That reports a size fault for a loop
that did not converge. An operator who believes a turn limit exists has no reason
to look for the real cause.

## Requested

Correct the entry so it describes the behaviour that is intended.

An absent turn limit is the accepted design, confirmed on 2026-11-16 and recorded
in `solved/2026-11-16-the-agent-loop-has-no-turn-cap.md` (closed as working as
intended). The entry should say so, rather than describing a limit that was
removed.

The surrounding entries in the same section are accurate and need no change: the
tool pairing rule, the spill threshold, and the shell timeout all still hold.

## A separate weakness in the same area

`crates/hearth-config/tests/budget_settings.rs:48` is named
`agent_has_no_turn_limit_setting`, which claims more than its body checks:

```rust
fn agent_has_no_turn_limit_setting() {
    let settings: Settings = serde_json::from_str(&models()).expect("settings");
    assert_eq!(settings.agent.workspace, None);
}
```

It asserts a default for `workspace` and never mentions `max_turns`. It would
keep passing if a `max_turns` field returned, as long as the JSON in `models()`
did not set one. The test that actually enforces the absence is
`s6_agent.rs:565`, which runs 125 rounds.

That is not part of this correction, but a worker touching these tests should know
the name overclaims. Splitting it into its own issue is reasonable if the fix is
not made here.

## Test

No test pins a documentation line, and this change does not need one. The
verification is that `grep -rn "turn_limit" --include=*.rs crates/` returns only
the `s1_session.rs` test that sets the reason by hand, and that the entry no
longer claims otherwise.

## Resolution

Fixed in `3d01d5c`.

**What changed.** The M2b-3 bullet in `docs/DECISIONS.md` now states the accepted
design: no turn limit, the three ways a loop ends, and the two tests that encode
it. It no longer names a `turn_limit` reason that no runtime code produces.

**The test-name overclaim is fixed too.** `agent_has_no_turn_limit_setting` now
parses `{"agent":{"max_turns":10}}` and requires the deserialization to fail, so
the name matches the assertion. The test passes on first run because
`Agent` already carries `deny_unknown_fields`; it is a guard, not a red/green
cycle.

**Verification.** `grep -rn "turn_limit" --include=*.rs crates/` returns
`budget_settings.rs:48` and `s6_agent.rs:565` and `settings_form.rs:1998` (all
test names), and `s1_session.rs:680`/`:685` (a settle that sets the reason by hand
to test the round trip). No source file produces it.
`cargo test -p hearth-config --test budget_settings` passes 11 tests.
