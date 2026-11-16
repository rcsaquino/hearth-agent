# Remove the warn-only prompt budgets

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-16 |
| Area | `hearth-core` |

## Observed

SPEC 6.1 says every prompt segment budget is a warning, not a refusal. The code
matches that description: none of the segment budgets is enforced. Production
code reads the six `SEGMENT_*` constants in `crates/hearth-core/src/budget.rs`
only through `TokenCounts::segments` and `TokenCounts::over_budget`, and neither
caller refuses, trims, compacts, or changes the request. `FIXED_OVERHEAD_TARGET`
in the same file is read only by `prompt_report` in
`crates/hearth-runtime/src/doctor.rs` and by tests.

The comparison has four readers:

- `Agent::run` in `crates/hearth-runtime/src/agent.rs` logs
  `prompt segment over budget`.
- `prompt_report`, reached from `doctor_with`, prints
  `prompt: <name> over budget` and `prompt: fixed overhead over target`.
- The Prompt tab's `over_layer`, `Notice::OverBudget`, and `Painter::notice` in
  `crates/hearth-cli/src/ui/prompt.rs` print `▲ <name> over budget`.
- Tests in `crates/hearth-core/tests/s1_budget.rs`,
  `crates/hearth-core/tests/s1_prompt.rs`,
  `crates/hearth-cli/src/ui/prompt.rs`, and
  `crates/hearth-cli/tests/doctor_offline.rs`.

None of them affects the request. The counts are already recorded by
`prompt::build` in `crates/hearth-core/src/prompt.rs`, and
`TokenCounts::from_document` reads them back for both read-only surfaces.

## Why

A threshold that never refuses is not a budget; it is a second definition of the
word. Every other budget in the system constrains something:

- `ContextBudget::new` and the checks in `Agent::run` shrink the prompt, compact
  the history, or settle the turn without a provider call.
- `check_rules_budget` in `crates/hearth-memory/src/store.rs` refuses a pinned
  rule that would overflow `memory.rules_max_tokens`.
- `recall_within` in `crates/hearth-memory/src/lib.rs` trims the digest to its
  token budget.

The segment warnings are the exception. A person reading `▲ identity over
budget` in the Prompt tab, or `prompt: identity over budget` in the Doctor output,
is told a limit was crossed by a request that was sent in full. That reads as an
error without being one.

The warning copies also compete with the enforcing settings. `SEGMENT_RULES` and
`SEGMENT_RECALL` duplicate the defaults of `memory.rules_max_tokens` and
`memory.recall_max_tokens`, and tests pin the pairs equal
(`the_rules_budget_matches_the_prompt_segment` in
`crates/hearth-config/tests/budget_settings.rs` and
`the_memory_default_rules_budget_matches_the_prompt_segment` in
`crates/hearth-memory/src/lib.rs`). The settings are the ones that act; the prompt
constants only make the two look like one limit. Removing the warning copies
leaves the enforced defaults authoritative.

The L4 skills warning is stale in the same way: the 300-token index cap was
removed, so `SEGMENT_SKILLS` still measures the rendered index against a number
that no longer limits the library.

## What should happen

Remove the seven warning thresholds and every comparison against them:

- `SEGMENT_IDENTITY`, `SEGMENT_TOOLS`, `SEGMENT_RULES`, `SEGMENT_SKILLS`,
  `SEGMENT_RECALL`, `SEGMENT_RUNTIME`, `FIXED_OVERHEAD_TARGET`,
  `SEGMENT_BUDGETS`, `SegmentCost`, `TokenCounts::segments`, and
  `TokenCounts::over_budget` in `crates/hearth-core/src/budget.rs`, along with
  the re-exports in `crates/hearth-core/src/lib.rs` and
  `crates/hearth-runtime/src/lib.rs`.
- The `prompt segment over budget` log in `Agent::run`.
- `over_layer`, `Notice::OverBudget`, and its arm in `Painter::notice`. The
  Prompt tab keeps the per-layer token counts it already shows.
- The budget and target comparisons in `prompt_report`. `doctor_with` no longer
  prints an over-budget or over-target line for a saved prompt.
- The tests that assert the removed constants and warnings. Replace the
  observable ones with tests that the same over-budget prompt still shows its
  counts and stays quiet, rather than deleting the coverage of the Prompt tab and
  Doctor surfaces.

Keep the measurements and the enforced limits:

- `TokenCounts`, `TokenCounts::from_document`, and the per-segment, total, and
  fixed counts in the saved prompt document. Removing a threshold must not
  remove the cost record that the Prompt tab and future diagnostics read.
