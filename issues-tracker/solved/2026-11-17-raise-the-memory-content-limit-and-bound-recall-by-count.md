# Raise the memory content limit to 200 characters and bound recall by count

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-17 |
| Area | `hearth-memory` |

## Observed

`store::CONTENT_LIMIT` in `crates/hearth-memory/src/store.rs` is 150. `validate`
counts characters after trimming, refuses a longer write with `invalid_input`,
and names the actual length and the limit. The limit applies to every memory,
pinned rules included; `RULE_LIMIT` was removed by the solved issue
`2026-11-16-every-memory-is-capped-at-150-characters.md`.

The same number appears in four other places:

- `digest::LINE_CHARS` is 150, so a newly written memory renders whole in the
  recall line. `clip` still protects a row written before the limit.
- `extract::MAX_CHARS` is 150. `MESSAGE_CHARS` is a separate 1,600 guard, so a
  long user message is still scanned for a shorter candidate.
- The `memory_save` schema in `crates/hearth-memory/src/tools.rs` says "at most
  150 characters".
- SPEC 7.2, 7.3, 7.6, and Appendix A, plus the 2026-11-16 DECISIONS entry,
  name 150.

Recall has two bounds. `memory.recall_max_items` is 10 and
`memory.recall_max_tokens` is 700. The second one shapes more than size:

- `MemoryService::recall_within` in `crates/hearth-memory/src/lib.rs` subtracts
  `digest::coverage_cost` from the token budget, passes the remainder to
  `rank::fill`, and then drops rows from the end until the rendered digest fits.
- `rank::fill` orders candidates by `score / cost`, so a shorter statement beats
  a longer one at equal score even when the budget is not the binding bound.
- `Agent` in `crates/hearth-runtime/src/agent.rs` re-calls `recall_within` with a
  budget of two thirds, down to `digest_floor`, whenever the assembled prompt is
  over `budget.usable`.
- The status report in `crates/hearth-runtime/src/ipc.rs` publishes `max_tokens`
  and `max_items`, and the Overview card in `crates/hearth-cli/src/ui/dashboard.rs`
  renders them as `{items}×{tokens} recall`.

The item cap already bounds the block. A row costs at most 200 characters of
content plus an id, kind, subject (120 at most), and age. Coverage is at most two
lines of three subjects each. With ten items and the 200-character limit, the
rendered digest cannot exceed about 1,200 estimated tokens, and the ordinary
digest is about 700. The benchmark's digest p95 is 139 tokens on statements of
30 characters at the median.

## Wanted

One content limit and one recall bound.

**Content:** 200 characters, counted after trimming, for a fact, a preference,
and a pinned rule alike. Exactly 200 is accepted; 201 is refused, names its
length and the limit, and saves nothing.

- `digest::LINE_CHARS` becomes 200, so a new memory is shown whole. A row
  written before the limit is clipped at 200 for display. `RULE_CHARS` stays
  400; a 200-character rule fits it, and it is still only a read-side guard.
- `extract::MAX_CHARS` becomes 200. `MESSAGE_CHARS` stays 1,600, and the
  fragment filter stays as it is.
- The `memory_save` schema and the store error both name 200. The store remains
  the final gate; the schema is a hint.
- Existing rows are not rewritten, truncated, or deleted.

**Recall:** the token limit goes away. `memory.recall_max_items` is the only
bound, and it stays at 10.

- `MemoryConfig.recall_max_tokens` and the `memory.recall_max_tokens` setting are
  removed, with their defaults, validation, and example entry.
- `MemoryService::recall_within` is removed; `recall` and `recall_at` are the
  only entry points and take no budget.
- `digest::coverage_cost` and `digest::line_cost` are removed if nothing else
  uses them, and `rank::fill` loses the token argument. With no budget to spend,
  `score / cost` has nothing to trade against, so the fill orders by score and
  keeps the best `recall_max_items`. Selection must not depend on statement
  length any more.
- The agent loses the shrink loop and `digest_floor`. When the prompt is over
  `budget.usable`, the levers are `shrink_tool_results` and compaction, which it
  already has.
- The status report drops `max_tokens`; `max_items` stays. The Overview card
  renders `10 recall` without the multiplied pair.
- SPEC states the bound as a count: ten items, 200 characters each, at most two
  coverage lines of three subjects. The worst case is about 1,200 tokens and the
  ordinary one about 700, and both follow from the caps rather than from a
  second setting. Raising `recall_max_items` raises the ceiling; its clamp stays
  1 to 64.

## Why

Two settings described one bound. The token cap existed because a memory could
be 2,000 characters, so a count alone did not bound the block. At 150 and then
200 characters that is no longer true: ten items are at most about 1,200 tokens,
and the token cap only decided which rows lost when their sum crossed a number
that was chosen to approximate the item cap. At 200 characters, a 700-token cap
would mean ten items no longer fit and the digest silently shows nine, which is
the failure the item cap was supposed to make visible.

