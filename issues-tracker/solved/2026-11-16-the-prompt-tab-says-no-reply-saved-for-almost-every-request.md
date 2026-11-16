# The Prompt tab says "no reply saved" for almost every request

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-16 |
| Area | `hearth-cli` |

## Current

Every request whose exchange has no matching assistant entry prints a faint
`  no reply saved` line, pushed by `derive` in
`crates/hearth-cli/src/ui/prompt.rs` as `Notice::NoReply` whenever `response` is
null, and rendered by the `notice` method.

That null is not a failure signal. `prompt_exchanges` in
`crates/hearth-runtime/src/inspect.rs` builds the response by joining
`prompt_blobs` to `entries` on the prompt hash: `LEFT JOIN entries e ON
e.prompt_hash = b.hash`.

A failed turn never produces an entry with that hash. `Agent::settle_failure` in
`crates/hearth-runtime/src/agent.rs` saves a `notice` entry through
`Save::new(...).entry("notice", ...)` with no prompt hash, and the reason it
records lives on the submission, passed to `settle_with_error` as
`{"reason": reason}`, which this query does not read. The prompt is also saved
before the provider call, so the newest exchange is unanswered for the whole
turn.

The result: the line appears while a turn is running, after a provider error,
after a budget failure, after a cancellation, after a crash, and after an answer
that was empty. One string covers five different situations, and the common case
is the least interesting one — a request that is simply still working.

## Desired

The notice appears only when an exchange really has no answer and the reason is
knowable: a provider error, an empty answer, a stop or cancel, or a crash. A
request that is still running shows no notice. An answered request shows none.

Where a notice is shown it should carry its cause. The reasons already exist in
the record — `"budget"` and `"cancelled"` are both written by `Agent::run` and
its cancellation path — so the view can name what happened instead of saying
nothing was saved.

## Constraint

The Prompt tab reads through a read-only connection with no access to the
daemon's in-memory state (`prompt_exchanges` opens `open_read_only`), so "still
running" has to be derivable from what is stored.

## Documented elsewhere

The `ui` tests assert today, in `a request with no answer says so`, that an
unanswered request says so.

## Verification

Open the Prompt tab during a turn and confirm no notice is shown for the request
in flight. Then fail a turn with a provider error and one with a budget error
and confirm each shows a notice that names its cause.

## Resolution

A saved request now has one of three stored outcomes. An assistant entry answers
it. A notice entry settles it without an answer: `Agent::settle_failure` and
`Agent::settle_cancelled` save the notice through `entry_with` with the in-flight
`prompt_hash` and a machine `reason` in its content, and
`Session::recover_interrupted` does the same for a poisoned request from the failed
task's checkpoint. A request with no entry is still in flight.

`inspect::prompt_exchanges` reads `entries.kind` and returns a `response`, a
`notice`, or neither; an assistant entry wins over a notice for the same hash. The
Prompt tab raises `Notice::NoReply` only from a notice, and `no_answer` maps the
reason to `provider error`, `the model returned no answer`, `over the context
budget`, `stopped`, or `interrupted by a restart`.

The budget failure now saves the prompt it never sent, so the audit has the
request the notice belongs to. The notice text stays in the conversation entry and
still reaches the next request's transcript.

Commits: `8eb4484` and `9b425f8` (tests), `af3773a` (feat), `db7efbe` (docs).

Verified by `prompt_exchanges_settle_each_request_with_its_outcome` in
`crates/hearth-runtime/tests/inspect.rs`, by
`each_no_answer_reason_reads_as_its_cause` and
`a_request_with_no_answer_and_no_reason_says_nothing` in
`crates/hearth-cli/src/ui/prompt.rs`, by the extended
`a_provider_error_settles_unanswered_with_a_notice` and
`an_oversized_prompt_settles_with_a_budget_notice` in
`crates/hearth-runtime/tests/s6_agent.rs`, and by
`a_second_interruption_is_reported_to_the_person` in
`crates/hearth-core/tests/s1_tasks.rs`. The old `a request with no answer says so`
assertion in the UI tests was replaced, as the issue anticipated.
`./scripts/ci.sh` passes.
