# The Overview skills row shows the review stamp instead of the index cost

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-16 |
| Area | hearth-cli |

## Observed

The `skills` row of the Overview "Work & memory" card reads:

```
skills   2026-11-16T04:30 · 3 changes
```

The left value is `last_skill_review`, truncated to 18 characters
(`crates/hearth-cli/src/ui/dashboard.rs:254`):

```rust
metric(
    "skills",
    format!("{} · {changes} changes", short(review, 18)),
    theme::muted(),
),
```

`review` comes from the daemon status payload
(`crates/hearth-runtime/src/ipc.rs:204`), and `changes` from
`last_skill_review_changes`.

Nothing on the card says how many skills are in effect or what they cost.

## Why it is the wrong thing to show

The stamp is maintenance trivia. It records when the nightly review last ran
(`crates/hearth-runtime/src/skill_review.rs:444`), which the schedule already
guarantees, and `N changes` is only meaningful the morning after a review ran —
for the rest of the day the row is a constant.

Meanwhile the number that matters is invisible: the skills index is text in the
prompt on every turn, and until this issue is worked it was capped and silently
archived learned skills past the cap. Whether the library is 3 skills or 40 is a
direct per-turn cost, and the Overview is where cost belongs.

## Wanted

Replace the stamp with how many skills are active and what the index costs in
tokens, in the shape `4 active · 246/300 tokens`.

The exact wording, whether the token figure is shown at all, and whether a count
is shown in place of a cost are design decisions. The observable requirement is
that the row reports the size of the index in effect rather than the date of the
last review.

## Where the numbers have to come from

Neither figure is reachable from the TUI today.

**Counts.** `SkillStats` (`crates/hearth-runtime/src/skills.rs:113`) already
carries `total`, `active`, `draft`, `quarantined`, `archived`, `user`, `learned`,
and `SkillLibrary::stats` (`skills.rs:477`) fills them. The TUI caches them in
`SkillsView.stats` (`crates/hearth-cli/src/app.rs:101`), but that is loaded by
`SkillsView::reload` (`app.rs:1329`), which runs when the Skills tab is opened or
the library is changed. `work_card` (`dashboard.rs:173`) reads only
`app.status`, so the count is not available on the Overview.

**Tokens.** The rendered index is built per request by `Agent::skills_index`
(`crates/hearth-runtime/src/agent.rs:109`) and never exposed. The daemon status
payload carries no skills field beyond the review stamp. The figure is already
measured — `TokenCounts.skills` (`crates/hearth-core/src/budget.rs:79`) holds it
— but only inside a prompt that has just been built.

So this needs one field added to the status payload, computed from the same
`index()` call the agent makes, rather than a second implementation in the TUI.

## Note on the cap

If `open/2026-11-16-remove-the-skills-index-token-cap.md` lands first, an
`N/M tokens` form has no ceiling to show and should become a bare token count.
The two issues touch the same number and should be worked in a known order.

## Also remove the stamp

The review stamp is not wanted on this card at all. Removing
`last_skill_review` and `last_skill_review_changes` from the row is part of this
change.

Whether the stamp is shown anywhere else in the TUI, or the daemon keeps
publishing it, is a separate question. The daemon fields are also read by
`crates/hearth-runtime/tests/s7_skill_review.rs` and
`crates/hearth-cli/tests/s8_telegram_daemon.rs`, so a worker removing them from
the payload must update those tests; removing them from the card alone does not.

## Test

The render is a string, so the check is a text assertion, as at
`crates/hearth-cli/src/ui/mod.rs:632`, which asserts the card title. The
requirement: the rendered card contains the active count and does not contain the
review stamp.

## Resolution

The daemon status now carries `skills.active` and `skills.index_tokens`. Both are
computed from the same `SkillLibrary::index` call the agent makes, and the count
comes from `SkillStats` after that call. The Work & memory card renders `N active · M tokens`, for example
`4 active · 246 tokens`, because the token cap has already landed, so there is
no ceiling to divide by. The
`last_skill_review` and `last_skill_review_changes` fields leave the card. The
daemon keeps publishing them for other readers.

Commits: `aeee873` (test), `277f86c` (change).

Verified by `cargo test -p hearth-runtime --test ipc_status`, which asserts the
status figure equals `estimate_tokens` of the library's own index, and by
`cargo test -p hearth-cli --lib`, which asserts the rendered card contains the
active count and no review stamp. `./scripts/ci.sh` is green.
