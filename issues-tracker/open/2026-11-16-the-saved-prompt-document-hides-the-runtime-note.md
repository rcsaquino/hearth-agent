# The saved prompt document does not show the runtime note where it is sent

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-16 |
| Area | hearth-core |

## Observed

`build_prompt` produces two things from one call: the request that goes to the
provider, and an audit document that is stored in `prompt_blobs`. The two do not
agree about the runtime note.

The request carries the note inside the newest `User` or `Tool` message. The
stored document carries a transcript with no note in it, plus a sibling
`"runtime"` key holding the note on its own.

```json
{
  "transcript": [ ... no note in any message ... ],
  "runtime": "<runtime>Current time: 2026-11-16T11:55:16+08:00.</runtime>"
}
```

Anyone reading the saved document therefore sees a request that was never sent.

## Mechanism

`crates/hearth-core/src/prompt.rs:85` attaches the note to the request:

```rust
attach_runtime_note(&mut messages, &input.runtime_note);
```

`attach_runtime_note` (`prompt.rs:192`) finds the newest `User` or `Tool` message
in `messages` and appends the note to it — into the `ToolResult` block's `content`
for a tool message, as a new `Text` block for a user message.

The audit document is then built from a different collection,
`history_messages` (`prompt.rs:87`):

```rust
let transcript: Vec<Value> = history_messages
    .iter()
    .map(|message| { json!({ "role": role_str(message.role), "blocks": ... }) })
    .collect();
```

`history_messages` is the output of `paired_messages(history)` and is never
passed to `attach_runtime_note`. The note reaches the request only. The document
keeps the unmodified transcript and records the note separately at
`prompt.rs:149`:

```rust
"runtime": input.runtime_note,
```

So the document is a description of the inputs, not of the request. The `runtime`
key is the note's value, positioned as a peer of `identity`, `tools`, and
`rules` — which reads as a prompt segment rather than as text living inside
another message.

## Why it matters

The Prompt tab exists to show what was sent. `prompt_exchanges`
(`crates/hearth-runtime/src/inspect.rs:49`) reads these blobs and returns them to
the TUI, and the module comment for the view
(`crates/hearth-cli/src/ui/prompt.rs:1`) states it shows "everything sent to the
model and everything back". For the runtime note that is not true.

The failure is not cosmetic. While investigating a report that some turns carried
only the clock, this document was read as evidence, and it showed no note attached
to any message — which pointed at the wrong conclusion. A reader with only the
repository and the database would repeat the mistake.

There is a second, smaller mismatch. `tokens.transcript`
(`prompt.rs:120`) is computed from `transcript_text`, which is the unmodified
transcript, and `tokens.runtime` from `input.runtime_note`. The two are counted
separately and correctly for the total, but a reader adding up the message-level
cost of the document cannot arrive at the `transcript` figure, because the note
it was sent with is not in the messages it can see.

## What has to stay true

The note must remain outside the cached prefix. It is rebuilt every turn
(`crates/hearth-runtime/src/agent.rs:270`), and moving it into `identity`, `tools`,
`rules`, `skills`, or `guidance` would put a changing value in
`prefix_parts` (`prompt.rs:125`) and defeat the cache on every request.

The note must also stay out of stored conversation entries. It is a prompt-time
value only.

## Not established

Whether the audit document should show the exact wire messages, or keep its
current segment-shaped layout and gain a field that says where the note was
attached. The first is truer; the second is cheaper and keeps the existing
rendering. That is a design decision, and the observable requirement is only that
a reader can tell, from the stored document, which message carried the note and
that it was not a message of its own.

## Test

`crates/hearth-core/tests/s1_prompt.rs` builds prompts directly and has sixteen
references to `runtime_note`. The requirement to pin: after `build_prompt`, the
saved document and `prompt.request.messages` agree about the note — the same
message carries it, and the document has no separate note field that suggests
otherwise.
