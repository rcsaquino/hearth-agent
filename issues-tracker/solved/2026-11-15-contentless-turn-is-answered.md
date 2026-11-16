# A turn with no user content is answered, and the runtime note cannot say so

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Build | 797291a |
| Area | `hearth-runtime`, `hearth-core` |

## Observed

A turn arrived carrying no message. The runtime note was the only content in the
request, and it contains one line — the current time.

The turn was answered. The reply acknowledged the timestamp as though it were
the request, then continued with an unrequested status recap. Asked what had
triggered it, the agent attributed the reply to the clock.

Observed three times on 2026-11-15: 17:37, and twice more before 17:51.

## Mechanism

`crates/hearth-runtime/src/agent.rs`, `runtime_note()`. It returns exactly one
line:

```
Current time: 2026-11-15T17:37:48+08:00.
```

A test fixes this shape — `crates/hearth-runtime/tests/s6_agent.rs`,
`the_runtime_note_renders_the_configured_zone_and_only_the_time`. It asserts the
note is a single line and carries no conversation or model identifier. The note
is therefore accurate about the clock and silent about everything else. It has
no field in which to record that a turn lacked a request.

`crates/hearth-core/src/prompt.rs`, `build()`. The note is appended after the
conversation history, as the final message of the request. When the history
ends in an empty user message, the note is the only content the turn carries,
and it is addressed to the model.

Three conditions allow the empty message to reach the request.

- `crates/hearth-core/src/session.rs`, `submit()`. Checks only for a repeated
  request id. The content value is stored without inspection, so nothing
  rejects a submission whose text is empty or absent.
- `crates/hearth-core/src/prompt.rs`, `parse_blocks()`. A value that is neither
  a string nor a recognised block yields no content blocks.
- `crates/hearth-core/src/prompt.rs`, `message_for_entry()` and
  `paired_messages()`. An entry becomes a message whether or not its block list
  is empty, and the pairing step removes tool messages only. An empty user
  message survives into the request.

A guard exists but does not cover this. `latest_user_text()` in
`crates/hearth-runtime/src/agent.rs` returns `None` for blank text, and that
decides only whether memory recall is added. It does not prevent the turn and
does not suppress the runtime note.

## Not established

How the contentless turns originated. Candidate paths, none traced to the
observed event:

- `crates/hearth-runtime/src/ipc.rs`, `handle_connection()`. Rejects a blank
  protocol line and returns without logging, which would produce exactly this
  symptom and leave no record. If this is the source, the empty turn is a
  malformed or prematurely closed request rather than a legitimately empty one.
- The Telegram submission path and the scheduled reminder path were not
  examined.

Whether a turn should be refused at ingestion or tolerated and answered with
less context is a design question the fixer has to settle.

## Consequence

A turn with nothing in it is answered rather than refused, and the answer is
anchored to whichever runtime note the turn happens to carry. For this note that
means a timestamp, and an answer that treats the clock as the request.

The failure is silent in both directions: nothing logs the empty submission, and
the note itself is shaped by a test that asserts it must stay minimal, so adding
a marker to the note is not available without changing that test.

## Verification of a fix

A fix should show that a submission with no usable content either does not
reach the model or reaches it with the emptiness made explicit. A test should
submit empty content and assert on the outcome. The existing note-shape test
constrains any solution that expands the note.

## Resolution

Fixed in `f8b2513`. The failing tests are in `b572811`.

**What changed.**

- `crates/hearth-core/src/prompt.rs` gains `carries_request`, the one definition
  of what asks for something: a block list with non-blank text, an image, a file, a
  tool call, or a tool result. Whitespace and thinking alone ask nothing.
- `crates/hearth-core/src/session.rs` gains `Session::require_request`, called by
  both doors that create a submission, `submit` and `submit_telegram`. Content that
  carries no request is refused with `CoreError::Invalid("the message was empty")`
  before anything is stored or written.
- The refusal in `submit` sits after the `request_id` dedupe read, so a repeated
  request id still returns the original and never starts a second run.
- `crates/hearth-core/src/prompt.rs`, `message_for_entry`. A stored entry that
  carries no request is left out of `paired_messages` and the request. This covers
  the rows the fault already wrote: each answered empty turn left an empty user
  entry, and every later turn in that conversation re-sent it to the provider.

**Mechanism confirmed at runtime.** Against a scripted provider, submitting `{}`
produced a request whose last two messages were `User blocks=[]` and
`System(Text("Current time: ..."))`. Submitting `[{"type":"text","text":"  "}]`
produced `User blocks=[Text("  ")]` before the same note. Both reached the
provider.

**What the report left open.**

- The origin of the contentless turn is still not established, and the fix does not
  depend on it. `Session::submit` and `Session::submit_telegram` are the only
  producers of `user` entries, and both now refuse content that asks nothing.
- The IPC candidate is ruled out. `crates/hearth-runtime/src/ipc.rs` serves `status`
  and `watch` only and cannot create a submission, so a blank protocol line cannot
  become a turn.
- The helper path was a live producer: `run_helper` passed `task.input["brief"]`
  through without an emptiness check. A blank brief now fails the task with
  `invalid input: the message was empty` instead of answering the clock. No test
  was added for it; the arm that handles the error already exists and is shared
  with a storage failure.
- The Telegram poller still refuses an update with nothing to answer before it
  reaches `submit_telegram`, because it owns the offset decision. That pre-check
  makes the record refusal unreachable from Telegram today, so no replay loop
  exists.
- A marker in the runtime note was rejected. The note stays one clock line, which
  keeps `the_runtime_note_renders_the_configured_zone_and_only_the_time` true.

**Verification.**

- `cargo test -p hearth-core --test s1_session a_submission_with_no_message_is_refused`
  failed before the change at the `expect_err` line, because both shapes were
  accepted and stored. It asserts the refusal, that neither request id left a
  submission behind, that history holds only the real message, and that a repeated
  request id with empty content still returns the original.
- `cargo test -p hearth-core --test s1_prompt a_stored_entry_with_no_request_is_left_out_of_the_prompt`
  failed before with `user messages were ["hello", "", "   "]` and passes after with
  `["hello"]`.
- `cargo test -p hearth-runtime --test s6_agent a_submission_with_no_user_content_is_refused`
  failed before at the refusal, and its provider count shows a real message calls
  the provider while neither contentless shape adds a call. With the source guard
  stashed it fails again, so the guard is what makes it pass.
- `./scripts/ci.sh` passes at `f8b2513`. The acceptance phases past the stale
  `tokens_bench` line (crash, scheduler, serialized Telegram daemon, daemon
  lifecycle, CLI lib, memory benchmark, soak, secret scan) pass when run directly.
  The stale line is a separate fault, filed as
  `open/2026-11-15-acceptance-names-a-removed-test-target.md`.
- `docs/SPEC.md` is Draft 0.27 and `docs/DECISIONS.md` records the choice.
