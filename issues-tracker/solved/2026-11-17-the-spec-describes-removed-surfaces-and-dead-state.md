# The spec describes removed surfaces and dead record state

| | |
| --- | --- |
| Kind | chore |
| Reported | 2026-11-17 |
| Area | `docs` |

## Observed

`docs/SPEC.md` and `docs/SCHEMA.md` still contain claims the tree does not
support.

- "Which setting wins" and "Reloading" describe command-line flags,
  `HEARTH_*` precedence, a `SIGHUP` reload, and a settings file watcher.
  `crates/hearth-cli/src/main.rs` accepts only `--version` and `--help` and
  otherwise opens the TUI. The daemon handles `SIGTERM` and `SIGINT` only, and
  the only environment override read is `HEARTH_HOME`. Section 2.3 already says
  there are no flags or subcommands.
- "Turning the record into a prompt" says guidance files are read at run start
  and capped. Draft 0.34 and `Guidance::load` read every file whole; the cap and
  truncation markers are gone.
- "Summaries of old conversation" says a memory flush runs before summarizing.
  `Compactor::compact` asks for the summary and the memories in one JSON answer
  and writes the memories after parsing. Section 7.3 says the same call, so the
  two sections disagree.
- "The everyday tools" lists a single `memory` tool with update and pin actions.
  The current core set has `memory_save`, `memory_recall`, `memory_forget`, and
  `memory_transcript`, and the table's count of ten core tools does not match
  `build_tools`, which also adds `tool_search` and `task` under conditions.
- "The workspace and oversized output" says tool results include duration and a
  stable call id. `ToolOutput` carries only `content` and `is_error`; the call id
  is added by the assistant entry, not by the tool result.
- "How a save works" says one transaction covers memory rows. `Save` has no
  memory writes; `MemoryService` writes through its own `Storage::write`.
- "One answer, step by step" and "Tools" describe tool calls as durable tasks
  when they can outlive a turn. `Agent::execute_calls` runs every tool inline in
  the turn and saves the result after the call returns. No production code
  creates a task of kind `tool`, and the scheduler has no tool handler. The
  decision log says files and shell are inline tools.
- `SCHEMA.md` says a memory id is `m_` plus a UUID. `store::new_id` generates
  `m_` plus ten Crockford base32 characters. The current SPEC Draft 0.49 says
  the new format and says old ids still read.
- `SCHEMA.md` and SPEC 4.3 list `documents` and fork as current record kinds.
  No production code writes a document or forks a conversation.

## Wanted

Rewrite the affected sections against the current symbols and remove the claims
that have no code. If a removed feature is still wanted, file it as a feature
and describe the desired behavior rather than leaving a false description. Keep
historical drafts in the note at the top, not as current behavior.

## Verification

Each claim can be checked by reading the named module. Run `main` with any
argument outside `--version` and `--help`; it prints a refusal. Search for a
`SIGHUP` handler; none exists. Compare the tool table with `build_tools`. Read `Save` and
`MemoryService::write`.

## Resolution

Rewrote the sections named in the issue. The settings precedence and reload
sections now say there are no flags, only `HEARTH_HOME`, no `SIGHUP`, and no
watcher. Guidance is read whole. Compaction asks for the summary and memories in
one answer. The tool table lists `memory_save`, `memory_recall`,
`memory_forget`, and `memory_transcript`, and says when `tool_search` and
`task` join. Tool results carry content and an error flag, not duration or a
call id. A save does not include memory rows. Tool calls run inline, not as
durable tasks. `SCHEMA.md` records the Crockford memory id, the five record
kinds, and no `documents` table or fork. The crash-point list drops
`during_effect`.

Commit: `aea4c7f`. `scripts/check-docs.sh` passes; this is a docs-only change with
no behavior test.
