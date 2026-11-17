# Capture reads the newest user entry instead of the run's entry

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-runtime` |

## Observed

`extraction::capture_conversation` reads `Session::history` for the conversation
and takes the last entry whose kind is `user`. The daemon calls it after the run
has delivered its answer.

If a queued user message arrives before capture runs, capture reads that newer
entry instead of the entry the finished run answered. The newer message is
extracted before its own turn, and its own turn extracts it again later. Exact
dedup may hide a repeated candidate, but a different surface form can create a
second row.

`extraction::capture` already accepts an entry id and text. The problem is that
`capture_conversation` chooses the entry by position in the history rather than
by run identity.

## Wanted

Capture uses the user entry that belongs to the finished run. A queued message
is extracted only by its own run.

## Scope

- Do not change the extractor patterns.
- Do not change the order of the conversation history.
- Keep a capture failure non-fatal to the answer.
- Keep the existing replay behavior: capturing the same entry twice must not
  create a duplicate.

## Test

- Queue a second user message before the first run's capture. Assert that only
  the first run's entry is captured.
- Run the second turn. Assert that its entry is captured then.
- Capture the same run entry twice. Assert that the second pass writes nothing.
- Restart between the run and capture. Assert that the entry id still resolves.

## Not established

- Whether the daemon should pass the entry id from the run outcome or from the
  submission. The chosen id must be stable across a restart.
- Whether the current exact-content dedup fully hides the fault. A different
  surface form can still duplicate, so the fault remains.


## Resolution

The run now records which user entry each submission answered. Migration 0012
adds `submissions.user_entry_id`; `Session::save` links a submission to the user
entry in the same save, and the backfill links an existing row by conversation,
content, and creation instant. `Session::entry` reads one entry by id.

`RunOutcome.user_entry_ids` carries the submission's entry plus every absorbed
submission's entry. `extraction::capture_entry` captures exactly those entries;
`capture_conversation` is gone. The daemon's memory tool source also uses the
submission entry, so a tool write points at the message the run answered. A
replay still writes nothing because the store sees the earlier row.

Commits: `4d1d9bb` (test), `ce63c35` (fix).

Verified by `a_submission_names_the_user_entry_it_answers` in
`crates/hearth-core/tests/s1_session.rs` and by
`a_later_message_is_not_captured_by_an_earlier_entry`,
`an_explicit_remember_is_written_once`, and
`a_captured_entry_still_resolves_after_a_restart` in
`crates/hearth-runtime/tests/extraction.rs`. The restart test reopens the home
between the save and the capture. `./scripts/ci.sh` passes.
