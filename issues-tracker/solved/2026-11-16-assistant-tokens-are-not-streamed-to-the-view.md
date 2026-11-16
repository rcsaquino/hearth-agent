# Assistant tokens are not streamed to the view

| | |
| --- | --- |
| Kind | feature |
| Reported | 2026-11-16 |
| Area | hearth-runtime |

## Observed

The Prompt tab shows a turn only after it has finished. While the model is
generating, the view shows the working indicator and nothing else. The text
appears in one step when the request completes and the saved prompt is reloaded.

The model's deltas do reach the runtime. They stop there.

## Mechanism

The stream loop in `crates/hearth-runtime/src/agent.rs:604` collects deltas into
local buffers and publishes nothing:

```rust
let mut text = String::new();
let mut thinking = String::new();
let mut calls = Vec::new();
let mut usage = Usage::default();
for event in stream {
    match event {
        Ok(StreamEvent::TextDelta(delta)) => {
            text.push_str(&delta);
            hearth_storage::crash_if("mid_stream");
        }
        Ok(StreamEvent::ThinkingDelta(delta)) => {
            thinking.push_str(&delta);
            hearth_storage::crash_if("mid_stream");
        }
        Ok(StreamEvent::ToolCall(call)) => calls.push(call),
        Ok(StreamEvent::Usage(value)) => usage = value,
        Ok(StreamEvent::Done { .. }) => {}
```

Publishing happens through `emit` (`agent.rs:1206`), which constructs a
`RunEvent` with a `&'static str` kind (`crates/hearth-runtime/src/events.rs:11`).
The kinds published during a run are `run_started`, `user_message`, `task_step`,
`turn_started`, `tool_call_started`, `tool_result`, `assistant_reply`,
`usage`, and `run_settled`. There is no delta kind, and no `emit` call exists
inside the stream loop.

`LiveEvents::publish` (`events.rs:57`) serialises one event and fans it to
subscribers held in an `mpsc` list. A CLI worker subscribes and forwards each
event as `AppEvent::Live` (`crates/hearth-cli/src/workers.rs:126`).
`App::apply_event` (`app.rs:1128`) pushes it onto the activity feed and, for
`assistant_reply` and `run_settled` only, reloads the Prompt view
(`app.rs:1129`).

The saved prompt is written once per finished request, so it cannot carry
partial text.

## Constraints

Publishing a delta per token would put one IPC write and one channel send on the
model's hot path. The stream loop already calls `hearth_storage::crash_if`
per delta.

`apply_event` treats every live event as an activity-feed row and appends it to
`self.events`, trimmed to `MAX_EVENTS` (`app.rs:1137`). A delta published as an
ordinary live event would fill the activity feed with partial text.

The transcript on the Prompt tab is built from saved exchanges and cached under
`CacheKey` (`prompt.rs:181`). Streaming text is neither saved nor part of that
key, so a live region has to be drawn separately from the cached document.

## Resolution

The runtime now publishes `text_delta` and `thinking_delta` events. A `Deltas`
buffer holds them and publishes at most every 100 ms or 512 bytes, flushes at
stream end, and flushes the other kind first when the provider switches from
thinking to text. A 1,000-delta reply produces two events, not 1,000, so the
model's hot path does not pay a socket write per token.

The CLI appends chunks to `PromptView::live` and does not push them into the
activity feed. `turn_started` starts a fresh live turn; `run_settled` clears it.
The live turn is painted after the cached document, thinking first and then text,
under the same role bars, with the spinner on the first live label. Because it is
not part of the cache, streaming never invalidates the document.

The refresh trigger moved from `assistant_reply` to `task_step` with phase
`request`, which is emitted after the prompt blob is written, plus `run_settled`.
That makes the saved document the thing that decides when a reload happens; a
finished reply no longer races its own entry.

Commits: `0c16256` (runtime test), `1d0b46c` (runtime), `e0da201` (CLI test),
`da99603` (CLI). The runtime tests failed with no delta kind on the old loop; the
CLI tests failed because deltas were fed to the activity feed and no live turn was
drawn. Verified by `cargo test -p hearth-runtime --test s6_agent`,
`cargo test -p hearth-cli --lib`, and `./scripts/ci.sh`. The live region was
verified through rendered cells at 80, 100, and 120 columns, including thinking
before text, a new turn dropping the old text, and `run_settled` clearing it; the
tmux capture verified the surrounding saved transcript and the indicator
placement in the real terminal.