A count is also the bound the model and the operator can reason about. "Ten
memories of 200 characters" is a shape. "700 tokens of an estimated line cost
that charges a quarter of the subject length" is not.

Removing the cap also removes the reserve arithmetic (`coverage_cost`), the
post-render trim loop, and the agent's recall shrink. `rank::fill` stops
penalising a long decisive statement, which is the point of raising the content
limit in the first place.

## Scope

- Do not change the subject limit (120), the kind list, pinning rules, the
  800-token rules block, or the extractor's 1,600-character message guard.
- Do not add a migration. No row is rewritten or lost to enforce a write limit.
- Do not change the retrieval channels, the fusion, the score, or the diversity
  pass. The only ranking change is that cost no longer divides the score.
- Do not add a replacement token cap elsewhere, and do not keep a hidden floor in
  the agent.
- Keep the coverage lines. They are bounded by `COVERAGE_SUBJECTS` and the
  subject limit, and they are what makes the digest honest about what it left
  out.

## Test

- A 200-character memory is accepted; a 201-character one is refused with
  `invalid_input`, the actual length, and `200`. A pinned rule uses the same
  boundary and the same error, and a refused write leaves the store unchanged.
- A 200-character memory renders whole; an older row above the limit is clipped
  at 200 with an ellipsis, and only for display.
- The extractor never proposes content above 200, and a long user message is
  still scanned.
- The schema description and the store error describe the same limit.
- The digest of ten maximum-length rows with two maximum-length coverage lines
  stays at or below the bound SPEC states, and the same digest with the ordinary
  coverage line stays near 700. Measure the worst case; do not assume it.
- A recall of eleven candidates returns ten, and the ten are the highest scores.
  A long statement is not displaced by a shorter one at a lower score.
- No settings file, status payload, or settings-form row mentions
  `recall_max_tokens`. A settings file that still sets it is refused as an
  unknown field, and the message names `memory.recall_max_tokens` so an
  installed home can delete the line.
- The agent assembles a prompt over `budget.usable` without re-running recall,
  and the existing tool-result shrink and compaction still apply.
- The memory benchmark still reports automatic recall at or above 0.92,
  one-call reachability at or above 0.98, and digest p95 at or below 300 tokens
  after the ordering change.

## Documents

- SPEC 7.2 (content row), 7.3 (write rules), 7.4 (recall bound and the ten-row
  claim), 7.6 (rules), 6.1 and Appendix A where the recall budget is named, the
  settings table row for `memory.recall_max_tokens`, the summary table row
  "Recall budget", and the Draft bump.
- DECISIONS: a new entry for the 200-character limit and count-only recall. Add a
  superseded note to the 2026-11-16 entry, in the form that entry already uses
  for the 2,000-character cap, covering both the 150 limit and the 700-token
  budget.
- `docs/SPEC.md` line notes that say "Recall now defaults to 700 tokens".
- The solved 150-character issue keeps its Resolution; this file references it
  and does not reopen it.

## Not established

- Whether the benchmark's automatic recall holds after `rank::fill` stops
  dividing by cost. A shorter row may have won a slot over a longer, higher
  scored one; the measurement decides, and the issue requires the gate, not a
  specific number.
- Whether an operator with a small context window relied on the recall shrink
  loop. The item cap is the replacement: lower `recall_max_items` rather than
  expect the agent to shrink a rendered digest.
- Whether any real store holds statements between 151 and 200 characters. The
  repository has no production store and the fixture does not reach the limit,
  so the demand comes from the write path, not from data.

## Resolution

`store::CONTENT_LIMIT`, `digest::LINE_CHARS`, and `extract::MAX_CHARS` are 200.
`MemoryConfig` has no `recall_max_tokens`, and `memory.recall_max_tokens` is
removed from settings, validation, the settings form, and the status report. A
settings file that still sets it is refused, and the schema error names
`memory.recall_max_tokens`. `MemoryService::recall` and `recall_at` take no
budget; `recall_within`, `digest::line_cost`, `digest::coverage_cost`,
`digest::trim_coverage`, the agent's recall shrink loop, and `digest_floor` are
gone. `rank::fill` orders by score and keeps `recall_max_items`.

The bound is a count. The worst case (ten 200-character rows, a 120-character
subject on every row, and two coverage lines of three 120-character subjects)
measured 1,071 estimated tokens after the header render, and ten ordinary full
rows measured 611. The benchmark reports automatic recall 0.928 lexical and
0.954 hybrid, one-call reachability 0.990, and digest p95 97 on the final render.

Commits: `eb5f617` (test), `0b78e1a` (feat), `4d4a8b3` (docs).

Verified with the store and extractor boundary tests, `budget_settings`
`a_retired_recall_token_budget_is_refused_by_name`, the `s8_telegram_daemon`
status (no `max_tokens`), the Overview test (`8 recall`, no token pair), the
`s6_agent` test that a full 10-row recall is not shrunk for the context budget,
and `memory_reachability`. `./scripts/ci.sh` and both benchmark runs pass.