- `ContextBudget` and the `usable`, `soft`, and `hard` checks in the agent.
- `memory.rules_max_tokens`, `memory.recall_max_tokens`, `check_rules_budget`,
  `rules_fit`, and `recall_within`.
- `INDEX_MAX_ACTIVE`, which is an enforced count, and the test-only schema caps
  in `crates/hearth-runtime/src/tools.rs`, which are assertions rather than
  runtime warnings.

Update the documents that state the thresholds:

- SPEC 6.1's budget column and its "every segment budget is a warning" prose,
  the 9.6 skills-index bullet, the Appendix A rows for recall, fixed overhead,
  tool schemas, and the skills index, and the section 14 target rows.
- DECISIONS entries "M5a context budgets", "Every segment budget is reported to
  the person", "The rules budget is 800 tokens", and the skills-index entry.
  Add a new entry recording that the warning thresholds were removed and why.
- Bump the spec Draft and state the removal in the header note.

## Design constraints

- This change removes warnings, not work. It must not add a refusal, a new prompt
  layer, or a model-facing note, and it must not change how a turn is sent.
- The only context limit remains `ContextBudget`, and it stays enforced before
  the provider request.
- Do not remove an enforcing memory budget just because it shares a numeric
  default with a removed prompt warning. The settings are separate from the
  segment constants and stay.
- The earlier solved issue
  `2026-11-16-an-over-budget-prompt-segment-warns-nobody.md` added the notices
  this change removes. Leave it in `solved/`; its Resolution records that
  revision. This issue supersedes the notice surface, not the bug it documented.

## Test

A saved prompt whose `identity` count is 3,200 and whose fixed total is above
4,500 should still show `3,200 tok` on the identity layer in the Prompt tab, with
no `over budget` text. `doctor_with` should print no `over budget` or
`over target` line for the same document. Both assertions fail today because the
notices exist, and pass after the removal. Put them at the existing seams in
`crates/hearth-cli/src/ui/prompt.rs` and
`crates/hearth-cli/tests/doctor_offline.rs`.

Keep the count assertions where they remain true:

- `token_counts_sum_and_report_the_fixed_overhead` in
  `crates/hearth-core/tests/s1_budget.rs` keeps the sums and drops the
  `over_budget` assertion.
- `the_prompt_reports_segment_tokens_and_fixed_overhead` in
  `crates/hearth-core/tests/s1_prompt.rs` keeps the counts and drops the
  fixed-target comparison and the empty `over_budget` assertion.
- `segment_budgets_match_the_spec`, `an_over_budget_segment_is_named`, and
  `every_segment_carries_the_budget_it_is_measured_against` lose their subject
  and should be replaced or removed.
- `the_rules_budget_matches_the_prompt_segment` and
  `the_memory_default_rules_budget_matches_the_prompt_segment` keep the enforced
  800 default and drop the equality with `SEGMENT_RULES`; rename them so they no
  longer claim a prompt segment.
- `a_prompt_segment_over_budget_is_named`,
  `doctor_names_a_prompt_segment_over_budget`, and
  `doctor_reports_a_prompt_that_fits` are replaced by the quiet-surface tests
  above.

The tests for the context gate, the rules write refusal, and the recall trim
stay and must keep passing.

## Not established

Whether the Doctor should keep any prompt line at all once the comparison is
gone. The Prompt tab already shows the counts, so a Doctor line that only repeats
them is optional. The constraint is that no line compares a count to a budget or
target.

## Resolution

Removed the seven warning thresholds and every comparison against them. The
per-layer token counts stay in the saved prompt document and the Prompt tab; the
Doctor now prints only `prompt: ok (N fixed tok)`. The Prompt tab no longer emits
an `over budget` notice, and the agent no longer logs one. `ContextBudget` and the
memory budgets that actually constrain a request are unchanged.

Commits: `ca00827` (test), `a7a0cf0` (feat), `81478b8` (docs).

Verified by `an_over_budget_prompt_still_shows_its_counts` in
`crates/hearth-cli/src/ui/prompt.rs`, which renders an identity layer at 3,200
tokens and a fixed total above 4,500 with no warning text, and by
`doctor_keeps_prompt_counts_quiet` in `crates/hearth-cli/tests/doctor_offline.rs`,
which asserts the same saved document prints neither `over budget` nor `over
target`. The core count tests keep their sums and drop the removed comparisons.
`./scripts/ci.sh` passes.
