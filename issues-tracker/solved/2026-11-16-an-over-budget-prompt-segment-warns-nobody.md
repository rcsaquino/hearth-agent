# An over-budget prompt segment warns nobody

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-16 |
| Area | `hearth-cli` |

## Observed

A prompt segment over its budget produces one log line and no other signal. The
agent is never told, and neither the Prompt tab nor the Doctor tab shows
anything.

The budgets involved are `SEGMENT_IDENTITY` 800, `SEGMENT_TOOLS` 1,200,
`SEGMENT_RULES` 800, `SEGMENT_SKILLS` 300, `SEGMENT_RECALL` 300,
`SEGMENT_RUNTIME` 150, and `FIXED_OVERHEAD_TARGET` 4,500, all in
`crates/hearth-core/src/budget.rs`.

## Mechanism

`TokenCounts::over_budget` names the segments over budget. It is called in one
place, `Agent::run` in `crates/hearth-runtime/src/agent.rs`, where the result
goes straight to `tracing::warn!(segments = ?over, "prompt segment over
budget")`.

That write goes to `$HEARTH_HOME/logs/hearth.log` as JSON, through `init_logging`
in `crates/hearth-runtime/src/lib.rs`, which rotates at 10 MB into a single `.1`.

The list never reaches the conversation. The saved prompt document carries the
per-segment counts, built by `prompt::build` in
`crates/hearth-core/src/prompt.rs`, but not the over-budget list, so no message
in the request contains it and the model cannot know that its identity layer is
over budget.

No interface reads it either. The Prompt tab parses each count out of the stored
document through its `token` helper and never compares it to a budget.
`doctor_with` in `crates/hearth-runtime/src/doctor.rs` reports settings,
database, lock state, and optionally live Telegram and provider checks, with no
prompt check. The only place the TUI names the log path is the daemon-start
failure message in `crates/hearth-cli/src/daemon_client.rs`.

`FIXED_OVERHEAD_TARGET` has no runtime reader at all: `tokens.fixed` is
serialized into the document by `prompt::build` and asserted against 4,500 only
in the `s1_prompt` tests, against a small fixture.

## Consequence

The documented "warning" budgets are not warnings. SPEC 6.1 calls the skills
budget "300, warning", Draft 0.29 says the 300-token budget "is a warning", and
SPEC 14 lists fixed prompt overhead at 4.5k tokens or less as a measured target.
A person whose `SOUL.md` is four times the identity budget, or whose tool schemas
are triple the tool budget, sees a normal app; the saved prompt looks normal too.
The cost is visible only to someone who already suspects it and greps the log.

`Guidance.truncated` in `crates/hearth-core/src/guidance.rs` is in the same
position and is covered by
`2026-11-16-guidance-and-identity-files-are-truncated.md`. That file asks for the
cut to stop; this one covers the reporting that stays missing even after it does.

## Verification

Write a `SOUL.md` of about 3,200 tokens, four times the identity budget, and run
one turn. The Prompt tab and the Doctor tab should both name `identity` as over
budget. Today the Prompt tab shows the count with no comparison, Doctor shows
nothing, and the only record is the `prompt segment over budget` line in
`logs/hearth.log`.

## Resolution

`SEGMENT_BUDGETS` in `crates/hearth-core/src/budget.rs` is the one table naming
each budgeted segment and its budget. `TokenCounts::segments` reads it,
`TokenCounts::over_budget` filters it, and `TokenCounts::from_document` reads the
counts a saved prompt recorded.

The Prompt tab prints `▲ identity over budget · 3,200 tok of 800` above the layer
it names. `derive` raises the notice from the saved document and
`Painter::notice` draws it, so the comparison sits beside the count it is about.
`doctor_with` prints one `prompt:` line per offending segment, a fixed-overhead
line against `FIXED_OVERHEAD_TARGET`, and `prompt: ok (N fixed tok)` when the
prompt fits.

The model is not told. It cannot trim `SOUL.md` or its own tool schemas, so a note
in the prompt would cost prefix tokens on a fact only the person can act on. The
existing `prompt segment over budget` log line stays for diagnosis.

Commits: `0cf8f3f` (test), `9648eff` (feat), `aa855da` (docs).

Verified by `every_segment_carries_the_budget_it_is_measured_against` and
`the_counts_are_read_back_from_a_saved_document` in
`crates/hearth-core/tests/s1_budget.rs`, by
`a_prompt_segment_over_budget_is_named` in `crates/hearth-cli/src/ui/prompt.rs`,
and by `doctor_names_a_prompt_segment_over_budget` and
`doctor_reports_a_prompt_that_fits` in `crates/hearth-cli/tests/doctor_offline.rs`.
`./scripts/ci.sh` passes.
