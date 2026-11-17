# Every turn reads the whole transcript before applying compaction

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-core` |

## Observed

`Session::history_for_prompt` calls `Session::history` without a sequence
filter. `history` selects every entry in the conversation, decodes every
`content_json`, and returns the full list. `history_for_prompt` then finds the
newest summary or reset and discards every row before its `first_kept`.

Every agent turn calls `history_for_prompt` at least twice, once for the prompt
and once for the compaction plan, and `Agent::compact_soft`, `try_compact`, and
`idle_compaction_candidate` call it again. Summaries are designed to let the
record grow, but the read path still pays for the entire record. A conversation
with 100,000 covered entries reads and parses all of them to show ten visible
entries.

`Session::history` is also used by skill review for every settled submission, so
the same cost can appear in the nightly review.

## Verification

Create a conversation with one summary entry that has `first_kept` near the end
and 50,000 older entries, then call `history_for_prompt`. It reads all 50,001
rows. A query bounded by the summary's `first_kept` plus the marker row reads the
same visible result with one indexed range scan. The existing compaction tests
use a handful of entries and do not expose the difference.

## Resolution

`Session::history_for_prompt` first reads the newest summary or reset marker,
then reads only the entries after it. A summary query starts at `first_kept`;
a reset query starts after the reset. The covered range is no longer decoded.
An uncompacted conversation still reads its whole history, which is the visible
record.

Commit: `c197417`. Verified by
`a_prompt_history_skips_the_covered_range_without_decoding_it`, which plants a
covered row whose `content_json` is not JSON: the old full read failed on it, and
the bounded read returns the summary and the kept tail. The test was added with
the fix and passes on its first run.
