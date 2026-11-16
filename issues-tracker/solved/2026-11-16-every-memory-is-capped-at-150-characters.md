# Every memory is capped at 150 characters

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-16 |
| Area | `hearth-memory` |

## Observed

`CONTENT_LIMIT` in `crates/hearth-memory/src/store.rs` is 2,000 characters, and
`validate` refuses a longer write. A pinned rule is different: `RULE_LIMIT` is
400, and `validate` checks it only when `NewMemory.pinned` is set. The two limits
overlap because the general check runs first.

Automatic recall is built by `digest::line` in
`crates/hearth-memory/src/digest.rs`. It clips the statement to `LINE_CHARS`, which
is 140, even though a memory may be 2,000 characters. The defaults in
`MemoryConfig` are 300 recall tokens and 12 items.

The deterministic extractor in `crates/hearth-memory/src/extract.rs` accepts
candidates up to `MAX_CHARS` 400 and uses `MAX_CHARS * 4` as the message-length
guard.

The 200-statement benchmark fixture in `crates/hearth-eval/fixtures/memory_bench.json`
has a median statement length of 30 characters and a maximum of 58, so the
current 2,000-character cap is far above the shape a memory actually takes.

## Wanted

One content limit for every stored memory, pinned rules included:

- **150 characters**, counted after trimming, hard limit.
- Exactly 150 is accepted; 151 is refused.
- No truncation, no partial save, and no per-rule exception.
- A pinned rule is a memory with `kind = rule` and `pinned = 1`; it uses the same
  150-character limit as a fact or a preference.

The rules block stays a separate aggregate budget:

- It remains **800 tokens** by default (`memory.rules_max_tokens`).
- A 150-character rule still consumes its share of that block.
- A write is refused if the rendered block would overflow, just as it is now.
- The per-rule 400-character cap goes away.

Recall changes to match the smaller statement:

- `LINE_CHARS` becomes **150**, so a newly written memory is shown in full.
- Automatic recall keeps at most **10** items (`memory.recall_max_items`).
- `memory.recall_max_tokens` defaults to **700**.
- Ten is an upper bound. Long subjects and coverage lines can still reduce the
  count; 700 fits ten typical 150-character rows but is not a reservation.

## Why

The current 2,000-character cap lets one row be a note or a document rather than
one remembered statement. That fights the retrieval design: recall clips the row
to 140 characters, so most of a long row is never seen unless the model makes a
second call, and a 2,000-character row can exceed the embedding model's 512-token
input. A 150-character cap makes the stored memory the same thing the recall line
shows.

Removing the separate rule limit keeps one definition. A pinned rule is not a
different size of memory; it is a memory that is injected every turn. Its real
budget is the 800-token rules block, not a second per-row number. The 800-token
block still protects the prompt from too many or too-large rules.

The 150 limit is generous for the data the system actually stores: the benchmark's
longest statement is 58 characters. It forces an over-long extraction or model
write to be split into separate statements, which is what retrieval and the
digest are built to handle.

## Strict write behavior

The store is the final gate. No write path may silently truncate or pre-filter a
long memory, and a refused write returns an invalid-input error (`invalid_input`
through the tool) and saves nothing.

- The error names the actual length and the limit, for example:
  `memory content is 168 characters; the limit is 150. Split it into separate memories.`
- The tool schema for `memory_save` names the 150-character limit so the model
  sees it before the call. The store check remains authoritative because imports,
  extraction, and direct service writes do not pass through the schema.
- The extractor must not produce over-limit candidates. Lowering `MAX_CHARS` to
  150 must not also lower the message-length guard: use a separate message limit
  so a long user message is still scanned.
- The legacy standing-file import pre-filters bullets through `RULE_LIMIT`, so an
  over-limit bullet disappears before the store can report it. It must use the one
  content limit and let a refused bullet count as skipped rather than vanish.

Existing rows above 150 are not rewritten or deleted. The strict check applies to
new writes; recall clips those older rows at 150 for display, and the rules render
needs a read-side guard so an old rule is not cut without a trace.

## Scope

- Do not change the subject cap, the kind list, pinning rules, or the 800-token
  rules block.
- Do not add a migration that truncates existing content. A memory is not lost to
  enforce a new write limit.
- Keep the whole new memory in the recall line. The only reason to clip is an old
  row that predates the 150 cap.
- Do not change the rules block's aggregate budget to 150; that number is a
  per-statement cap, not a rules-block cap.

## Test

Boundary and strictness:

- A 150-character ordinary memory is accepted; a 151-character one is refused.
- A 150-character pinned rule is accepted; a 151-character one is refused with
  the same content error, not a separate rule error.
- A refused write leaves the store unchanged and returns an `invalid_input` error
  whose message contains the actual length and `150`.
- The tool schema and the error describe the same limit.

Recall:

- A stored 150-character memory is rendered in full, not clipped at 140.
- Ten 150-character memories fit within a 700-token budget in the ordinary case.
- A digest with a long subject or coverage still stops at its token budget and
  shows fewer than ten rather than exceeding it.
- An older row above 150 is present in the store and is clipped only for display.

Extraction and rules:

- The extractor never proposes content above 150, and a long user message is
  still scanned for shorter candidates.
- The rules block still refuses a write that would exceed 800 tokens and still
  renders the existing rules in creation order.
- `RULE_LIMIT` no longer enforces a separate 400-character rule. `RULE_CHARS` may
  stay 400 as a read-side guard for older rules, but it must not clip a new rule
  below 150 and it is not the new-write check.

## Documents

- Update SPEC 7.2's content row, 7.3's write rules, 7.4's recall defaults, 7.6's
  rules rules, 6.1's recall budget, and Appendix A's recall row.
- Update the DECISIONS entries that describe the 2,000-character write cap, the
  400-character rule cap, and the 300-token/12-item recall defaults. Add one entry
  recording the 150-character cap and why the rule exception was removed.
- Bump the Draft and state the change in the header note.
- If the open issue to remove the warn-only prompt budgets is not taken first,
  `SEGMENT_RECALL` must move to 700; otherwise the 700-token recall will always
  report itself over the old 300-token warning.

## Not established

Whether ten full memories must be guaranteed when every subject is at the
120-character maximum. At 700 tokens that case can show fewer than ten. The
alternative is a larger default, but the token budget is a maximum and the
current benchmark's digest p95 is far below 700.

## Resolution

`store::CONTENT_LIMIT` is now 150 characters after trimming and is the only
new-write limit; `RULE_LIMIT` is gone. A refused write returns `invalid_input`
with the actual length and the limit and saves nothing. The `memory_save` schema
names the same limit. `digest::LINE_CHARS` is 150, so a new memory is shown whole;
`RULE_CHARS` stays 400 only as the read-side guard for an older rule.

The extractor caps a candidate at 150 with a separate 1,600-character message
guard. The legacy import no longer pre-filters bullets: a refused bullet counts
as skipped and keeps the old files. Recall defaults are 700 tokens and 10 items in
both `MemoryConfig` and the settings defaults.

Commits: `577a400` (test), `338036f` (feat), `7f1a273` (docs).

Verified by the boundary tests in `crates/hearth-memory/src/store.rs`, the line
tests in `digest.rs`, the extractor tests in `extract.rs`, the skipped-bullet test
in `legacy.rs`, the schema and error tests in `tools.rs`, the ten-row digest test
and defaults test in `lib.rs`, the config default test, and the daemon status
test. The memory benchmark still reports automatic recall 0.933, one-call
reachability 0.990, and digest tokens p95 139 against the 300-token gate.
`./scripts/ci.sh` passes.
