# A provider stream that never finishes is saved as a complete answer

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-model`, `hearth-runtime` |

## Observed

The stream adapter treats end of body as the end of the answer. It never checks
that a terminal chunk arrived, and the agent ignores the finish reason it does
receive.

`crates/hearth-model/src/openai.rs:503`, `Iterator::next`:

```rust
Ok(0) => {
    self.done = true;
    let events = self.finish();
    self.pending.extend(events);
}
```

`finish()` always emits a terminal event, even when no `[DONE]` sentinel and no
`finish_reason` was seen:

```rust
events.push(Ok(StreamEvent::Done {
    finish_reason: self.finish_reason.take(),
}));
```

The agent discards it. `crates/hearth-runtime/src/agent.rs:616`:

```rust
Ok(StreamEvent::Done { .. }) => {}
```

Then the run saves the accumulated text as the assistant entry and settles the
submission `done` (`agent.rs:693`).

Three conditions reach that path:

- A proxy or provider closes the SSE body cleanly between frames. Nothing
  distinguishes that from a completed stream.
- A provider stops on its own output cap and sends `finish_reason: "length"`.
  The reason is parsed into `self.finish_reason` and never read.
- A provider returns `finish_reason: "content_filter"`. Same.

A fourth variant is an error object delivered as a normal event. The existing
fake server always appends `data: [DONE]`
(`crates/hearth-model/tests/s3_provider.rs:30`), so no test reaches this branch.

```text
data: {"error":{"message":"...","type":"..."}}
```

`handle_data` looks only at `choices` and returns when the array is absent, so
the error is dropped and the stream ends as an empty answer (or, if some text
arrived first, as a partial answer). The non-streaming JSON error path is caught
by the content-type check; a `text/event-stream` error is not.

## Consequence

A truncated answer is delivered as final, with no notice, and the conversation
records it as the assistant's reply. A turn that stopped on the provider's
output cap or a content policy is indistinguishable from a short answer. The
usage row for that turn records zero tokens, so the cost of the cut request is
invisible.

## Verification

Point the provider at a fake server that writes one text frame and closes the
body without `[DONE]`, then collect the stream. It yields `Done { finish_reason:
None }` rather than an error. Repeat with a frame carrying
`"finish_reason":"length"` completed by `[DONE]`, and with a 200
`text/event-stream` body of `data: {"error":{...}}`. All three are accepted.

## Resolution

The stream adapter tracks whether `[DONE]` arrived. End of body with no terminal
chunk and no finish reason is a protocol error; `length` and `content_filter`
finish reasons are errors that name the reason; an `{"error": ...}` SSE frame is
reported with the provider's message. A partial answer is no longer saved as a
complete one.

Commit: `a3f3140`. Verified by three tests in
`crates/hearth-model/tests/s3_provider.rs`.
