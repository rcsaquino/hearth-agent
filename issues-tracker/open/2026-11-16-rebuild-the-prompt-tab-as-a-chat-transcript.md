# The Prompt tab is rebuilt as a chat transcript view

| | |
| --- | --- |
| Kind | feature |
| Reported | 2026-11-16 |
| Area | hearth-cli |

## Wanted

Replace the Prompt tab with a view that reads like a chat window, in the manner of
Pi, Claude Code, Codex, and OpenCode. Two differences from those tools:

- **There is no input field.** The tab is view-only. This is a record of what the
  harness sent and received, not a place to type.
- **Every message is shown, including the system layers.** The tab is the full
  record of what reached the model, not a conversation with the scaffolding
  hidden.

Add a sign that the agent is working while a turn is in flight, so a long tool
loop does not look like a hung terminal.

The current view is not good enough and is not to be incrementally adjusted.

## What exists today

`crates/hearth-cli/src/ui/prompt.rs`, 1282 lines. The module comment states the
intent:

> Live session export: everything sent to the model and everything back.
> One scrollable document.

The shape is a flat document of labelled rows, not a transcript:

- `push_header` (`prompt.rs:352`) renders four header rows — session, live or
  paused state, prompt and prefix hashes, token totals, cache peers, and a legend.
- `build_rows` (`prompt.rs:544`) emits one `PromptRow` per request segment
  (`system · identity`, `system · guidance`, `system · rules`, tools, skills,
  guidance, recall, transcript entries, then response blocks).
- `PromptRow` (`prompt.rs:472`) carries `label`, `label_style`, `content`, and an
  optional `meta` string. Every row renders as `label` in a left column and
  wrapped text in a body column, indented by `BODY_INDENT`.
- `build_cache` (`prompt.rs:226`) caches the rendered lines keyed by exchange,
  width, revision, and raw flag. Rows are pre-wrapped, so this cannot be
  restructured without touching the cache key.

Navigation is `[` and `]` between saved exchanges, `f` to follow, `r` for raw
(`app.rs:960`). `PromptView` (`crates/hearth-cli/src/app.rs:109`) holds
`exchanges`, `cursor`, `selected`, `follow`, `scroll`, `max_scroll`, `loading`,
`pending`, and the render cache.

Two facts shape what is displayable:

- **One exchange per saved request, not one per turn.** `prompt_exchanges`
  (`crates/hearth-runtime/src/inspect.rs:49`) joins `prompt_blobs` to the
  assistant entry that answered it and returns
  `{request, prompt_hash, requested_at, response, usage}`. A task with nineteen
  turns produces nineteen exchanges, each a full snapshot of the same
  conversation at a different depth.
- **`usage` is always `null`.** The doc comment at `inspect.rs:41` explains why: `usage_events`
  has no `prompt_hash`, and the only candidate link is rewritten every turn.

There is no animation anywhere in the TUI today. The event loop polls at 100 ms
(`app.rs:1526`), which is a usable frame budget.

## What it should do

The observable requirements, with layout and styling left to the implementer:

- A turn reads as a conversation: the user's message, the assistant's response,
  and the tool calls and results between them, in order.
- System layers are present and identifiable as such. A reader can tell a system
  instruction from something the user said, and can collapse or skip the system
  layers when reading a long conversation.
- A saved request can be identified as a distinct snapshot, so the reader can
  tell which turn they are looking at and move between turns.
- While a turn is in flight the view shows that work is happening.
- The full text of every segment remains reachable. Nothing is truncated with no
  way to read it.
- One exchange per request becomes a transcript. Nineteen snapshots of one
  conversation must not read as nineteen copies of the same conversation.

## Constraints

**The last point is the hard one.** `prompt_blobs` stores a complete prompt per
request, so consecutive exchanges differ only in their tail. A chat-shaped view
has to decide what to do with that: show only the newest snapshot as the
conversation and keep older ones as history, or show the delta between
consecutive snapshots. Both are reasonable; showing nineteen full copies is not.

**The saved document is not the request.** See
`open/2026-11-16-the-saved-prompt-document-hides-the-runtime-note.md`: the
runtime note is attached to the newest message in the request but stored as a
separate `runtime` key in the document. A rebuilt view that renders the document
as a transcript will misplace that note until that issue is fixed. The two should
be worked in a known order.

**The event loop is 100 ms.** That is enough for a text spinner but not for
anything heavier. `build_cache` exists because rendering is expensive; a redesign
that re-wraps a 340 KB document on every frame will stutter. Reuse or replace the
cache deliberately.

**No input field.** There is no composer, no cursor in the transcript, no
sending. Keyboard handling stays navigation only (`app.rs:960`).

**Testing is string-based.** The existing tests (`prompt.rs:1054` onwards,
`render_audit` and its callers) assert on rendered text. A restructured view
keeps that shape of test: render to a fixed width, assert the visible lines.

## Style

Beautiful is a requirement, not a nicety. That means at minimum: colour used to
separate roles rather than to decorate, alignment that survives narrow widths,
no ragged wrapping, and a first screen that tells the reader what they are
looking at without a legend. `crates/hearth-cli/src/theme.rs` provides the
palette; `theme::think()` exists and is currently unused by this view.

## Not established

Whether "a turn" is one exchange or one conversation is a product decision the
implementer cannot settle alone. The two readings lead to different views:

- If a turn is one request, the tab is a request inspector with chat styling.
- If a turn is one conversation, the tab is a transcript and the request
  snapshots become a detail behind it.

State which one was built.

## Test

Rendered text at a fixed width. The requirements above are each a string
assertion: a user message is distinguishable from a system layer, tool calls and
results appear in order, the in-flight indicator appears while `loading` is true
and stops when it is false, and consecutive snapshots of one conversation do not
render as duplicates.
