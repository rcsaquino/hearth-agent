# The runtime note is sent as a final system message and reads as a request

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-16 |
| Area | hearth-core |
| Build | 1d380d9 |

## Observed

Every provider request ends with a system message that holds one line:

```
Current time: 2026-11-16T00:52:59+08:00.
```

The line changes on every turn, because the clock moves. On a continuation turn
inside the agent loop, that line is the only new content in the request. The
model then answers it as though a person had asked for the time.

Observed on task `01a1e01b-8518`. At seq 2763 the assistant entry reads:

> The user hasn't said anything new — the turn shows "Current time:
> 2026-11-16T00:49:40+08:00."

Later in the same task, seq 2775 begins "Let me carefully piece together what I
found. The user said …", and the content it works from is the clock line.

## Mechanism

`Agent::runtime_note` (`crates/hearth-runtime/src/agent.rs:270`):

```rust
fn runtime_note(&self) -> String {
    let zone = Zone::resolve(&hearth_config::identity_timezone(&self.config.home));
    // The note is paid fresh on every turn, and the conversation id already
    // travels with the request.
    format!("Current time: {}.", zone.iso(self.session.now_micros()))
}
```

`build_prompt` pushes it after the transcript as a final message whose role is
`System` (`crates/hearth-core/src/prompt.rs:85`):

```rust
messages.push(ChatMessage {
    role: Role::System,
    blocks: vec![ContentBlock::Text(input.runtime_note.clone())],
});
```

Two properties of that placement cause the fault.

**It sits after the conversation.** Every other system layer — identity, rules,
tools, skills, guidance — is part of the cached prefix and comes before the
transcript. This one comes last, in the position where a turn's newest
instruction normally appears.

**Its role is `System`, not `User`.** A trailing system message is the strongest
position in the request. The model reads it as an instruction from the operator,
not as a note about the environment.

Combined with the loop in `agent.rs:461`, which has no turn cap (see
`open/2026-11-16-the-agent-loop-has-no-turn-cap.md`), each self-generated turn
adds a fresh instruction to the request. The timestamp is a valid instruction to
answer, so the model answers.

## Why `f8b2513` does not cover this

`solved/2026-11-15-contentless-turn-is-answered.md` fixed the contentless
submission. `Session::require_request` and `prompt::carries_request`
(`crates/hearth-core/src/prompt.rs:385`) now refuse a submission whose blocks ask
nothing:

```rust
pub fn carries_request(blocks: &[ContentBlock]) -> bool {
    blocks.iter().any(|block| match block {
        ContentBlock::Text(text) => !text.trim().is_empty(),
        ContentBlock::Thinking(_) => false,
        _ => true,
    })
}
```

That check runs on stored entry content at the submission door. The runtime note
is not entry content and never passes through that door. It is assembled inside
`build_prompt`, after the check has already been satisfied by the earlier
request. So a submission that asked for something — "Sure", "Go" — can produce
ten turns that ask for nothing.

Confirmed in the live store: 96 submissions, 96 user entries, zero blank
submissions. The gate works. The turns are not produced through it.

## What is deliberate and must stay

- The note is paid fresh every turn. The comment at `agent.rs:272` states this,
  and a stale clock is worse than none.
- The note carries no conversation or model identifier.
- The note is exactly one line.

`crates/hearth-runtime/tests/s6_agent.rs:1366`,
`the_runtime_note_renders_the_configured_zone_and_only_the_time`, pins all three:
it reads `requests[0].messages.last()`, requires one line, strips the
`Current time: ` prefix and the trailing `.`, and checks the zone offset.

## Requested

A self-generated turn must not be able to read the runtime note as a request.

The observable requirement: when the agent continues a task with no new user
content, the model must not be able to answer the clock, and the operator must be
able to tell from the transcript that no request was made on that turn.

Whether the note changes role, moves into the cached prefix, gains a line that
says what the turn is, or is paired with a continuation marker is a design
decision. Note that moving it into the prefix would defeat the cache, because its
text changes every turn — that is why it is paid fresh, and any design must keep
that cost.

## Not established

Whether a continuation turn should be sent to the provider at all when the only
change is the clock. The loop is not bounded by design — no turn cap is intended,
and `closed/2026-11-16-the-agent-loop-has-no-turn-cap.md` records that — so the
exposure is not narrowed from that side. Every continuation turn the model takes
carries the note, and each one is answerable as a request.

## Resolution

Fixed in `1fe8b5b`, with the failing tests in `4f4f5ae`.

**Root cause.** `build_prompt` appended the note as a standalone `System` message
after the transcript. That is the strongest instruction position in the request, so
a continuation turn whose only new system-level content was the clock read as a
fresh request. The content gate could not see it because the note is assembled
after the submission door.

**What changed.** The note no longer gets a message of its own.
`prompt::attach_runtime_note` appends it to the newest user or tool message: the
last tool result on a continuation, and the newest user message on the first turn
or after steering. The note is now
`<runtime>Current time: <ISO>.</runtime>`, still one line, still paid fresh every
turn, still with no conversation or model identifier. The tags keep it distinct
from the message that carries it.

`agent::runtime_note` produces the tagged line, so the runtime segment token count
includes what is sent. The Prompt tab labels the row `runtime note · newest input`
instead of `system · runtime note`, and the audit transcript still shows the stored
conversation, so an operator can see that a continuation added a tool result and no
user message. `docs/SPEC.md` 6.1 and `docs/DECISIONS.md` record the placement.

A prompt with neither a user nor a tool message has nowhere to carry the note, so
the note is dropped rather than put back in instruction position. Every submission
has a user entry, so this affects only a diagnostic prompt with no history.

**Verification.** The new tests fail on `4f4f5ae` and pass on `1fe8b5b`:

- `the_prompt_starts_with_identity_and_carries_runtime_on_the_newest_input`
- `the_runtime_note_rides_with_the_newest_tool_result`
- `a_continuation_turn_carries_the_clock_on_the_tool_result`
- `the_runtime_note_is_not_labelled_as_a_system_message`

The continuation test scripts one tool call and then a final answer, and on the
second request requires the last message to be the tool result with the clock on
it and no standalone system message containing the clock. `cargo test --workspace`
passes, and `cargo clippy --workspace --all-targets -- -D warnings` is clean.
`a_parent_spawns_a_helper_through_the_daemon` now matches the helper's user message
by prefix, because the clock is appended to that message.

**Not changed.** A continuation turn is still sent when the only new content is
tool output. The loop remains unbounded by design, as
`closed/2026-11-16-the-agent-loop-has-no-turn-cap.md` records.
