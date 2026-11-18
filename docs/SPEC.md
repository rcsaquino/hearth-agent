# Hearth Specification

**Status:** Draft 0.61, plain-language edition
**Date:** 2026-11-18
**Maintainer:** the project team
**Built with:** 100% Rust application code; SQLite is a bundled native dependency, and there is no Node, Python, or JavaScript runtime.
**Borrowed from:** the ideas in `@earendil-works/pi-durable`, not its code

---

## A note on language

Draft 0.61 makes the digest name a pair it cannot judge. Two rows about one
subject that overlap enough to be one fact, and that the write gate cannot prove
are one, are both shown and followed by one line naming both ids and the repair.
The store does not choose, because a reworded restatement overlaps by 0.476 and a
changed value by 0.667, so no word-overlap threshold separates them. A pair the
store can prove is now shown once and counted, for the same reason a canonical
duplicate is. The model closes the pair it judges to be one fact with
`memory(forget)`, and the line disappears with the row.

Draft 0.60 counts a load of a user-made skill. A learned skill keeps its counters
in its own frontmatter, but a user skill is read-only to Hearth, so the library
records the count in a hidden ledger beside it and never rewrites the user's file.
The Skills tab shows `used` for a user skill that has been loaded and keeps no
success rate for it, because an outcome cannot be written anywhere Hearth owns.

Draft 0.59 removes the keyword memory extractor. Every statement is written by
the agent, which reads the request and the store together, and the `memory` tool
description says to save when the user asks for something to be remembered. The
extractor rewrote the user's sentence and stored its own copy of statements the
agent had already saved, and a reworded copy is the one duplicate class the store
cannot refuse. The consolidation pass now closes the near pairs its write gate
would already have refused, keeps the fuller wording when one statement contains
another, and still reports the pairs that only overlap, because a restatement and
a changed value score the same on words alone.

Draft 0.58 stages each album member in the record and admits the group after its
quiet window on a later poll. The poller no longer sleeps on an album, the
window resets on every member, and a staged album survives a restart. The window
is 1 s, with no fixed cap.

Draft 0.57 makes an album one submission, keeps the quote of a rich message, and
delivers a shared answer once. Telegram sends an album as one message per photo,
so the poller holds the members for a short quiet window and admits them
together. Hearth's own answers are rich messages, so a reply to one carries a
`rich_message` rather than text, and that text now becomes reply context. A run
that absorbs a queued message settles both with the same answer, and only the
earliest submission for an answer owns its delivery, so a restart does not send
it again.

Draft 0.56 makes memory one tool with four actions, the same shape as `files`.
The `memory` tool takes a required `action`: `save`, `recall`, `forget`, or
`transcript`. The four separate tools are gone. The schema measures 491 tokens,
down from 811, and the core set measures 1,294 tokens, or 1,459 with every media
service configured, against a 1,470 guard. The handlers, the store, and the
status key `memory_recall` do not change.

Draft 0.55 gives memory one identity and one statement per row. A write is
refused when it carries more than one clear statement, and the refusal says to
split it. The same conservative splitter serves the extractor, so an
abbreviation, a decimal, or a repeated punctuation mark is not a boundary. The
extractor names `user` for a remembered first person, classifies a favourite as
a preference before the possessive clause, and composes both relation orders the
same way. The store compares synonyms, spelling variants, and negation only for
duplicates. Stored content and fingerprints do not change. An automatic extract
cannot replace a stronger subject, a non-fact, or a pinned row. The digest
collapses canonical duplicates and says how many it hid.

A nightly `memory.consolidate` pass closes canonical duplicates in one
transaction, reports near duplicates, cross-subject repeats, and compound rows,
and stamps `last_consolidation` and its report. The Memory tab shows pinned,
active, and retired totals from the store rather than the loaded page, marks
report candidates, and reads an audit line. The status payload carries the same
totals and the last audit. A submission records the user entry it answers, so
capture reads the run's own words instead of whichever message arrived last.

Draft 0.54 removes the browser, the documents tool, and the lazy tool groups. The
Camofox client advertised a service Hearth never installed, started, or checked,
and `tool_search` existed only to keep optional schemas out of the core prompt.
The `browser` tool, its client, the `documents` tool, and the group registry are
gone. `media` is an ordinary tool that appears only when a media service is
configured. The task tool loses its `tool_groups` input, and a checkpoint no
longer records a selection. The core schemas measure 1,611 tokens, or 1,776 with
every media service configured, against a guard of 1,800.

Draft 0.53 removes the document record and the conversation fork. Neither had a
production caller. Migration 0011 drops the `documents` table; the conversation
parent columns stay because helpers use them. Draft 0.52 makes the settings
schema strict. `identity.soul_file`,
`models.pricing`, `tasks.eta_reminder_minutes`, and the `https` fetch alias are
rejected like any unknown key. The TUI no longer writes a `daemon` section on
startup. Draft 0.51 drops the actor rule. A memory is stored in whatever words it was
written in. The store no longer refuses a pronoun, the extractor no longer drops
a statement its rewrite cannot resolve, and neither the `memory_save` schema nor
the compaction instruction mentions one. The agent picks its own wording, and a
statement with a pronoun is neither refused nor encouraged. Draft 0.50 had
required every stored memory to name its actor: a pronoun was refused with its
name, the extractor rewrote first person to `the user` and dropped an
unresolvable third person, and both prompts asked for a named actor. Existing
rows are not rewritten and still render.

Draft 0.49 gives a new memory a 12-character id: `m_` plus ten Crockford base32
characters, which drop `I`, `L`, `O`, and `U`. `memory_recall` accepts an id in
either case with surrounding whitespace, and a prefix is not an id. An existing
34-character id keeps working, because `superseded_by`, `memory_vectors.memory_id`,
and saved transcripts point at it.

Draft 0.48 makes each memory row a delimited header. A line is
`kind | subject | age: statement`, the kind is always present, an empty subject is
`-`, and a whole row carries no id. A clipped row appends its id after the
ellipsis, and the recall tool always ends a row with its id. The rules block is one
plain line per rule. The ten-row digest measures 611 estimated tokens, the worst
case 1,071, and the benchmark p95 falls to 97.

Draft 0.47 raises the memory content limit to 200 characters and bounds recall by
count alone. `memory.recall_max_tokens` is gone, `recall_max_items` stays 10, and
the fill keeps the highest scores instead of the best score per token. The worst
case is ten 200-character rows plus two coverage lines of three 120-character
subjects, measured at 1,143 estimated tokens; ten ordinary full rows measure 682.
A settings file that still sets `memory.recall_max_tokens` is refused, and the
error names that line.

Draft 0.46 moves spill to the temp folder. Oversized tool output is written under
`$TMPDIR/hearth/spill`, or `/tmp/hearth/spill` when `TMPDIR` is unset or empty.
The home no longer carries a `spill/` folder, and a restored home does not create
one. The root is created at mode `0700`, is refused when it is a symlink, is not a
directory, or is owned by another user, and a missing root makes a spill read a
not-found rather than a broken workspace.

Draft 0.45 names an uncounted skill instead of drawing a dash. The Skills tab
writes `(unused)` in the rate column for a user skill and a never-run learned
skill, widens the column to eight characters, and pads the label so a percentage
and `(unused)` end at the same edge. A counted skill keeps its percentage, and
`Skill::success_rate` and the sort and archive logic do not change.

Draft 0.44 gives the Prompt tab one fold state, folded at the start. The first
screen is a five-line preview of every foldable section, thinking included.
Enter opens the whole view and folds it again, `t` is gone, and the answer and
the live reply never fold. Thinking keeps its own block; only its fold key
changes. This supersedes the `t` key in Draft 0.40.

Draft 0.43 groups the Memory tab: pinned rows first, then current rows, then
closed windows, each group newest first. A retired row with a newer update time
can no longer sit above a current statement.

Draft 0.42 gives every memory one 150-character content limit, pinned rules
included. A longer write is refused and names its length and the limit; nothing
is truncated. Recall now defaults to 700 tokens and at most 10 items, and a new
memory fits its recall line whole. (The limit is 200 characters and recall is
bounded by count alone since Draft 0.47.) The 400-character rule cap is gone, while a
400-character read-side guard still protects a row written before this limit.

Draft 0.41 removes the prompt segment warnings. The per-layer token counts stay
in the saved prompt document and in the Prompt tab, but no count is compared with
a per-layer threshold. The only prompt limit is the context budget that shrinks
or refuses a request before it is sent. The fixed-overhead target and the
skills-index token warning are gone with the rest; the 50-active skill cap and
the memory budgets stay enforced.

Draft 0.40 gives thinking its own block. A saved or streaming reasoning block is
drawn with a `▌ THINKING` header, its own gutter, its own turn and time, and its
own five-line fold on `t`; the chat fold no longer hides it, and folding it never
touches the reply. A `HEARTH` block can no longer open with thinking, and a
`TOOL CALL` block can no longer contain it, because thinking is split out before
the block chooses its label.

Draft 0.39 lets the Skills tab filter by scope. `[` and `]` cycle both, user, and
learned, the header names the mode (`showing both`), and the empty state says which
mode is empty instead of claiming the library has nothing. The filtered list is the
one the selection indexes, so the body always matches the selected row.

Draft 0.38 makes the Skills tab read-only. `e`, `n`, and `v` are gone with the
editor, the new-skill prompt, and the version-history view they opened, along with
the footer, help, and empty-state hints that named them. The tab selects, reads,
and scrolls. A skill is read where it lives, and the library changes by editing the
files or through Hearth's own skill loop. `skills/<name>.md` no longer has a TUI
write path, so `EditorKind::Skill` is gone.

Draft 0.37 stops reporting a zero rate for a skill that never ran. A user skill
can never be counted, and a learned skill is uncounted until its first run, so
both now show an em dash in the Skills tab; a percentage appears only once a use
has been counted. A zero read as a measured failure and could not be told apart
from a skill that failed every time. (Draft 0.45 replaced the dash with
`(unused)`.)

Draft 0.36 gives every saved request an outcome. An assistant entry answers it, a
notice entry settles it without an answer and records the reason (a provider
error, an empty answer, the context budget, a stop, or an interruption), and no
entry means it is still in flight. The Prompt tab prints `no reply · <cause>`
only when a notice exists, so a running turn and an answered request stay quiet.
A budget refusal now saves the prompt it could not send, so the audit shows what
did not fit.

Draft 0.35 reports the segment budgets. A prompt layer over its spec budget now
prints a `▲ identity over budget · 3,200 tok of 800` line above the layer in the
Prompt tab, and the Doctor prints `prompt: identity over budget (3200 tok of
800)` from the newest saved prompt, plus a fixed-overhead line when the total
passes the section 14 target. The budget table, the segment lookup, and
`TokenCounts::from_document` live in `budget.rs`, so both surfaces read one
definition. The model is not told: it cannot trim `SOUL.md` or its own tool
schemas, and the person is the one who can.

Draft 0.34 reads guidance whole. `HEARTH.md` and every `AGENTS.md` reach the
prompt exactly as written: the 16 KB per-file cap, the 32 KB combined cap, the
truncation markers, and `Guidance.truncated` are gone. The per-segment cost
stays in the saved document and is reported. When the guidance alone leaves no
room for a turn, the budget failure names the largest guidance file and its
size instead of cutting an instruction the person wrote.

Draft 0.33 names the clock note for what it carries and anchors the working row.
The one uncached line is
`<current_time>2026-11-16T11:55:16+08:00</current_time>`, still appended to the
newest input message instead of a separate system message. The Prompt tab puts a
blank row between its status line and the chat. The live turn ends in a `WORKING`
row, so its clockwise flux spinner holds the tail of the card while the text
grows above it. Enter is the only fold key: it folds the chat and thinking
together, and `t` no longer folds thinking.

Draft 0.32 gives the Prompt tab one fold key. Enter folds or opens the chat and
keeps the first five wrapped lines of a folded section; `t` folds thinking on
its own. The agent's answer never folds with the chat, and saved thinking is
shown before the answer. `n`/`p`, `e`/`c`, the per-block fold overrides, and
the selection band are gone, so the view has no cursor and the footer stays the
only key legend.

Draft 0.31 rebuilds the Prompt tab as a chat window. Every saved message is a
role-labelled block: user and tool blocks are full-width panels, assistant text
sits on the page, and system layers and memory are their own folded receipts. One
cursor visits every block, Enter folds or opens it, and `e`/`c` expand or collapse
everything. Headers carry the loaded turn and the request time, the merged
transcript still prints each message once, and a response's thinking no longer
makes the next request look rewritten. Assistant text and thinking stream into a
live block at the tail.

Draft 0.30 finishes the Prompt tab as a conversation. Each message is enclosed
by a role bar, long blocks fold under an `n`/`p` cursor and open with Enter, the
snapshot vocabulary and its keys are gone, and one bare rule keeps the boundary
between saved requests. The footer is the only key legend. Assistant text and
thinking stream into a live turn drawn at the tail, with the spinner on the
message label and its phase starting with the turn; a saved request, not a
finished reply, reloads the document.

Draft 0.29 rebuilds the Prompt tab as one conversation. The saved requests are
merged into a single transcript and each request is marked as a snapshot with
its delta, so nineteen snapshots of one conversation no longer read as nineteen
copies. The header names the selected snapshot, model, session, token total,
prefix hash, and cache peers. System layers show once and collapse with `s`,
relevant memory is shown per request, a new session starts a fresh transcript,
and a spinner says when a turn is in flight. The saved prompt document marks the
message that carried the runtime note instead of storing the note as a peer
segment. The skills index no longer has a token cap: the 300-token warning budget was
removed in Draft 0.41, and only the 50-active count archives a learned skill. The Overview
skills row reports the active count and the index cost instead of the review
stamp.

Draft 0.28 moves the clock out of instruction position and bounds the two reads
that grew with the store. The runtime note is a `<runtime>` line carried by the
newest input message instead of a final system message, so a continuation turn
cannot answer the clock as though it were the request. The rules budget is 800
tokens, and the rules block counts its notice and joining newlines before it spends
the budget. The task tool no longer takes a per-helper token budget; a helper runs
on the configured context budget, and `wall_seconds` remains the time bound. The
Activity tab keeps one pair of keys for the ends of the feed. The recall subject
list is capped at 4,096 longest names behind a new index, and a 200,000-row lexical
retrieval measurement is recorded in section 14.

Draft 0.27 refuses a submission that carries no request. A message with no
non-blank text and no attachment is rejected before it is stored, so it never
becomes a turn and the clock note is never answered as though it were the
request. A stored entry with no request is also left out of the prompt, so an old
home stops carrying an empty message. The refusal names the reason to the caller
instead of leaving an empty turn in the record.

Draft 0.26 keeps an unpinned write from retiring a pinned rule. A statement that
repeats a live rule without naming it as `supersedes` returns the rule unchanged,
so the automatic extraction path cannot silently demote a rule the user pinned.
Only an explicit replacement closes the rule's window and clears its pin.

Draft 0.25 makes the systemd user unit self-healing and puts every timestamp in the
user's zone. A `login` or `startup` install owns both the unit file and its
`default.target.wants` link, and the daemon plus the TUI start path recreate
whichever is missing, so a hand-deleted unit comes back within a minute without a
daemon restart. Repair only adds: it never rewrites an existing unit and never
deletes. Timestamps shown to the person or the model are now ISO 8601 with the
offset in force at that instant, following `identity.timezone`: the runtime note,
memory `created` and `updated`, the activity and prompt views, the nightly backup
date, the backup and skill-review wall-clock slots, and the day boundary that the
usage totals and a bare `as_of` date use. Stored instants stay UTC.

Draft 0.24 rebuilds memory. Two parallel stores, a hand-edited `standing.md`, five
tools with an id-prefix convention, and a per-turn extraction model call are
replaced by one table in the main database, three retrieval channels, a linear
ranker over cheap features, and four tools that never call a model. Rules live in
the cached prefix and the digest sits in the uncached suffix, so the per-turn cost
is 116 tokens at the 95th percentile instead of 900. The digest now reports how
much it left out, and a replaced statement keeps its history instead of being
overwritten. Section 7 is the new contract, and section 7.9 carries the measured
numbers.

Draft 0.23 keeps the raw version hash out of the Overview. The memory card names the
number of rules instead of the 64-bit content hash, which stays in the
prompt export and the daemon status where it is useful.

Draft 0.22 makes `settings.json` the only bootstrap file. When the home loses its folders, `SOUL.md`, or an empty `HEARTH.md`, the daemon recreates them from the settings on the next start, so a home holding only a valid `settings.json` comes back as if onboarding had just finished. Existing files are never overwritten, and the repair runs only after the daemon holds its lock, so a systemd unit and a TUI-spawned daemon cannot both write.

Draft 0.21 adds local hybrid memory: the configured item and token budgets now bound automatic recall, snippets replace full rows, and tags stay out of the injected text. Long-term memory gains INT8 vectors from the local `bekko-embedding-v1-a8m` model, fused with FTS5 by reciprocal rank. A missing model falls back to lexical recall. New writes were limited to 2,000 characters and 8 tags (the content limit is now 200; see Draft 0.47), credential-shaped content is refused, durable cues trigger a background extraction pass, and Overview reports whether recall is lexical or hybrid. Nightly backups now include the memory stores.

Draft 0.20 replaces the two-card Prompt audit with one live, scrollable session export that shows every prompt layer, tool declaration, message, reply, and usage line in full. A reply or settled run refreshes the open view, and a coalesced follow-up read keeps the final saved answer from being missed. `[` and `]` walk older snapshots, and `f` toggles the live tail.

Draft 0.19 removes token pricing and cost tracking entirely. Usage keeps token counts and latency only. Draft 0.18 still covers the plain-text prompt skip, recent-activity seconds, bounded Prompt scrolling, Settings cleanup, media reset behavior, and the fixed SOUL.md path.

Draft 0.17 removes the editable soul-file setting (SOUL.md is always `$HEARTH_HOME/SOUL.md`), removes the duplicate Settings key hints, keeps media rows visible when reset, and drops the redundant web search/fetch heading.

Draft 0.15 raises the default output reserve to 32,000 and recent-token keep to 20,000, makes fetched models.dev limits the persisted reset defaults, has `r` fetch them when no cached lookup exists, adds space above the Settings tip, and orders the tabs Overview, Activity, Memory, Skills, Prompt, Doctor, Settings.

Draft 0.14 polishes the settings and onboarding surfaces: the timezone is a picker, the selected row shows its full tip, Workspace names the default folder, the separate Files tab is gone, the context window is labeled local, models.dev can fill the context window, the ETA reminder is measured in seconds, empty media drafts do not disappear, and onboarding skips disabled optional-service rows.

Draft 0.13 shows every setting inline on the final Settings tab, marks changed rows, adds a reset-to-default key, keeps only non-default values in `settings.json`, combines the Overview cards, and makes daemon shutdown and TUI close responsive. It also moves SOUL.md and HEARTH.md editing into Settings; the separate Files tab is gone.

Draft 0.5 adds the optional `telegram.base_url` override for local Bot API servers and self-hosted proxies.

Draft 0.12 leaves generation settings at the provider's defaults. Hearth sends no `max_tokens` and no sampling parameters; only `models.thinking` is sent, as `reasoning_effort` or DeepSeek's `thinking` toggle for off. `models.max_output_tokens` remains a local context-budget reserve and is never sent to the provider.

Draft 0.11 polishes the settings surface: a fresh home records `SOUL.md` as the identity file, the default run limit is 120 model requests, web search and fetch order are toggled and moved instead of typed, web provider API keys have masked form fields, and editing a setting shows a field-specific tooltip.

Draft 0.10 makes the TUI the complete customization surface: editable SOUL.md and HEARTH.md files, an all-settings form with optional media setup and a raw JSON fallback, a read-only saved-memory browser, and user-skill creation and editing. Every settings save is validated before the file is written atomically with mode 0600.

Draft 0.9 records the TUI usability pass and two additional thinking levels. The tagline is "A durable personal agent, built with Rust." The terminal palette has stronger contrast, form fields are visually distinct, onboarding explains terminal paste shortcuts, the Activity feed coalesces stream deltas into readable rows, the Prompt tab renders every prompt layer with token counts, and a daemon stop waits for the writer lock before the TUI exits.

Draft 0.8 records the first feedback slice: a message is acknowledged by the typing indicator, not a reaction; `identity.user_name` is removed; the speech tools teach the model Eleven v4 audio tags; and `HEARTH.md` plus project `AGENTS.md` files are included in the prompt as project instructions.

This document has two readers: the user, who wants to understand what Hearth is,
and the builder or AI, who must make it real without guessing.

To serve both, three kinds of writing appear throughout:

- **In plain terms** gives the idea in everyday language. Start here.
- **The rules** state exactly what must happen. These sentences are requirements.
- **Builder details** are the field tables, schemas, code blocks, and limits. A
  nontechnical reader can skip them.

"Must" means required for version 1.0. "Should" means strongly expected. "May"
means optional. Words in `code font` are literal names, paths, or values. Numbers
are exact. If a plain-language explanation and a precise rule ever seem to
disagree, the precise rule wins. This edition is a plain-language rewrite of Draft
0.1; no requirement was intentionally weakened.

**Suggested review path.** For a product review, read every "In plain terms"
paragraph plus the short version, glossary, sections 1, 4, 6, 7, 8, 17, and 18.
For implementation, read everything.

---

## The short version

Hearth is a personal agent that lives in Telegram and keeps its own records. It is
not a coding tool that also chats. It is a companion that can code when needed.

- **It remembers for years.** Facts, preferences, people, projects, and past work
  are kept in a memory built to last, not a chat window that forgets.
- **It never loses committed work.** A message, an answer, a tool result, or a task
  is written down before it is shown or acted on. A crash can interrupt work, but it
  does not erase work that was already saved.
- **It stays cheap.** Hearth reuses the beginning of each request, retrieves only
  what a turn needs, loads tools and skills on demand, and summarizes old
  conversation before it becomes expensive.
- **It is yours.** One user, one local database, no Hearth account, no telemetry.
  Model providers are choices, not dependencies baked into the design.
- **It is built test-first, and built small.** Every behavior begins as a failing
  test at a public boundary. Tests are part of the definition of done. Code exists
  only for the current milestone, in the simplest form that passes, with one
  definition per rule (YAGNI, KISS, DRY; section 15.5).

If you read nothing else, read sections 1, 4, 6, 7, and 8. They explain what
Hearth is, how it survives failure, how it stays affordable, how it remembers, and
how you talk to it.

---

## Glossary

| Word | What it means here |
|---|---|
| **Hearth** | The agent described by this document. |
| **User** | The single person Hearth serves. |
| **Daemon** | The one long-running program that owns the database and talks to Telegram. |
| **Conversation** | One ongoing chat and its permanent record. Telegram chats and threads map to conversations. |
| **Entry** | One permanent line in a conversation: a user message, an answer, a tool result, a summary, or a notice. Entries are never edited. |
| **Save (commit)** | One all-or-nothing write to the database. A save can include messages, task progress, memory rows, and a note that an outside action is about to happen. |
| **Session** | The single writer: the open database and the one path every change goes through. |
| **Document** | A small mutable JSON record attached to a conversation, changed only through a save. |
| **Submission** | A message handed in with a receipt, so a repeat is harmless. |
| **Run** | Everything from an admitted input to its final answer. |
| **Turn** | One model response and the tool calls it makes. |
| **Effect** | A recorded action outside the database, such as a Telegram send or a shell command. |
| **Task** | A piece of work that can pause, survive a restart, and continue from its last saved step. |
| **Tool** | Something Hearth can do beyond talking: read a file, run a command, search the web, send a file. |
| **Memory** | What Hearth knows beyond the current conversation. It has its own records, search, and maintenance. |
| **Context** | Everything the model reads for one request: instructions, memories, conversation, and tool output. |
| **Token** | A small piece of text the model reads or writes. Roughly 3/4 of an English word. Cost and limits are measured in tokens. |
| **Provider** | The company or local server that runs a model, for example OpenRouter or a local server. |
| **Model** | The AI that reads the context and writes the answer. |
| **Prompt** | The full package of text sent to the model for one request. |
| **Compaction** | Replacing an old stretch of conversation with a summary so the model can keep working without rereading everything. |
| **Checkpoint** | A saved note of the current step and what comes next, so work can resume. |
| **Provenance** | The record of where a fact came from and when. |
| **SQLite** | A small, single-file database built into the program. |
| **Backoff** | Waiting longer after each failed retry. |
| **Context window** | The maximum amount of text a model can read in one request. |
| **JSON** | Structured text that both people and programs can read. |
| **Idempotent** | Safe to repeat without changing the result. |
| **Seam** | A public boundary where a test can observe behavior without reaching inside the code. |
| **Yolo** | Running tools without asking for permission first. The default. |

---

## Contents

A note on language
The short version
Glossary

1. What Hearth Is
2. The Shape of the System
3. Where Everything Lives
4. The Permanent Record
5. One Answer, Step by Step
6. Keeping It Cheap
7. Memory
8. Telegram
9. Tools
10. Settings
11. Storage
12. Safety and Privacy
13. Watching the Cost
14. Speed and Size
15. Building It Test-First
16. Test Suites and Release
17. Milestones
18. Open Questions
19. Public Release
Appendix A. Defaults at a Glance

---

# Part 1: The Idea

## 1. What Hearth Is

**In plain terms.** Hearth is a personal assistant with a memory and a workshop. It
talks to you through Telegram, remembers what matters over years, and can do real
work: read and write files, search the web, run commands, make images and
speech, schedule reminders, and check on longer jobs. It is built for one person, on one machine,
with one durable record of everything it knows and does.

### 1.1 What it does

Hearth:

- talks to its user primarily through Telegram;
- remembers facts, preferences, people, projects, and past work across years;
- runs durable background tasks that survive restarts, crashes, and reboots;
- uses tools such as files, shell, web, media, and scheduling under a
  configurable policy;
- keeps its transcript, memory, task state, and usage ledger in one local database;
- stays affordable by caching, budgeting, compacting, and retrieving instead of
  stuffing everything into every request.

### 1.2 Seven principles

These are the decisions that shape everything else. When a later detail is
ambiguous, these principles decide it.

**1. Write it down before you show it.**
Every input, answer, tool result, memory write, and task step is saved before it is
observed or acted on. There is no exception: provider tokens are a preview, not a
record, and only saved text is sent. If the process dies, Hearth resumes from the
last saved point.

**2. Memory is a library, not a prompt trick.**
Memory is not a growing block of text pasted into every request. It has records,
provenance, search, conflict handling, consolidation (tidying and merging), and
forgetting. The model
asks the library for what it needs, and Hearth also brings a few relevant items
without being asked.

**3. Every token pays rent.**
A token spent on repeated instructions, unused tools, or six copies of the same
fact is a token not spent on the actual request. Stable text goes first, changing
text goes last, and every addition to the prompt has a budget.

**4. One user, one writer.**
A single daemon owns the database and its conversations. There is no multi-writer
coordination, no distributed consensus, and no conflict-resolution machinery for
multiple writers. This makes durability and reasoning about state much simpler.

**5. Local first, provider agnostic.**
State lives on the user's machine. The model provider and the service that turns
text into searchable numbers are configuration choices. Hearth itself does not
require an account, a cloud sync service, or a vendor-specific database.

**6. Boring, inspectable storage.**
SQLite and JSON. Any fact can be found with a query. A human can open the database
with standard tools and see what Hearth knows. No proprietary format stands between
the user and the record.

**7. Compiled capabilities, configured behavior, data knowledge.**
Tools are Rust code compiled into Hearth. Behavior is a small settings file plus
code defaults. Knowledge lives in memory rows and Markdown skills. There is no
untrusted plugin runtime and no dynamic JavaScript extension layer.

### 1.3 What it is not

Hearth is not, in version 1.0:

- a service for many people or organizations;
- a web app, mobile app, or desktop GUI;
- a plugin marketplace or a runtime for untrusted third-party extensions;
- a distributed system across several machines;
- a migration tool for other agent harnesses;
- a coding IDE or repository agent, although coding tools exist;
- a model trainer or fine-tuning system;
- a promise of perfect recall. Hearth targets measured, budgeted recall, not
  omniscience.

### 1.4 What it borrowed from pi-durable, and what it changed

**In plain terms.** Hearth borrows the durable ideas from pi-durable, not its
TypeScript code: save first, keep history immutable, treat work as resumable
tasks, and summarize old conversation without deleting it.

Changed, and why:

Changed, and why:

| Change | Reason |
|---|---|
| No dynamic extension registry | Hearth is single-user and local. Compiled Rust tools are easier to test, audit, and keep fast. Markdown skills cover the "add knowledge without recompiling" need. |
| No document operation log | A single writer plus a transactional database makes operation logs unnecessary. Documents are JSON values with a version number. |
| Memory is built in | Memory is the reason Hearth exists. It should not be an optional add-on with a different storage model. |
| Telegram is the primary transport | The user is already in Telegram. The transport is part of the product, not an example. |
| One user | Authorization is an allowlist, not a permission system. The threat model is different and simpler. |
| Token accounting is core | Cost is a first-class product constraint, not a hook added later. |

---

## 2. The Shape of the System

**In plain terms.** One program runs all the time. It owns the database, talks to
Telegram, runs the model calls, and does the work. Other commands are thin clients
that ask the running program for status or maintenance.

### 2.1 The always-on program

The daemon:

- holds an exclusive lock on the home folder, so two copies cannot write at once;
- serves a private Unix socket (a local-only communication file) for the
  command-line client;
- writes through one dedicated database thread and reads through a small pool;
- starts recovery before it accepts Telegram updates;
- shuts down gracefully: stop accepting updates, finish or checkpoint active work,
  save, release leases, exit.

A second interrupt forces exit. Recovery handles whatever was interrupted.

### 2.2 The parts

The code is split into small crates, which are Rust packages, so each part can be
tested and reasoned about alone. The names are literal.

| Crate | Job |
|---|---|
| `hearth-config` | Reads, validates, and reloads `settings.json`. Interpolates environment variables. |
| `hearth-core` | The permanent record: entries, saves, submissions, tasks, scheduler, and prompt assembly. |
| `hearth-storage` | Owns SQLite connections, migrations, full-text search, and backups. |
| `hearth-model` | Talks to providers: streaming, tool calls, retries, token estimates, and the media clients (image, transcription, speech). |
| `hearth-memory` | Statements, rules, retrieval, ranking, the digest, and one memory tool. |
| `hearth-tools` | The built-in tools, output spill, and workspace confinement. |
| `hearth-telegram` | The Bot API client, polling, rendering, and attachments. |
| `hearth-runtime` | Wires everything together: daemon, agent loop, notifications, shutdown. |
| `hearth-cli` | The `hearth` binary and the terminal application. |
| `hearth-eval` | Golden conversations, recall benchmarks, and crash injection. Development only. |

The main public interfaces are `Harness` (open the system and run work), `Session`
(the single writer), `Storage` (a database backend), `Provider` (a model service),
and `Memory` (the memory subsystem). Tests attach at these boundaries.

### 2.3 The application

There is one binary, `hearth`, and one user-facing entry: running it opens the
terminal application. There are no subcommands or flags.

| Area | What it does |
|---|---|
| Onboarding | First run replaces `hearth init`: bot token, user id, provider, API key, model picker, thinking, agent name, optional media, start mode, keep-running. |
| Overview | Runtime, model, thinking, context, usage and cache, tasks, memory, Telegram, providers, media, workspace, and recent activity. |
| Activity | The live event stream from the daemon, one row per whole event, with a readable event detail. |
| Memory | Browse and search saved memory records, pinned first, then current, then closed windows, each newest first. The title shows the store's pinned, active, and retired totals and the last audit's candidate counts; report candidates carry a warning mark. |
| Skills | Browse skills with automatic, scrollable body preview; create and edit user skills; learned skills stay read-only. |
| Prompt | A live conversation, folded at the start: the saved requests merged into one transcript, every prompt layer and message in its role bar, one key that opens every section from a five-line preview, memory per request, and the turn being written at the tail. |
| Doctor | Offline and live checks. |
| Settings | The last tab. Every option is shown inline, with SOUL.md and HEARTH.md actions, daemon controls, a raw JSON editor, changed-row markers, and reset to default. |

The TUI is a control plane. Conversations happen in Telegram; the TUI configures,
observes, audits, and controls the daemon. A TUI-started daemon runs detached from
the terminal. The Settings daemon controls show Start while the daemon is stopped
and Stop plus Restart while it is starting, running, or stopping. An attached
daemon can be stopped and restarted from this window. Closing the TUI signals a
daemon it owns and exits without waiting for it to release the lock; the Stop
action still waits for a clean stop.

### 2.4 One user at a time

Only one process may own a given home folder. Ownership is enforced by an exclusive
advisory lock on `$HEARTH_HOME/hearth.lock`, plus a process id (PID) record for
diagnostics. A second daemon refuses to start. A read-only inspection command may
open the database while the daemon runs, but it never writes.

Clients talk to the daemon over the socket in versioned JSON lines: one request
object and one response object per line.

### 2.5 Observing a run

The daemon publishes a live event stream on the private socket. The Activity tab
follows it.

- Activity attaches to the active run and shows helper or scheduled task events
  inline.
- Events: run started, turn started, thinking delta when the provider exposes it,
  text delta, tool call start, tool result, task step, usage update, and run settled.
- Assistant deltas are coalesced before they are published and fill the Prompt
  tab's live turn. The Activity feed keeps whole events, so partial text never
  becomes a row.
- Tool arguments and results are truncated and secrets are redacted. Thinking is
  shown live only; it is never replayed from storage.
- The stream is read-only. Closing the TUI does not cancel work. Attaching late
  shows events from that point; entries and tool results hold the earlier record.
- Telegram shows only the final answer. Activity is for the local terminal.
- Activity is for observing. The Prompt tab is a live export of the whole session as the model saw it.

---

## 3. Where Everything Lives

**In plain terms.** Everything Hearth knows lives in one folder. Backing up that
folder backs up Hearth. Deleting the wrong file can lose memory, so the table below
says what is safe to remove.

### 3.1 The home folder

The default home is `~/.hearth`. `HEARTH_HOME` can move it.

| Path | What it holds | Safe to delete? |
|---|---|---|
| `settings.json` | Credentials and a few personal choices. Mode `0600` (user-only). | No. This is the front door. |
| `hearth.db` | The durable record: conversations, memory, tasks, usage. | No. This is Hearth's memory and history. |
| `hearth.lock` | The single-user lock. | Only when the daemon is stopped. |
| `hearth.sock` | The private command socket. | Only when the daemon is stopped. |
| `SOUL.md` | Identity, voice, and standing character. Editable by the user. | No. Without it, Hearth loses its personality. |
| `HEARTH.md` | The user's rules and workspace guidance. Editable; may be empty. | No. These are the user's standing instructions. |
| `workspace/` | Default sandbox and durable output files. | Only after checking what is inside. |
| `skills/` | User-made Markdown procedures. Read-only to Hearth. | No. These are authored workflows. |
| `skills/learned/` | Learned skills, versions, and changelog. | Yes, but Hearth loses what it learned. |
| `media/inbound/` | Downloaded Telegram attachments. | Yes, with the understanding that old references break. |
| `media/outbound/` | Generated or staged files waiting to be sent. | Yes, after sending. |
| `logs/` | Structured diagnostic logs. | Yes. They are rotated automatically. |
| `backups/` | Database snapshots. | Keep the recent ones. |

**Why logs exist.** Logs record what Hearth did, when, and how long it took,
without message bodies by default. They are diagnostics, never prompt input (13).

Temporary work goes to `$TMPDIR/hearth-*`. Oversized tool output is the one current
example: its root is `$TMPDIR/hearth/spill`, or `/tmp/hearth/spill` when `TMPDIR`
is unset or empty. Temp data may vanish at any time, and nothing durable belongs
there.

`SOUL.md` is plain Markdown with three sections: Identity, Voice, and Rules.
`HEARTH.md` holds the user's rules and workspace guidance; it is read on every run
and may be empty. Hearth also reads every `AGENTS.md` from the filesystem root down
to the workspace, most specific last. Both are read whole: no cap applies to one
file or to the total. The prompt reports their cost, and a turn that cannot fit
them fails naming the largest file and its size rather than cutting an
instruction. `SOUL.md` is who Hearth is; `HEARTH.md` and `AGENTS.md` are what the
user wants it to do.

### 3.2 Which setting wins

There are no flags or subcommands beyond `--version` and `--help`.
`HEARTH_HOME` selects the home folder. Everything else comes from
`settings.json`, and an omitted value uses the built-in default.

Secrets may be written inline or referenced as `${ENV_VAR}`. A missing referenced
variable is a startup error if the feature that needs it is enabled. Optional search
providers are the exception: a missing key skips that provider. `settings.json` must
be mode `0600`; the daemon refuses to start otherwise.

### 3.3 Reloading

Settings are read once when a process starts. There is no reload signal and no
file watcher. Restart the daemon to apply a change; the TUI opens a fresh process
and reads the file again. An invalid file refuses the start and names the field.

### 3.4 First run

**In plain terms.** Setup should take minutes and require no file editing.

- Running `hearth` for the first time opens the setup wizard. It shows the
  `@BotFather` bot-token steps and the `@userinfobot` numeric-id steps, then asks
  for both values.
- It offers a provider menu: DeepSeek, OpenCode, OpenCode Go, Command Code, or
  another OpenAI-compatible endpoint. A preset supplies the base URL; the last
  choice asks for one. It shows a provider-specific key hint, then calls the
  OpenAI-compatible `GET {base_url}/models` and lists the ids. The user searches
  and chooses, or types a name when the list cannot be loaded.
- It asks for the default thinking level: provider default, off, low, medium,
  high, xhigh, or max. The choice is stored as `models.thinking`.
- The timezone is not prompted. It comes from `TZ`, the `/etc/localtime` target,
  `/etc/timezone`, then UTC.
- It asks for the agent name, then offers optional media blocks. Image generation
  asks for an OpenRouter key and model; transcription asks for a Groq key; speech
  asks for an ElevenLabs key. Each can be skipped.
- It asks when the daemon should start: when Hearth opens (default), at login, or
  at startup. Startup uses systemd user linger; if linger cannot be enabled, the
  TUI shows the exact command. It also asks whether a TUI-started daemon should
  keep running after the window closes.
- It creates the home folder, writes `settings.json` with mode `0600`, keeping only
  values that differ from the defaults, creates `SOUL.md` and an empty `HEARTH.md`,
  and runs migrations. It sets `models.session_header` automatically when the
  provider URL is opencode-go.
- It applies the start mode and starts or attaches to the daemon.
- The Doctor tab repeats the checks and explains every failure in one sentence.
- Settings changes are written atomically with mode `0600`; a start-mode change
  restarts the daemon.
- A `login` or `startup` install is two files, the unit and its
  `default.target.wants` link. The daemon and the TUI start path recreate either
  one when it is missing, so a hand-deleted unit heals without a restart.
- Every value remains editable in `settings.json`, including `identity.timezone`,
  which decides how every visible timestamp is rendered and when the nightly jobs
  run.

### 3.5 Recovery

**In plain terms.** `settings.json` is the only file Hearth cannot rebuild.

If the home folder loses everything except a valid `settings.json`, the next daemon
start restores the post-onboarding state: `workspace/`, `skills/`, `skills/learned/`,
`media/inbound/`, `media/outbound/`, `logs/`, and `backups/` exist again,
`SOUL.md` is written from `identity.agent_name`, and `HEARTH.md` is created empty.
`hearth.db` is created and migrated on the same start. Nothing that still exists is
overwritten, so a healthy home is left untouched and a restore is safe to repeat.
The temp spill root is not part of the home and is created on first use.

- The repair is idempotent and never rewrites an existing `SOUL.md` or `HEARTH.md`.
- Only the daemon repairs, and only after it holds `hearth.lock`; the TUI still never
  opens a database writer.
- A start-mode unit is used only when it names this home; otherwise the TUI starts
  its own daemon. An active unit is restarted rather than started, so a daemon left
  running after its home was wiped comes back against the restored record.
- Only credentials and personal choices live in `settings.json`; deleting it returns
  the user to the setup wizard.
- A record created from scratch logs one warning that Telegram may replay recent
  updates. No offset sentinel is added: a fresh setup starts at zero too, and
  dropping the pending updates instead would lose unread messages.

---

# Part 2: How It Behaves

## 4. The Permanent Record

**In plain terms.** Think of a ledger. Nothing is shown or acted on until it is
written in the ledger. A save is all or nothing. Work is split into tasks that can
pause, survive a restart, and continue from their last saved step. Actions in the
real world are written down before and after they happen.

### 4.1 The words

The glossary defines the vocabulary. The tables below define the records.

### 4.2 Ten promises

These are the guarantees the rest of the design protects.

1. **All or nothing.** Every part of a save lands together or none does.
2. **Nothing is real until saved.** Provider tokens are a preview, not a record.
3. **History is not rewritten.** A correction is a new entry or a new memory that
   supersedes an old one.
4. **Numbers never repeat.** IDs are never reused; the save sequence only increases.
5. **Actions happen outside the save.** Hearth writes the intent, acts, then writes
   the outcome. A transaction never waits on the network.
6. **A task knows its next step.** Every step saves the next step, a wake time, or a
   terminal outcome.
7. **One ending per message.** A submission ends once, as done, unanswered, or
   cancelled.
8. **A crash loses only unsaved work.** Saved order holds and settled work is never
   duplicated.
9. **One answer at a time per conversation.** New inputs are queued and folded in at
   the next boundary.
10. **Memory uses the same path.** Memory writes share the save, ordering, and
    provenance rules.

### 4.3 What is recorded

**In plain terms.** The record has five kinds: conversations, entries, submissions,
tasks, and effects. Each has a small set of fields and a few invariants.

IDs are time-ordered UUIDv7. Times are UTC microseconds, displayed in the configured
timezone. The exact columns are in `SCHEMA.md`; this section states what must hold.

- **Conversation.** One chat and its metadata.
- **Entry.** One permanent line. Append-only; a reset is an entry, not a deletion.
  The model sees the newest reset or summary marker onward.
- **Submission.** A message with a receipt. A duplicate `request_id` returns the
  original and never starts a second run. A submission whose content carries no
  request is refused, so every new turn has something to answer.
- **Task.** Resumable work. It carries status, phase, attempt, immutable input,
  checkpoint, result, lease, and wake time.
- **Effect.** One outside action with a stable key, payload, status, result, and
  attempts.

### 4.4 How a save works

**In plain terms.** Hearth gathers everything that belongs to one change, checks
that its assumptions are still true, writes the whole change, and only then tells
the rest of the system that it happened. Outside actions start after that.

The steps:

1. Begin a database transaction (a group of changes that either all succeed or all
   fail).
2. Append entries.
3. Update tasks and submissions.
4. Record effect intents, meaning "we are about to do this."
5. Increase the save sequence.
6. Commit.
7. Publish the committed state to watchers.
8. Run effect intents outside the transaction.

Publication happens only after SQLite reports success. Watchers include Telegram
delivery, command-line views, and the task scheduler. They receive a read-only
copy. If publication itself fails, the next process re-derives pending work from
the database; it does not rewrite the save.

**For builders.** A save is one SQLite transaction across `entries`, `tasks`,
`submissions`, and `effects`. Memory writes use their own storage transaction, so
one save never holds both. `PRAGMA synchronous=FULL` is the
default. `NORMAL` is allowed by configuration for lower latency, with the documented
loss window: a power failure can lose the last few transactions, but cannot corrupt
the database.

### 4.5 Work that survives a restart

**In plain terms.** A task is a step-by-step procedure with a memory. It finishes a
step, writes down what it learned and what comes next, then releases its claim. If
the process dies, another process can pick up the task and continue.

A task definition declares its kind, version, initial step, and step handlers. A
step handler returns exactly one of:

- `Next { phase, checkpoint }`: continue with the next step.
- `Sleep { until, phase, checkpoint }`: wake at a time, then continue.
- `Terminal { outcome }`: finish successfully, fail, or abort.
- `Failed { error, retryable }`: stop and record why.

The scheduler claims runnable tasks with a lease (a short-lived claim that says
which process is working on it), heartbeats while working, and saves each
transition. On startup:

1. Tasks with an expired lease become runnable again.
2. Tasks asleep with `next_wake_at <= now` become runnable.
3. Running tasks resume from their last saved checkpoint, not from memory.

A step must be idempotent from its checkpoint, meaning it is safe to repeat without
changing the result. Model requests are retryable and count attempts. A request that
kills the process twice is marked `poisoned` (blocked from retrying) and fails the
task with a user-visible notice instead of looping forever. Unsafe outside actions
are never retried; they are marked `unknown` and reported.

### 4.6 Actions in the real world

**In plain terms.** Hearth uses a three-part habit for anything outside its own
database: write down what it is about to do, do it, then write down what happened.
This is called the effect sandwich.

The three parts:

1. Save an effect intent with a stable key.
2. Perform the effect outside the transaction.
3. Save the outcome.

Each tool and effect kind declares a retry policy:

| Policy | What happens after a restart |
|---|---|
| `safe` | Retry freely. |
| `idempotent` | Retry with the same key, so the outside system can recognize a repeat. |
| `at_most_once` | Mark `unknown`, do not retry, notify the user. |
| `never_retry` | Mark `unknown`, require explicit user action to retry. |

Model generation is `safe`; it has no local side effects beyond recorded usage. Telegram sends are
`at_most_once`. If delivery is uncertain, the durable answer is delivered once as a
"recovered reply" notice on the next successful interaction rather than silently
duplicated. An answer is delivered once: a run that absorbs queued submissions
settles them with the active answer, and only the earliest submission that settled
with an answer owns its delivery, so a restart does not send a delivered answer
again.

### 4.7 Busy conversations and interruptions

**In plain terms.** Hearth answers one thing at a time per conversation. If you send
another message while it is working, the message is kept and folded into the work
at the next natural break.

The rules:

- One active run per conversation.
- New inputs during a run are saved as submissions and queued.
- At each turn boundary, the generation task should incorporate queued inputs as
  new user entries before the next model request. This is steering.
- If a queued entry lands between a tool call and its result, prompt assembly moves
  the result next to the call and drops unmatched calls or results. A provider
  never sees an unanswered `tool_calls` message.
- A run never changes a model request that is already running. Cancellation takes
  effect at the next boundary or through a provider abort.
- A message that is only `stop` aborts the active run and settles the submission as
  `cancelled`.

---

## 5. One Answer, Step by Step

**In plain terms.** A message becomes a small loop: think, use a tool if needed,
think again, then answer. The loop is durable at every step, so a crash does not
lose the conversation.

### 5.1 The loop

```
admit input -> submission queued -> generation task
  assemble context (instructions, rules, recall, transcript)
  model request (stream)
  no tool calls -> save answer -> settle submission -> done
  tool calls -> save answer with calls
               spawn tool tasks, wait
               save tool results -> next turn
```

A run ends when the model returns a final answer with no tool calls, the task fails,
or the user cancels. There is no turn limit. A tool loop runs until the model stops
asking for tools or the user cancels, matching the durable-agent behavior Hearth
borrowed its ideas from.

### 5.2 Turning the record into a prompt

**In plain terms.** Hearth stores messages in a form that does not depend on any
one provider, then translates them into whatever shape the chosen provider
accepts. Tool calls and results are kept paired. Images are stored as files and
encoded only at request time.

The rules:

- Content blocks do not depend on the provider; the adapter translates them.
- Tool calls and results are paired by call id. A result with no matching call is
  dropped with a warning.
- Images are referenced by stored file path and encoded only at request time.
  Base64 image data (a text form of binary data) is never stored in the database.
- The model's private reasoning blocks may be stored but are not replayed unless
  the provider and model support them.
- `HEARTH.md` and the workspace's `AGENTS.md` chain become one
  `<project_instructions>` system message after the identity message and before
  rules. Every file is read whole, and the result is shown in the Prompt tab.

### 5.3 Tools

**In plain terms.** A tool is a capability with a name, a description, a form for
its arguments, a safety level, and a time limit. Hearth calls it, saves the result,
and shows the model a bounded version.

A tool declares a name, description, argument schema, retry policy, timeout,
and output cap. All of it is compiled in.

Every tool runs inline in the turn, and its result is saved before the next
request. No production path creates a durable task of kind `tool`; shell and HTTP
calls finish or time out inside the turn. Results are truncated by
policy, with overflow written to spill.

Errors are structured with `code`, `message`, `retryable`, and `detail`, and
rendered compactly. Stack traces and raw provider errors never enter the context
unless the user asks for them.

### 5.4 Helpers and scheduled work

**In plain terms.** Hearth can hand a self-contained job to a helper. The helper
gets a brief, its own small conversation, a tool subset, and a budget. It returns a
report. The main conversation does not carry the helper's working clutter.

The `task` tool has four actions: `spawn`, `list`, `result`, and `kill`. A spawned
helper gets:

- a self-contained brief: objective, paths, constraints, and completion criteria;
- a model and tool subset;
- a token and wall-clock budget and an expected duration;
- no chat history, so its prompt stays small.

The defaults are four helpers at once, two levels deep (a helper
may spawn one more), and retention of 30 days or the newest 25 runs. A nightly
sweep deletes older helper task records and archives their conversations; the
entries and report file stay part of the record. When a helper settles, the user
gets one notice with the outcome and the report path. If it runs
past its expected duration plus the configured delay, one reminder is sent. A kill cancels the task, aborts any in-flight request, and settles silently.

The child runs like any conversation. Its final message is the report: it is saved
as an entry in the parent and delivered to the user.

The `schedule` tool creates sleeping tasks for reminders and recurring maintenance.
A missed wake-up runs once on startup, coalesced if several were missed.

### 5.5 Summaries of old conversation

**In plain terms.** When the conversation gets long, Hearth replaces the oldest
part with a summary. The raw messages stay in storage and remain searchable. The
summary is a new entry, not a deletion.

Triggers:

- background at 65% of usable context (`soft_compact`);
- blocking at 85% (`hard_compact`);
- idle after 60 minutes, when the conversation has at least `min(100000, 65% of
  usable)` estimated tokens and new history since its last compaction;
- a provider overflow error.

Range selection walks backward from the newest entry, adding estimated tokens until
`keep_recent_tokens` (default 20,000) is reached, then cuts at the next user or
assistant entry. A cut never lands on a tool result or a system entry, and an
assistant message keeps its tool results.

The summary request asks for the summary and the durable facts in one JSON
answer, and writes the memories after parsing it. Messages are turned into text
with tool results truncated. It must preserve decisions, open threads, names, dates, and
user preferences. A summary entry records `covers` (first and last sequence number)
and `first_kept`. Summaries may summarize earlier summaries.

Compaction uses the configured model, sends no output cap, and records its own usage.

Idle compaction handles at most one conversation per idle period, most recently
active first. It runs only when that conversation has no active run or queued
submissions. If another task holds the writer, it waits one more full idle period.
It never retries a history it has already attempted, and it is silent: no Telegram
message. It uses the same memory flush and summary path.

---

## 6. Keeping It Cheap

**In plain terms.** The model has a desk of limited size. Hearth keeps the stable
things at the front of the desk, puts new things at the end, and only brings out
what this turn needs. It reuses the provider's cache, retrieves summaries instead
of full histories, and lets a cheaper model do the housekeeping.

### 6.1 What the model sees

Every request is assembled in the same order:

| Layer | Content | How often it changes |
|---|---|---|
| L0 identity | `SOUL.md`, voice, hard rules | Stable |
| L1 project guidance | `HEARTH.md` and project `AGENTS.md` | Stable until edited |
| L2 tools | Core tool schemas | Stable |
| L3 rules | Pinned instructions, versioned | Rare |
| L4 skills index | Skill names and one-line descriptions | Rare |
| L5 transcript | Entries, summaries, tool results | Append-only |
| L6 recall | Memory retrieved for this input | Every turn |
| L7 runtime | Current time in `identity.timezone`, ISO 8601 | Every turn |

L0 through L4 form the cacheable prefix. It changes only when `SOUL.md`, the
guidance files, tools, rules, or skills change. Every layer's token count is
recorded in the prompt audit document, and the Prompt tab and the Doctor show
those counts without comparing them to a per-layer threshold. Every active skill
is indexed, up to the 50-active count cap, and a long description never retires
one. L5 is append-only, so the provider's prefix cache
keeps working. L6 is a delimited context message immediately before the newest user
message; it never rewrites the prefix. L7 is a `<current_time>ISO 8601</current_time>`
line paid fresh on every turn and carried by the newest input message, not by a
separate system message: it
is appended to the last tool result on a continuation and to the newest user message
otherwise. A trailing system note sits in instruction position, so a continuation
whose only new content is the clock can be answered as though the clock were the
request. The tags keep the note distinct from the message that carries it, and it
carries only the time: the conversation id and the model already travel with the
request.

Each section after identity is wrapped in an XML tag: `<project_instructions>`,
`<rules>`, `<skills>`, `<memory>`, and
`<summary_of_earlier_conversation>`. The default `SOUL.md` uses `<identity>`,
`<voice>`, and `<rules>`.

### 6.2 The budget

**In plain terms.** Hearth calculates how much room it has before sending anything.
If the request would be too large, it shrinks tool output first, then
summarizes, then reports a clear budget error rather than silently truncating.

For a model with context window `W`:

```
reserve   = max(configured_output, 4096) + 1024
usable    = W - reserve
soft      = soft_compact * usable
hard      = hard_compact * usable
keep      = min(keep_recent_tokens, 0.35 * usable)
```

Every request is estimated before sending. If the estimate exceeds `usable`, the
runtime first shrinks tool output, then compacts, then fails with a clear budget
error. Estimates use a real tokenizer where available and a characters/4
heuristic with a 10% margin otherwise.

`configured_output` comes from `models.max_output_tokens` and is a local reserve
only. Hearth does not send `max_tokens`, temperature, or other sampling parameters;
the provider and model keep their own defaults. The one generation control Hearth
sets is the reasoning level from `models.thinking` (`reasoning_effort`, or DeepSeek's
`thinking` toggle for off).

### 6.3 Cost rules

1. **Stable prefix, changing suffix.** Anything that changes per turn goes last.
2. **Retrieve less, later.** Summaries and recall first; full content, optional
   tools, and skill bodies only on demand.
3. **Bounded by default.** Inline tool output is capped at 4,000 tokens and spills
   to a file. Prompts carry no duplicates and no filler.
4. **Bounded housekeeping.** Extraction, summaries, titles, and classification get
   narrow instructions and bounded inputs.

### 6.4 Reusing the provider's cache

**In plain terms.** Providers can remember the beginning of a prompt and charge less
when the next request starts the same way. Hearth keeps the beginning stable and
relies on the provider's automatic prefix cache. There is no provider-specific cache
code.

The details:

- One OpenAI-compatible adapter: `POST {base_url}/chat/completions` with SSE
  streaming, tool calls, and usage from the final chunk.
- It supports `api_key`, an optional `headers` map, and an optional `session_header`
  sent with the conversation id, as opencode-go requires.
- The prefix hash is stored with each generation checkpoint. A prefix change is
  logged with its cause: `SOUL.md`, tools, rules version, or skills index.
- Cache reads and writes are recorded separately in the usage ledger.

### 6.5 One model

One model is configured under `models` and used for everything: conversation, tool
use, memory capture in compaction, summaries, titles, and classification. There is no
second routing path. The usage ledger and the TUI Overview record the provider and model
that served each request.

Housekeeping stays cheap by bounding the work, not by switching models or changing
the provider's generation defaults.

### 6.6 Auditing the prompt

Every byte sent to the provider comes from the assembled prompt. There are no hidden
preambles and no server-side additions.

- The exact prompt text is saved in `prompt_blobs` before the request and linked from
  the generation checkpoint. A prompt the budget refuses is saved too, with the
  notice that says why it was not sent.
- A saved request is answered, settled without an answer, or still in flight. The
  answer is an assistant entry carrying the prompt hash. A request that fails, is
  stopped, or runs out of budget saves a notice entry carrying the same hash and
  the machine reason, and the Prompt tab prints `no reply · <cause>` under it.
  The causes read `provider error`, `the model returned no answer`, `over the
  context budget`, `stopped`, and `interrupted by a restart`. A request with no
  entry stays quiet, because only a settled request has a cause to name.
- The Prompt tab is one conversation assembled from the loaded saved requests.
  Consecutive requests are merged by their common message prefix, so each
  request prints only the messages it added. A rewritten history, such as a
  compaction, prints a notice instead of hiding the change. A change of session
  prints a conversation boundary and starts a fresh transcript. Memory prints
  when it changes; a stable recall is not repeated. A blank row separates the
  status line from the first block.
- Every block is one message or one context layer. User and tool blocks are
  full-width background panels, assistant text stays on the page, and system
  layers and memory are tinted receipts. Each header opens with `▌`, names its
  role in uppercase, and carries the loaded turn and the request time in
  `identity.timezone` on the right. A context layer reads `SYSTEM · SOUL.md`, a
  user message `YOU`, an assistant message `HEARTH`, a tool call
  `TOOL CALL · name`, a tool result `TOOL RESULT · name`, and recalled memory
  `MEMORY`. A message with several parts keeps one header. Thinking is shown
  before the answer, both saved and streaming, and it is its own block: a
  `▌ THINKING` header, its own gutter, its own turn and request time. It folds
  with the chat. The next request never sends it back.
- Enter folds the whole chat and Enter again opens it. A fold applies to each
  body section: context layers, memory, user messages, tool calls, tool results,
  and system transcripts. A folded section keeps its label, its first five
  wrapped lines, and a stub naming the hidden line count. A section with five
  lines or fewer shows in full and gets no stub. A block with several parts
  previews each part. The agent's answer never folds with the chat. Thinking
  folds with the chat. The chat starts folded, so the first screen is a compact
  outline of every section. Enter opens every foldable section, thinking
  included, and the next Enter folds them again. A folded block still shows its
  runtime note.
- Up/Down scroll one line, PageUp/PageDown move one page, and Home/End (or
  `g`/`G`) jump to the top or tail. `f` toggles live follow. Enter changes a
  fold without moving the scroll position. The footer is the only key legend.
  Scrolling away from the tail pauses follow; End or reaching the bottom resumes
  it.
- Assistant deltas are drawn as a live block after the cached document: a
  `THINKING` block first with `streaming` on the right, then text, then a
  `WORKING` row with a spinner in place of `▌` and `streaming` on the right. The
  working row is the last row of the block, so the spinner holds the tail of the
  card while the text grows above it. The block keeps a blank row above the
  working row and one below it. The eight-frame clockwise flux spinner steps one
  frame every 100 ms and its phase starts when the turn starts. The live answer
  never folds; live thinking folds with the chat to the same five-line preview.
  The live block is never cached. It clears when
  the run settles. A saved request or a settled run reloads the document; a reload
  alone does not animate. The status row names the model, the loaded turns, the
  message count, the token total, the session, and the prefix hash of the newest
  saved request.
- A saved response contributes to the merge baseline in the form it is sent
  back: its thinking blocks are dropped and a message that carries no request is
  omitted. A response with thinking therefore neither makes the next request
  look rewritten nor repeats the answer.
- Tool definitions show argument names, types, enums, required or optional
  markers, and argument descriptions instead of pretty-printed schema JSON.
  Wrapping preserves leading indentation for code and JSON. The rendered
  conversation is cached by the loaded set, width, the fold state, and zone, so
  scrolling and streaming do not rebuild the document.
- The open view refreshes when a `task_step` request event or a `run_settled`
  event arrives. A refresh that arrives while one is already running is
  coalesced into one follow-up read, so the final saved response is shown even
  when the settled event follows closely.
- The view is read-only. Saved prompt content is redacted by the save path.
- Prompt blobs are pruned after 30 days by default.

---

## 7. Memory

**In plain terms.** Memory answers three different questions, and it gives each
its own shape. What does the user want done? That is a rule, and it belongs in
front of the model on every turn without being searched for. What did the user
say? That is knowledge, and it belongs in an index that can be reached by name,
by word, and by meaning. What was actually said, word for word? That is the
transcript, and it is never edited.

Memory is global. There is one library for the user, not a scope per chat or per
project. Every conversation and background task reads and writes the same rows.

### 7.1 Three stores and one database

| Store | What it holds | Where it lives | When it is loaded |
|---|---|---|---|
| Rules | Instructions that must be obeyed without a retrieval cue | `memories` rows that are pinned and still current | Every request, in the cached prefix |
| Knowledge | Facts, people, preferences, decisions, commitments, episodes | `memories` rows | A digest before the newest user message; detail on demand |
| Evidence | Every saved conversation entry | `entries`, indexed by `entries_fts` | Only when the model asks |

All three live in `hearth.db`. A memory is written by the same writer thread as
the entry it came from and is included in the same backup. A memory write is its
own transaction, not a rider on the conversation save: the agent writes memory
after the turn, and a memory that outlived a failed save is harmless while a save
rolled back because of a memory would not be. `source_entry` is therefore a soft
reference to the entry that produced it. There is no second database file and no
hand-edited markdown store.

Rules are not a separate table because a rule is a statement with a flag, not a
different kind of evidence.

### 7.2 What a memory is

One row is one statement the user would recognise as one thing they said.

| Field | Notes |
|---|---|
| `id` | Stable `m_` id: ten Crockford base32 characters, excluding `I`, `L`, `O`, and `U` |
| `kind` | `rule`, `preference`, `person`, `decision`, `commitment`, `episode`, or `fact` |
| `content` | One standalone statement; a write is capped at 200 characters and refused when it carries more than one statement |
| `subject` | The normalised person, project, or thing it is about |
| `pinned` | Injects the statement as a rule; only valid with `kind = rule`, and only while the row is current |
| `source_kind`, `source_entry` | Provenance, down to the entry the user typed |
| `valid_from`, `valid_to` | The window the statement was true |
| `superseded_by` | The statement that replaced this one |
| `created_at`, `updated_at`, `revision` | Bookkeeping |

A memory is read with no conversation around it, so a statement written in
pronouns depends on what it was written beside. That is a cost of the words, not
a rule: the content is stored as written, and a pronoun is neither refused nor
required. The `subject` field, not the wording, is what makes a statement
findable. Existing rows are not rewritten.

A write carries one statement. Content with two clear sentences is refused with
a message that names the number and asks the caller to split it. The splitter is
conservative: a title such as `Dr.`, an initial, a decimal, an ellipsis, and a
lowercase continuation are not boundaries. A row written before this rule keeps
its compound content; the audit reports it, and nothing rewrites it.

There is **no importance score**. A stored importance is a number no one can
calibrate, and it invites a write-time model call to produce it. Ranking is
computed from evidence at read time instead.

A plain read is the same validity rule evaluated at now. A statement dated in the
future does not answer a question asked today, and a closed statement answers
only an as-of read inside its window.

`valid_to` is what makes contradiction cheap. A statement is never overwritten.
When a newer statement replaces it, the old row's window closes and
`superseded_by` points at the replacement. "What is true now" reads the open
windows; "what did we think in March" reads the closed ones. Nothing is lost, and
no reconciliation model call is needed to keep the two apart.

`subject` is the strongest retrieval signal a personal memory has, because a
personal question almost always names somebody or something. The model sets it
when it writes, and the statement's expansions are indexed beside it.

### 7.3 How memories get in

**The agent writes.** The `memory` tool with `action = save` is the only path from
a turn, and the tool description carries the promise: save when the user asks for
something to be remembered. The model names the subject, chooses the kind, and can
name the statement it replaces. The write result reports any current statement
about the same subject that overlaps the new one, and names the pair as one the
store cannot judge, so a contradiction or a restatement is repairable on the same
turn rather than discovered by a later question.

**There is no second writer.** A deterministic pass used to read "remember that
..." from the user's message, rewrite the sentence into third person, and store
it. It only ever produced a second copy in different words, which is the one
duplicate class the store cannot refuse, so it is gone.

**Compaction carries extraction in the same call.** The summary request already
contains the range, so it returns the summary and up to three durable statements
in one JSON object. This replaces a second request over the same text. Neither
path adds a request: the save is part of the turn, and the extraction rides on the
summary call.

Write rules:

- Identical content is refused and the existing row is returned. A live pinned
  rule wins over an unpinned copy of the same words, so the copy is refused and
  the rule stays current.
- A near duplicate about the same subject is refused the same way, which is what
  stops eight weeks of "ordered pad thai" from becoming eight rows. A duplicate
  is judged by token containment. The comparison folds common spelling variants
  and the preference verbs `prefer`, `like`, and `love`. It runs only for the
  duplicate decision, so stored content and the fingerprint keep the words the
  caller wrote. A negation is never a duplicate of its affirmative form. A
  statement that swaps its own non-empty subject is never a duplicate, but a
  subjectless write is compared against the store so the missing field cannot
  hide a repeat.
- A statement that contains an earlier one replaces it, so the fuller wording is
  the live one and nothing is lost: every word of the earlier row is in the new
  one. Between two statements that merely overlap, the new one is written beside
  the earlier row, because token overlap cannot tell a restatement from a changed
  value. The two reported pairs measure this: a reworded restatement of one fact
  overlaps by 0.48, while "the primary database is SQLite" against "Postgres"
  overlaps by 0.67. Any threshold low enough to merge the first merges the
  second.
- The write result names a related current statement about the same subject, with
  its overlap and how to remove one of the pair. The store does not call the pair
  separate, because it cannot know.
- An automatic extract, which is now only the compaction summary, cannot replace a
  subjectful, non-`fact`, or pinned row with a weaker one. It returns the earlier
  row instead.
- A pinned rule must fit the rules budget; a write that would overflow it is
  refused with the current rules listed, so the model can replace one.
- Credential-shaped content is refused before it reaches the table.
- Every statement, a pinned rule included, is capped at 200 characters after
  trimming. A longer write is refused, names its length and the limit, and saves
  nothing. A new memory fits its recall line whole; only a row written before
  the limit is clipped for display.
- The store does not inspect the wording. A pronoun is stored like any other
  word, and nothing in the write path asks for a named actor. The store keeps its
  other gates: empty content, the 200-character limit, NUL, a subject over its
  limit, credential-shaped content, and a pin that is not a rule.

### 7.4 How memories come back

Retrieval is a four-stage cascade, and every stage is local work.

```
message
  -> analyse: drop scaffolding, keep names, expand the forty relation groups
  -> candidate generation, three cheap channels into one pool of 128
       lexical   FTS5 porter + prefix, strict then broad
       entity    exact and prefix match on subject
       semantic  local INT8 vectors, cosine floor 0.40
  -> fuse by reciprocal rank, k = 60
  -> rank by a linear score over cheap features
  -> diversify, then keep the best scores up to the item cap
```

The ranking features are the evidence a personal message actually provides.

| Feature | Weight | Why |
|---|---|---|
| Fused relevance | 1.00 | What the channels agreed on |
| Subject match | 0.70 | The message named this statement's subject |
| Term coverage | 0.80 | Share of the message's own words the statement answers |
| Kind prior | 0.25 | A preference is more often the answer than an episode |
| Still current | 0.35 | A closed window is history, not the answer |
| Recency | 0.10 | A tie-break, never a filter |

Term coverage compares with a prefix in either direction, which is the cheap
stand-in for a stemmer and is what lets "queued" find "queue". The fill orders by
score and keeps the best 10, so a long decisive statement is not displaced by a
shorter one at a lower score.

An as-of read skips the semantic channel. The vector index holds only current
statements, and mixing today's meaning into a question about March would answer
the wrong question.

### 7.5 The digest, and why it says what it hides

Automatic recall sits immediately before the newest user message, which is the
one prompt position that is never cached. Every token there is paid in full on
every turn, so a line is terse:

```text
fact | alice | 12d: Alice manages the OR board
pref | - | 3mo: prefers dark roast coffee
more: alice 6 | memory(recall, subject) for the rest
```

A line is the kind, the subject, the age, and the statement, separated by ` | `
with a colon before the statement. The kind is always present, so every row
starts with a known token. An empty subject is `-`, so the field count never
changes, and a multi-word subject stays one field. There is no bullet: the
newline is the row boundary, and a statement cannot contain one.

A whole row carries no id, because the statement is shown whole. A row clipped at
200 characters appends its id after the ellipsis, because the `memory` tool with
`action = recall` and `id` is the only way to read the rest. The recall action always ends a row with its id,
because that is where addressing belongs. A new id is `m_` plus ten Crockford
base32 characters; an older 34-character id keeps working. The recall action
accepts an id in either case with surrounding whitespace, and a prefix is not an id.

A duplicate the store can prove, meaning the same fingerprint or one statement
that contains the other, under the same subject, is shown once and the digest
reports how many rows it collapsed. The repair pass closes it later; the digest
does not spend tokens on it before then. The digest uses the write gate's own
test, so a reworded copy is collapsed too.

Two rows about one subject that overlap by 0.45 or more, and that the store cannot
prove are one statement, are both shown with one line that names their ids and the
repair. The store does not pick one: a reworded restatement and a changed value
overlap by about the same amount, so the line says `similar: m_a/m_b | if they say
one thing, memory(forget) one` and leaves the judgement to the model. The line
names at most two pairs, strongest overlap first, and is measured to fire on 10 of
the benchmark's 195 questions. A pinned row is never hidden or closed by it.

The coverage line is the part that makes recall honest. For each subject the
message named, the store counts what it holds and reports the difference. A digest
that silently shows six of fourteen facts teaches the model that six is all
there is. Naming the remainder, and the call that fetches it, is the difference
between a bounded digest and a bounded memory.

The rules block is capped at 800 tokens. Recall is bounded by count: at most 10
statements of 200 characters each, plus at most two coverage lines of three
subjects. The worst case measures 1,071 estimated tokens and ten ordinary full
rows measure 611; the benchmark digest is 97 tokens at the 95th percentile.

### 7.6 Rules

Rules are injected as a system message in the cached prefix, ordered by creation
so an edit does not reorder the block and the prefix hash only moves when the
text moves.

- The block is capped at 800 tokens. A write that would exceed the cap is
  refused, not silently evicted.
- Every rule uses the one 200-character content limit. A row written before
  that limit is clipped at 400 characters for display, with an ellipsis.
- The block renders one plain line per rule, with no id, no fields, and no bullet.
  A rule is read, not addressed; `memory` with `action = recall` and `kind = rule`
  returns the ids.
- Any rule that does not fit the render is named as `N more rules were not
  shown`, so a hand-edited store cannot hide content silently.
- The content hash is the rules version, so a change invalidates the cached
  prefix and the Prompt tab names `rules` as the changed layer.
- A statement that is not a rule cannot be pinned. Pinning a fact would repeat it
  on every turn, which is the rule's job.
- A rule is an instruction, not a sentence about a person. An imperative such as
  `Never deploy on Friday` is stored like any other content, and a pronoun in one
  is neither refused nor required.
- A pin belongs to the live statement. A write that names a rule as `supersedes`
  retires it: the window closes and the pin clears, so it leaves the block and
  stops spending the budget. An unpinned copy that does not name the row returns
  the rule unchanged. A write that arrives already retired cannot be pinned.

### 7.7 Forgetting and secrets

- The `memory` tool with `action = forget` removes one statement permanently,
  including its vector.
- A replaced statement is not deleted. It leaves the current set and stays
  readable by id and by `as_of`.
- There is no decay and no automatic archival. A statement stops being current
  only when another statement replaces it, a canonical or provable near duplicate
  is closed by the audit, or the user removes it.
- The nightly `memory.consolidate` pass makes no model call. In one transaction
  it closes all but one row of each canonical group, keeping the pinned row and
  then the stronger kind, and records the report and the instant. It closes a
  near-duplicate pair when the write gate would already have refused the second
  write: the same statement under one subject, or one statement that contains the
  other. The fuller wording is the row that stays. A pair that only overlaps is
  reported and left current, because closing it could retire a different fact.
  It also reports one statement under different non-empty subjects and compound
  rows, and changes none of those. A second run is a no-op.
- The report is bounded. Canonical groups are found over the whole store; the
  near and compound reads take the newest 10,000 current rows, and each sample
  list stops at 20 examples. The pass is restart-safe: a crash rolls the whole
  transaction back, so no row is half-closed.
- Credential-shaped content is refused before a write. The check runs in the
  store, so every path shares it.

### 7.8 The memory tool

One tool, four actions, and each action does one thing.

| Action | Use |
|---|---|
| `save` | Store one statement, optionally replacing another |
| `recall` | Search, read by id, read a subject, or read as of an instant |
| `forget` | Remove one statement permanently |
| `transcript` | Search the saved conversation record, or read one citation |

`action` is required. The properties are flat and shared where the actions are
related: `subject`, `kind`, and `limit` serve both `save` and `recall`, `id`
serves `recall` and `forget`, and `query` serves `recall` and `transcript`. There
is no store selector, no id-prefix convention, and no tag array. An id is exact,
never a prefix. The one schema measures 491 tokens against 811 for the four it
replaced, and the saving is paid on every request.

A bare `as_of` date means that whole local day in `identity.timezone`, so its
boundary is the start of the next local day. An instant is read exactly as given.

### 7.9 How we know it works

**In plain terms.** Memory is measured on a fixed fixture, and the numbers are
printed with the target beside them.

`crates/hearth-eval/tests/memory_bench.rs` replays 200 planted statements as the
history they describe, then asks 195 questions across seven categories. It
reports two numbers on purpose:

- **Automatic recall** is what the digest put in front of the model.
- **Recoverable** is automatic recall, or the answer appearing in one
  `memory` recall call.

The second number is the honest form of "nothing is lost", because a bounded
digest will always miss something. Measured on the committed fixture:

| Measurement | Target | Result |
|---|---|---|
| Recoverable in one call, lexical | 0.98 | 0.99 |
| Recoverable in one call, hybrid | 0.98 | 0.99 |
| Automatic recall, lexical | 0.92 | 0.933 |
| Automatic recall, hybrid | 0.92 | 0.954 |
| Paraphrase, automatic, hybrid | 0.85 | 0.900 |
| Digest tokens, p95 | 300 | 97 |
| Retrieval latency, p95 | 100 ms | 6.5 ms lexical, 14.1 ms hybrid |
| Contradiction answered by the current statement | all | 15 of 15 |
| Duplicate writes that became new rows | 0 | 0 |
| Secrets stored | 0 | 0 |

`crates/hearth-eval/tests/memory_reachability.rs` proves the stronger property
that makes 100% a construction rather than a hope: every stored statement is
reachable by its id, by its subject, by a direct term, and by paging the whole
store. Enumeration is always available, so a statement can be unreachable by a
question but never by the system.

The residual is stated rather than hidden. Two of the 195 questions need more
than one call, and both are paraphrases with no shared vocabulary. Six of the
thirteen automatic misses sit on a topic the fixture gives two live answers to,
which is an ambiguity no ranking can settle. The vector channel is measured only
when the local model is installed; without it recall falls back to lexical and
the paraphrase category drops to 0.65.

---

## 8. Telegram

**In plain terms.** Telegram is the front door. It is also the security boundary.
Only the user's account is allowed. Duplicate messages are ignored, attachments
are downloaded, replies update in place as they are written, and a crash cannot
make the bot process the same message twice.

### 8.1 Who is allowed

- `telegram.allowed_user_ids` must be non-empty when `telegram.bot_token` is
  present. An empty list is a startup error, not an open bot.
- `allowed_chat_ids`, when non-empty, further restricts chats. Groups require both
  the user and the chat to be allowed.
- Unauthorized updates are ignored and logged without content.
- The bot token is read from settings or an environment reference and is never
  logged.

### 8.2 How messages arrive

The poller:

```
getUpdates(offset, timeout=poll_timeout, allowed_updates=[message, callback_query])
  -> authorize
  -> dedupe update_id (unique insert, same save as the submission)
  -> download attachments
  -> save submission with request_id tg:<chat>:<message_id>
  -> persist the offset only after the submission save
```

`update_id` is stored in `telegram_updates` with a unique constraint. A crash
between Telegram's response and the save replays the update; the dedupe row makes
the second attempt do nothing. The offset is saved in the same transaction as
processing, so it never advances past work that was not saved yet.

Ordering is preserved per chat. A chat's submissions execute in save order. The
daemon shows the typing indicator while a run is active; it does not add a seen
reaction to the message.

An album is one submission. A member is staged durably with its group's ready
time, and the offset advances in the same save. The poller never sleeps on an
album. It stages the member, keeps polling, and submits the group at the start of
a later poll once 1 s has passed since the last member. The wait resets on every
member, so a slow album keeps its full window. Every photo or video is kept in
message order, and a repeated caption appears once. A group still staged after a
restart is submitted on the first poll after startup.

A reply carries the quoted message as a context block that names its author and
its text, shortened to 500 characters. A quoted rich message, which is what Hearth
sends, is flattened to plain text from its blocks. A quoted message with no text is
described by its media kind.

### 8.3 How replies look

- Outgoing text uses Telegram rich messages (`sendRichMessage`, Bot API 10.1 or
  newer) with Markdown, up to 32,768 UTF-8 characters and 500 blocks. Headings,
  bold, italic, underline, strikethrough, spoilers, links, inline code, fenced
  code, nested lists, blockquotes, tables up to 20 columns, math, footnotes, and
  collapsible sections are supported.
- If Telegram rejects the rich message or it exceeds a limit, Hearth falls back to
  `sendMessage` with message entities, split at 4,096 characters on paragraph or
  code-block boundaries with `(n/m)` markers. Markdown is converted to plain text
  plus entities. On parse failure, it sends plain text. Only a server
  rejection triggers the fallback; a transport failure stays uncertain and is
  never resent by a different method.
- Code blocks are never split mid-block. Oversized blocks are split with fences
  reopened.
- While a run is active, Telegram shows the typing indicator. Hearth sends one
  final answer when the run settles. Partial text, thinking, and tool calls never
  appear in the chat.
- Hearth respects HTTP 429 rate-limit responses and the server's requested wait
  time (`retry_after`). A global limit allows at most 30 sends per second.

### 8.4 Files and voice

- Inbound photos, documents, audio, voice, and video are downloaded to
  `media/inbound/YYYY-MM-DD/` with safe file names and size limits. The Bot API
  download limit is 20 MB. Larger files produce a clear notice.
- Images are downscaled to at most 1568 px on the long edge before being sent to the
  model when it accepts images. Originals are retained.
- Inbound voice is transcribed by the configured `media.stt` service; the default
  is Groq with `whisper-large-v3-turbo`. The transcript becomes the user entry; the
  original file is referenced. With no STT block, voice stays a file reference.
- Outbound files use the `message` tool: one path, or up to 10 compatible paths as
  an album. Captions are added only when explicitly requested.
- Voice replies use the configured `media.tts` service; the default is ElevenLabs
  with the voice and model from settings, returned as OGG/Opus. They are sent only
  when the user explicitly asks for voice. This is a behavioral rule in `SOUL.md`
  and a memory, not merely a transport feature.
- When `media.tts.kind` is `elevenlabs`, the speech tools tell the model to use
  sparse Eleven v4 audio tags in square brackets, for example
  `[warm, composed voice]` or `[softly]`. The tags are sent to ElevenLabs verbatim;
  no SSML is used.

### 8.5 No commands

Telegram has no slash commands. Every request is plain language, and the agent
handles it with tools or says why it cannot.

- A new chat starts a fresh conversation. There is no reset command.
- To stop a run, send `stop`. It aborts the active run and settles the submission as
  `cancelled`.
- Status, memory edits, reminders, and voice preferences are requests to the agent.
  Preferences are saved as rules.
- Compaction runs automatically at the thresholds in 5.5. There is no manual
  trigger.
- The TUI Overview shows the daemon state; the agent has no command-line tool.

### 8.6 When something fails

- Telegram outages are retried with a growing delay and randomness (exponential
  backoff with jitter). Polling resumes from the saved offset.
- If delivery is uncertain, the answer remains durable and is delivered as a
  recovered reply on the next interaction.
- If the model provider is down, the submission settles `unanswered` with a short
  notice and a retry hint. It is not retried forever.
- A per-chat queue cap prevents unbounded growth. Beyond the cap, Hearth sends one
  "busy" notice.

---

## 9. Tools

**In plain terms.** Tools are Hearth's hands. The everyday set is small enough to fit
in the prompt. Optional groups are loaded only when a job needs them.

### 9.1 The everyday tools

| Tool | Purpose |
|---|---|
| `memory` | Save, recall, forget, or read the transcript. |
| `message` | Send files, albums, voice to the user |
| `web_search` | Search the web |
| `web_fetch` | Fetch and extract a page or PDF |
| `files` | Read, write, edit, list within the workspace |
| `shell` | Run a command in the workspace |
| `schedule` | Reminders and recurring tasks |
| `skill` | Load, save, or archive a Markdown skill |
| `media` | Generate an image, transcribe audio, or synthesize speech |

`task` joins the core set only for a Telegram conversation below the helper depth
limit. `media` joins only when `media.image`, `media.stt`, or `media.tts` is
configured, so its schema costs nothing on an install that does not use it.

### 9.2 Capabilities that wait

`code`, `calendar`, `documents`, and `home` wait until there is evidence they are
needed. Hearth ships no browser and reads no office document. The Camofox client,
the documents tool, the tool-search group they lived in, and the lazy group loader
were removed: the browser was a separately installed service, and both compiled
clients advertised capabilities that most installs could not use. `web_fetch`
still extracts text from a PDF at a URL. Browser work belongs to whatever the user
installs and drives through `shell`.

### 9.3 Approval

Hearth ships yolo only: every tool runs without approval, and nothing is sandboxed.
That is a deliberate trust decision for a single-user local harness. Guarded mode,
with confirmation tiers and Telegram buttons, is deferred to 1.1.

### 9.4 The workspace and oversized output

- File tools are confined to configured roots (`workspace` by default). Links are
  resolved and checked after conversion to a real path, so a link cannot escape the
  folder.
- Shell starts in the workspace but is not a sandbox.
- Tool output above the inline cap is written to `spill/<id>.txt` under the temp
  spill root (`$TMPDIR/hearth/spill`, or `/tmp/hearth/spill` when `TMPDIR` is unset
  or empty). The model sees the size and the path, and can read more with `files`.
  The root is created at mode `0700` and may vanish at any time; a missing spill
  file reads as not found.
- A tool result carries its content and an error flag. The stable call id lives on
  the assistant entry that requested it, not on the result.

### 9.5 Web search and fetch

**In plain terms.** Search works out of the box through Exa, with no key. Brave,
DeepSeek, Firecrawl, and TinyFish are optional alternatives, tried in the configured
order. Fetch uses the same fallback idea: Jina Reader first, then a direct HTTP or
HTTPS fetch.

- The first entry in `web.search.order` is the main provider; later entries are
  fallbacks. A provider with no key is skipped. Exa needs no key.
- One call accepts one or more queries, runs at most `web.search.parallel` (3) at a
  time, requests `web.search.results` results each (5 by default), then dedupes by
  URL, merges, and returns at most `web.search.max_results` (10).
- Search results are title, URL, snippet, and date. There is no answer synthesis, no
  scraping, no cookies, and no MCP client beyond Exa's search endpoint.
- **Fetch order.** `web.fetch.order` defaults to `["jina", "http"]`. The allowed
  providers are `jina`, `http`, `https`, `tinyfish`, `firecrawl`, and `deepseek`.
  `http` is the one direct fetcher. Jina and `http` need no key; the other three
  are skipped without one.
- **Fetch providers.** Jina Reader returns readable Markdown. TinyFish, Firecrawl,
  and DeepSeek return extracted page text. The direct fetcher extracts HTML to text
  and PDFs to text, follows at most 3 redirects, caps the page at 2 MB, and refuses
  private and link-local addresses. Every fetch provider validates the target URL
  before it is used.
- **Untrusted content.** Fetch output is labeled as untrusted web content. The
  model treats it as data, never as instructions.
- If every search or fetch provider fails, the tool returns the first error and the
  call fails cleanly; it never silently returns nothing.

### 9.6 Skills and self-improvement

**In plain terms.** Hearth gets better by writing down procedures that worked,
revising them when they fail, and retiring them when they stop helping. It never
rewrites its own core.

Self-improvement has two outputs: memory rows for facts and preferences, and skills
for procedures. Nothing else changes itself. `SOUL.md`, settings, code, tools, and
the system prompt are off limits.

Memory and skills are separate loops that share evidence. Skill review reads recent
successful entries, the skill index and bodies, and relevant memory rows
(preferences, rules, prior lessons) to avoid conflicts and duplicates. Memory is
context, not proof: only entry ids justify a skill. The review writes skills only;
extraction and consolidation own memory writes. A skill's own output is never
evidence for its revision; only the recorded outcome of using it counts.

User-made skills live in `skills/`. Learned skills live in `skills/learned/`. Hearth
may read both and write only the learned directory. A user skill's load count
lives in `skills/.uses.json`, a hidden ledger, so the count survives a restart
without writing into a file the user owns. A learned skill is a Markdown
file with frontmatter:

| Field | Notes |
|---|---|
| `name`, `description` | Index entry; description is one line |
| `version` | Increases on every revision |
| `status` | `draft`, `active`, `quarantined`, `archived` |
| `created`, `updated`, `last_used` | Timestamps |
| `uses`, `successes`, `failures` | Outcome counters |
| `evidence` | Entry ids that justify the skill |

The body has three short sections: When to use, Steps, Pitfalls.

Lifecycle:

- **Creation.** A workflow that succeeded at least three times in 30 days, with no
  matching skill, becomes a `draft`. The user can also say "save this as a skill".
- **Promotion.** A draft becomes `active` after one successful use.
- **Revision.** Two consecutive failures, or a clearly better method in a later run,
  update the skill and bump its version. Every revision cites evidence.
- **Quarantine.** A skill that fails twice in a row becomes `quarantined`: removed
  from the index until revised.
- **Archive.** Skills unused for 90 days, or with a failure rate above half after four
  uses, are archived. Nothing is hard-deleted.

A nightly `skill.review` task at 04:30 in `identity.timezone` does the revision and archiving. It creates
zero skills when nothing qualifies, and up to three in one night when each one
independently passes the evidence threshold. Candidates beyond three are
reconsidered at the next review, so a productive day is not lost and the library
still grows in bounded steps.

It is a scheduled background task, not a helper and not part of a conversation. It
catches up on the next start if the machine was asleep, and it never blocks a user
turn. It sends no Telegram message; the result appears in the changelog and in
the TUI Overview as the last review time and change count.

Rules:

- Every creation or revision cites entry ids from a successful run. Instructions
  inside web pages or forwarded messages are not evidence.
- The `skill` tool can read any skill but rejects a save or archive outside
  `skills/learned/`. A user-made skill is read-only to Hearth.
- The index lists every active skill, up to 50 across both directories. When the
  count is over 50, archive the lowest success rate, then the least recently used.
- Every version is kept under `skills/learned/.history/`, and every change is
  appended to `skills/learned/CHANGELOG.md`.
- The TUI Skills tab exposes the library read-only: bodies and counts, with no
  key that edits, creates, or walks versions. A skill is read where it lives, and
  the library changes by editing the files or through Hearth's own skill loop.
  A skill with no counted use shows `(unused)` in the rate column; the column is
  eight characters wide and the label is padded to the widest row, so a
  percentage and `(unused)` end at the same edge. A rate appears only after a use
  has been counted. A user skill that has been loaded shows `used` and never a
  percentage: Hearth counts the load, and it cannot record an outcome for a file
  it does not write. `[` and `]` cycle the list through
  both scopes, user only, and learned only. The header names the mode, the empty
  state names the mode it is empty in, and the selection and body stay on a row
  that is still listed.
  The `skill` tool loads a body by name, records the use for either scope, records
  the outcome of a learned skill, and can save or archive a learned skill.
- The skills index is the only prompt component the skill loop writes. The Prompt
  tab shows it, and a loaded body appears in the transcript.

---

## 10. Settings

**In plain terms.** The settings file is small on purpose. It holds only what must
be personal: who the user is, how Telegram is reached, which models to use, and
which media services are configured. Everything else has a sensible default in the
code.

### 10.1 The whole file

```json
{
  "identity": {
    "agent_name": "Hearth",
    "timezone": "UTC"
  },
  "telegram": {
    "bot_token": "${HEARTH_TELEGRAM_BOT_TOKEN}",
    "allowed_user_ids": [123456789],
    "base_url": "https://api.telegram.org"
  },
  "models": {
    "kind": "openai-compatible",
    "base_url": "https://opencode.ai/inference/go/openai/v1",
    "api_key": "${OPENCODE_API_KEY}",
    "model": "deepseek-v4.1-flash",
    "context_window": 128000,
    "max_output_tokens": 32000,
    "session_header": "x-opencode-session"
  },
  "media": {
    "image": {
      "kind": "openrouter-images",
      "api_key": "${OPENROUTER_API_KEY}",
      "model": "openai/gpt-image-2.6-sunburst"
    },
    "stt": {
      "kind": "groq",
      "api_key": "${GROQ_API_KEY}",
      "model": "whisper-large-v3-turbo"
    },
    "tts": {
      "kind": "elevenlabs",
      "api_key": "${ELEVENLABS_API_KEY}",
      "voice_id": "VOICE_ID",
      "model": "eleven_v4",
      "output_format": "opus_48000_64"
    }
  },
  "tasks": {
    "max_concurrent": 4,
    "max_depth": 2,
    "retention_days": 30,
    "retention_runs": 25,
    "eta_reminder_seconds": 1
  },
  "web": {
    "search": {
      "order": ["exa", "brave", "deepseek", "firecrawl", "tinyfish"],
      "parallel": 3,
      "results": 5,
      "max_results": 10,
      "providers": {
        "exa": {},
        "brave": { "api_key": "${BRAVE_API_KEY}" },
        "deepseek": { "api_key": "${DEEPSEEK_API_KEY}" },
        "firecrawl": { "api_key": "${FIRECRAWL_API_KEY}" },
        "tinyfish": { "api_key": "${TINYFISH_API_KEY}" }
      }
    },
    "fetch": {
      "order": ["jina", "http"],
      "providers": {
        "jina": {},
        "tinyfish": { "api_key": "${TINYFISH_API_KEY}" },
        "firecrawl": { "api_key": "${FIRECRAWL_API_KEY}" },
        "deepseek": { "api_key": "${DEEPSEEK_API_KEY}" }
      }
    }
  }
}
```

That is a complete install with every optional block enabled. The minimum is
`identity`, `telegram`, and `models`. It uses `deepseek-v4.1-flash` on opencode-go;
change the model in one line. `SOUL.md` is always `$HEARTH_HOME/SOUL.md`.
`HEARTH.md` in the home holds rules and workspace guidance; it is created empty and
is not a JSON setting. Project `AGENTS.md` files are read from the filesystem root
down to the workspace. The workspace, polling, context budgets, memory budgets, and
logging all have defaults.

The three `media` blocks are optional and independent. Present means enabled. The
defaults are OpenRouter for images, Groq for transcription, and ElevenLabs for
speech. `kind` selects the wire protocol; `base_url` overrides the
provider endpoint when needed.

The `tasks` block is optional; the values shown are the defaults. The `web` block is
optional too. Exa is the zero-config default; the other providers are used only when
their key is present. In the TUI, search and fetch order are provider rows rather
than text fields: Enter turns a provider on or off, Shift+Up and Shift+Down move it,
and the last enabled provider cannot be turned off. Each keyed provider has a masked
API key row, so a key can be added without editing JSON.

Optional overrides, only when needed:

| Key | Default | Purpose |
|---|---|---|
| `agent.workspace` | `$HEARTH_HOME/workspace` | Tool sandbox |
| `compaction.idle_minutes` | `60` | Idle time before auto compaction |
| `compaction.idle_min_tokens` | `100000` | Minimum context for idle compaction |
| `compaction.soft_compact` | `0.65` | Soft compaction fraction of usable context |
| `compaction.hard_compact` | `0.85` | Hard compaction fraction of usable context |
| `compaction.keep_recent_tokens` | `20000` | Recent tokens kept through compaction |
| `models.context_window` | `128000` | Model context window in tokens; a local value that models.dev can refresh |
| `models.max_output_tokens` | `32000` | Local reply reserve; not sent |
| `tasks.eta_reminder_seconds` | `1` | Delay after a task’s expected finish before its reminder |
| `memory.rules_max_tokens` | `800` | Rules budget, in the cached prefix |
| `memory.recall_max_items` | `10` | Per-turn digest item cap |
| `memory.embeddings` | `true` | Request the local hybrid recall path; lexical fallback remains active |
| `memory.embedding_model_dir` | `models/bekko-embedding-v1-a8m` | Local embedding model directory |
| `media.image.model` | `openai/gpt-image-2.6-sunburst` | Image model |
| `media.stt.model` | `whisper-large-v3-turbo` | Transcription model |
| `media.tts.model` | `eleven_v4` | Speech model |
| `media.tts.voice_id` | none | Required for voice replies |
| `logging.level` | `info` | Diagnostics |

### 10.2 What is checked

- Unknown fields are rejected, to catch typos. `identity` has no `user_name`; an old
  settings file that still contains it is rejected. A settings file that still sets
  the retired `memory.recall_max_tokens` is refused, and the error names that line.
- Telegram is enabled when `telegram.bot_token` is present. Then
  `allowed_user_ids` must be non-empty. `telegram.base_url`, when present, must be
  an http or https URL.
- A models-only home can run the daemon. The nightly backup and skill review are
  scheduled without Telegram; reminders and helper workers need its transport.
- The `models` block requires `kind`, `base_url`, `api_key`, and `model`. `kind`
  must be `openai-compatible`. `models.context_window` and
  `models.max_output_tokens` must be positive.
- `compaction.soft_compact` must be greater than 0 and below
  `compaction.hard_compact`, which must be below 1. `compaction.keep_recent_tokens`
  must be positive.
- Each `media` block is optional and enabled only when present. A block missing
  `api_key` or `model` is rejected; `tts` also requires `voice_id`. When `tts.kind`
  is `elevenlabs`, the speech tools guide the model to use Eleven v4 audio tags.
- `media.*.kind` must be a supported kind, and `base_url`, when present, must use
  `https`.
- `web.search.order` must be a non-empty list of `exa`, `brave`, `deepseek`,
  `firecrawl`, or `tinyfish`; unknown names are rejected.
- `web.search.parallel` is 1 to 3, `web.search.results` is 1 to 20, and
  `web.search.max_results` is 1 to 50. A provider with no key is skipped, not an
  error.
- `web.fetch.order` must be a non-empty list of `jina`, `http`, `tinyfish`,
  `firecrawl`, or `deepseek`; unknown names are rejected. `http` is the one direct
  fetcher. A fetch provider with no key is skipped except `jina` and `http`.
  Provider names under `web.fetch.providers` follow the same list.
- `tasks.max_concurrent` is 1 to 8, `tasks.max_depth` is 1 or 2, and the retention
  values are positive. `tasks.eta_reminder_seconds` is a non-negative delay after the
  expected finish; the default is 1.
- `memory.rules_max_tokens` is 50 to 4,000 and `memory.recall_max_items` is 1 to
  64. A value outside the range is a settings error, not a silent clamp.
  `logging.level` sets the logger's maximum level, and
  `compaction.idle_minutes` and `compaction.idle_min_tokens` drive the idle pass.
- `identity.timezone` is an IANA zone name. Every timestamp Hearth shows the person
  or the model is ISO 8601 with the offset in force at that instant, such as
  `2026-02-14T09:41:05+08:00`, and the nightly backup and skill-review slots are
  wall-clock times in that zone. A change applies without a restart. Stored
  instants stay UTC, so the record does not shift when the preference changes.
- `models.session_header` is optional and names a header sent with the conversation
  id, for gateways that route by session.
- `models.thinking` is optional and is one of `default`, `off`, `low`, `medium`,
  `high`, `xhigh`, or `max`. `default` leaves the provider's own thinking behavior
  unchanged. `low`, `medium`, `high`, `xhigh`, and `max` become
  `reasoning_effort`; `off` becomes `reasoning_effort: none`, except on DeepSeek
  URLs, which use the `thinking` toggle instead.
- `daemon.start` is `tui` (start when Hearth opens), `login` (systemd user unit at
  login), or `startup` (user unit plus linger at boot). `daemon.keep_running`
  controls whether a TUI-started daemon survives the window closing. `login` and
  `startup` own the unit file and its enable link, and a deleted pair is recreated
  on the next daemon pass or TUI start.
- Missing sections take defaults, versioned with the binary. An empty file parses but
  cannot run a conversation until `models` is defined.
- Inline secrets are permitted, but environment references are recommended.
- Logs, error messages, the TUI views, and the audit trail redact anything matching
  configured secret patterns and any configured credential value of 8 or more
  characters. Shorter values are placeholders and are left alone.

---

## 11. Storage

**In plain terms.** One SQLite file (a single-file database built into the program)
holds the durable record. Media lives beside it as ordinary files. The database is
ordinary, inspectable, and backed up by copying it in a consistent way.

### 11.1 The database

- One writer connection on a dedicated thread; four read-only connections.
- `journal_mode=WAL` (write-ahead log, which lets reads continue during a write),
  `foreign_keys=ON`, `busy_timeout=5000`, `synchronous=FULL` by default,
  `auto_vacuum=INCREMENTAL`.
- Migrations (schema changes) are versioned and transactional, tracked with
  `PRAGMA user_version`.
- All JSON is validated before storage. There are no BLOBs (raw byte values) except
  small thumbnails. Media lives on disk.

### 11.2 Tables

The exact schema is in `SCHEMA.md`. The tables are `meta`, `commits`,
`conversations`, `entries`, `prompt_blobs`, `submissions`, `tasks`,
`effects`, `usage_events`, `telegram_updates`, `memories`, `memories_fts`, and
`memory_vectors`.

### 11.3 Backups and retention

- Nightly `VACUUM INTO backups/hearth-YYYYMMDD.db`; keep 7 daily and 4 weekly. Memory lives in that database, so the snapshot is the whole store. The date, and the 03:00 slot that takes it, are local to `identity.timezone`.
- Media is operator managed. Hearth never deletes a file under `media/`; an
  operator who wants a smaller backup removes old inbound and outbound files by
  hand.
- The Doctor tab verifies integrity, schema version, lock state, settings, and
  provider credentials without printing secrets.

### 11.4 Install and service

One binary at `/usr/local/bin/hearth`. The systemd user unit named
`hearth.service` sets `HEARTH_DAEMON=1` and `HEARTH_HOME`, runs the binary with
no arguments, and restarts on failure. The TUI Settings tab installs or removes
the unit and applies the login or startup mode; startup uses
`loginctl enable-linger`. Release binaries cover Linux x86_64 and aarch64 with
published SHA-256 checksums, so no Rust toolchain is needed.

An installed unit is two files under `~/.config/systemd/user`: the unit and the
`default.target.wants/hearth.service` link that makes it start at login. While
`daemon.start` is `login` or `startup`, the daemon checks both at start and every
60 seconds, and the TUI start path checks them before it starts anything. Repair
is additive: it creates a missing unit or link and never rewrites an existing unit
or deletes anything. It writes files only and does not call `systemctl`, because
systemd loads a unit on demand and reads the enable link when `default.target`
comes up.

---

# Part 3: Trust, Cost, and Quality

## 12. Safety and Privacy

**In plain terms.** One user, locked files, no secrets in memory, no telemetry, and
no approval prompts. The user trades approvals for speed. The rest of the boundary
still holds.

- Single-user allowlist. Authorization fails closed when it is missing.
- `settings.json` and the socket are mode `0600` (user-only). The database is
  `0600`.
- Secrets never enter prompts, memory, logs, or the transcript. Tool output is
  scanned for credential patterns before it is saved where feasible.
- Untrusted content (web pages, files, forwarded messages) is labeled as untrusted
  in the prompt. Instructions found inside tool output are data, not commands. The
  label is advisory; the user accepts the risk.
- Hearth runs yolo only: no approval prompts and no sandbox. Guarded mode is
  deferred to 1.1.
- Network access comes only from providers and tools. No telemetry is sent by
  default.
- Optional encryption at rest is deferred. When added, it uses SQLCipher (an
  encrypted version of SQLite) or an application-level key from the operating system
  keyring, never a key stored beside the database.
- An audit log records effect outcomes, memory deletions, and configuration changes.
  It never records message bodies by default.

---

## 13. Watching the Cost

**In plain terms.** Every model call is metered by tokens. The user can see input,
output, cache, and latency totals.

- Structured JSON logs with timing records for each submission, run, turn, task
  step, provider request, and tool call. Message content is off by default; ids and
  token counts are on.
- The usage ledger records input, cache read, cache write, output, reasoning, and
  latency for every model request. Media calls (image, transcription, speech) are
  recorded in the same ledger.
- The Overview tab combines runtime (daemon, model, thinking, provider, context,
  workspace), usage and cache hit rate for today, counted from local
  midnight in `identity.timezone`, the last week, the last month, and all time,
  work and memory (tasks, helpers, statements, rules
  bullets, budgets, maintenance, active skills and the index cost), provider
  connections, and recent activity. Each usage period shows input tokens, output tokens, and cache reads.
- The Activity tab follows a live run or task; logs and the usage ledger keep the
  durable record.
- The usage ledger aggregates by day, model, and task kind. It does not carry a
  per-tool column, so tool-level totals are not claimed.

---

## 14. Speed and Size

**In plain terms.** The targets below describe a system that feels instant on a
small server and stays small after years of use. They are measured, not aspirational.

Reference host: 4 CPU cores, 8 GB RAM, SSD.

| Metric | Target | What it means |
|---|---|---|
| Idle memory use | 120 MB or less | The daemon is quiet when nothing is happening. |
| Startup and recovery, 100k entries | 500 ms or less | Restarting does not feel like an outage. |
| Save time, 95th percentile | 15 ms or less | Most saves finish before a person notices. |
| Memory retrieval, 4,200 items | 100 ms or less | The measured gate. Recall is fast on a personal store; this is the number the benchmark suite gates. |
| Memory retrieval, 200,000 items | measured, no gate | 9.1 ms at p50, 221.3 ms at p95, and 937.3 ms at p99 on the reference host with 200,000 distinct subjects and the 195 fixture questions. Measured by `the_200k_store_is_measured`. The p95 is driven by questions that share a word with the noise; the 100 ms gate stays on the 4,200-row store. The hybrid vector scan at 200,000 rows is not measured: linear extrapolation from about 10 ms at 4,000 rows gives about 500 ms, so the 100 ms gate is not claimed there. |
| Time to first token, excluding provider | 300 ms or less | Hearth's own overhead is small. |
| Prompt cache hit, consecutive turns | 80% or more of prefix tokens | The provider reuses most of the stable prefix. |
| Annual database size | 2 GB or less | Years of personal use fit on a small disk. |

"95th percentile" means 95 out of 100 measurements are at or below the number.

---

## 15. Building It Test-First

**In plain terms.** Every behavior starts as a test that fails for the right reason.
Then comes the smallest code that makes it pass. Then the next small step. This is
called test-driven development, or TDD. It is not a coverage target. It is how the
work is done.

TDD here means a red-green loop at pre-agreed seams: write a failing test (red),
make it pass (green), repeat. The rules are requirements for every crate and every
milestone. Tests decide when behavior is correct; YAGNI, KISS, and DRY (15.5)
decide how much code is allowed to exist.

### 15.1 The loop

1. **Red before green.** Write one failing test, watch it fail for the expected
   reason, then write only enough code to pass it. No production code without a
   failing test that demanded it.
2. **One vertical slice at a time.** One vertical slice, meaning one small,
   complete step at one seam, one test, one minimal implementation, repeat. Writing
   a batch of tests before the implementation is forbidden. Bulk tests verify
   imagined behavior and harden the wrong shape.
3. **No speculative features.** Do not anticipate future tests. The next test is
   chosen after the last cycle teaches something.
4. **Bug fixes start with a reproduction.** The failing test is committed to
   version control before the fix, and it fails for the reported reason.
5. **Refactoring is not part of the loop.** Refactoring, meaning tidying code
   without changing behavior, happens after green, under review, with tests passing
   before and after. A refactor change does not change behavior.
6. **Tests ship with the code.** A slice is not done until its test is in the same
   change. "Tests later" is not a valid state.
7. **Spikes are disposable.** Spikes, meaning throwaway experiments, may skip
   tests, but they are deleted after they answer their question, and the real
   implementation is redone test-first.
8. **Definition of done.** A behavior is done when its seam test fails without the
   implementation, passes with it, and the full suite is green.

### 15.2 The seams

**In plain terms.** A seam is a public boundary where a test can see behavior
without reaching into private code. Testing only at agreed seams is how effort lands
on critical paths instead of every internal corner.

The seams below are approved for M1, the durable-core milestone. Adding or removing
a seam requires the user's approval. Each seam lists what it catches and what it
deliberately misses.

**What each seam covers, in everyday terms.** S1 checks that saves, submissions,
and resumes behave. S2 checks that both storage backends behave the same. S3 checks
translation to and from providers. S4 checks memory. S5 checks Telegram. S6 checks
a whole answer end to end. S7 checks crash recovery. S8 checks the commands.

| Seam | Public boundary | Catches | Misses |
|---|---|---|---|
| S1 Core session | `Harness` and `Session` API: save, submit, resume | Atomicity, visibility, submission endings, turn ordering | Rendering, provider behavior |
| S2 Storage contract | `Storage` trait with one conformance suite over memory and SQLite | Backend divergence, migrations, idempotent replay, sequence monotonicity | Scheduler policy, UI |
| S3 Provider adapter | `Provider` trait driven by recorded streaming fixtures and a fake HTTP server | Protocol errors, tool-call assembly, retries, usage mapping, cache hints | Live provider quirks, real latency |
| S4 Memory and skills | `Memory` and `Skills` APIs: save, search, update, forget, consolidate; skill lifecycle | Retrieval ranking, provenance, supersession, budget, decay, promotion, quarantine | Human relevance judgment |
| S5 Telegram | Bot API client against a fake server | Authorization, update dedupe, offset persistence, chunking, attachments | Live rate limits, real Telegram edge cases |
| S6 Agent loop | End-to-end run with a fake provider, real core, temp SQLite | Tool loops, compaction, budget enforcement, recall injection, unbounded loop until cancel | Model answer quality |
| S7 Crash recovery | The daemon runs as a separate process and is killed at named failure points | Durability, effect retry policy, no duplicate ending | Performance, non-crash concurrency |
| S8 Operator surface | `hearth` binary: config validation, doctor, backup, watch | Settings errors, migration safety, command contracts | Internal design |

S1 through S8 are the approved set. They are the only test seams. Internal modules are exercised through them. The storage
conformance suite is the one place where direct database inspection is allowed, and
only for named invariants such as save atomicity, tombstones, and sequence gaps.

### 15.3 What a good test is

- **Behavior names.** Name the capability, not the function, in the vocabulary of
  the glossary.
- **Independent expectations.** Expected values come from the spec or a known-good
  literal, never recomputed the way the implementation does.
- **Integration style.** Real SQLite, scheduler, and renderers. Mock only system
  boundaries: model HTTP, Telegram HTTP, media endpoints, clock, and randomness.
- **No internal mocks.** No assertions on Hearth's own call counts or order.
- **One logical assertion.** Several fields of one outcome are fine; two behaviors
  are not.
- **Determinism.** Inject the clock and ID generator, seed randomness, and pause
  Tokio time. Tests never sleep for a scheduler.
- **Golden files and fixtures.** Goldens are reviewed diffs for rendering, prompts,
  and budgeting. Recorded fixtures replay streaming, Bot API responses, and provider
  failures.

### 15.4 Enforcement

- `cargo test --workspace` is the default gate. `cargo nextest` is allowed, but the
  workspace suite remains authoritative.
- CI runs formatting, `clippy -D warnings`, the full test suite, the short crash
  injection set, and a memory evaluation smoke test on every change.
- Coverage is informational only. Seam coverage is reviewed at each milestone. A
  milestone does not close because a percentage was met.
- A change that alters behavior without a corresponding test change is rejected,
  unless it is a pure refactor with an unchanged test suite.
- Code review checks that tests are at seams, that expectations are independent, and
  that no test broke because of an internal rename.
- Code review also rejects speculative generality, unused abstractions, and copied
  rules, in line with 15.5. YAGNI, KISS, and DRY are judged against the current
  milestone, not an imagined later one.

### 15.5 YAGNI, KISS, and DRY

**In plain terms.** The test loop says when the code is right. These three rules
say how much code should exist. They are requirements, not preferences, and they
are checked in review alongside the tests.

- **YAGNI (you aren't going to need it).** Build what the current milestone and
  its failing tests require. No speculative settings, traits, hooks, plugin points,
  cache layers, or abstractions for later work. That a later section of this spec
  describes something makes it scheduled work, not an invitation to pre-build it.
  Delete dead code and unused fields instead of keeping them for later.
- **KISS (keep it simple).** Choose the smallest, most direct design that passes
  the test and is understandable without explanation. Prefer one obvious path over
  a configurable one. A new dependency, runtime indirection, or macro needs
  justification. Cleverness a reviewer must decode is a defect.
- **DRY (don't repeat yourself).** One authoritative definition for each rule,
  schema, constant, and behavior. Extract shared logic when the second real use
  appears, not before. Duplicated tests and golden files are duplication too. DRY
  does not mean merging things that merely look alike: two behaviors that change
  for different reasons stay separate.

**How the three fit the loop.** YAGNI chooses the test, KISS chooses the
implementation, and DRY tidies up after green. A refactor that removes duplication
is allowed only with the full suite passing before and after (15.1). When the rules
conflict, correctness and clarity win: a little repetition is cheaper than a
premature abstraction, and a simple special case is cheaper than a general
mechanism nobody asked for.

---

## 16. Test Suites and Release

**In plain terms.** Section 15 defines how code is written and tested. This section
defines the suites that must pass before release.

### 16.1 Test suites

- Unit and property tests for records, budgets, parsers, renderers, save atomicity,
  sequence monotonicity, and idempotent submissions.
- Integration tests with a fake provider that replays streaming, tool calls, errors,
  rate limits, and overflows; Telegram tests against a local fake Bot API.
- Crash tests: a separate process is killed at `before_commit`, `after_commit`,
  and `mid_stream` (`HEARTH_CRASH_AT`).
- Memory benchmarks from 7.8, and a 7-day soak with restarts and backup/restore.

### 16.2 Release acceptance for 1.0

1. A kill at every named failure point loses no admitted input and duplicates no
   settled answer.
2. Telegram single-user flow works end to end: text, image, document, voice input,
   and files out.
3. A scheduled reminder survives a restart and fires once.
4. The memory benchmark meets the targets in 7.8.
5. Prompt cache hit and fixed overhead meet section 14.
6. The Doctor tab and the backup/restore checks pass on a clean host.
7. No secret appears in logs, memory, or transcripts across the adversarial test
   suite.

---

## 17. Milestones

**In plain terms.** Each milestone produces something usable and testable. Memory
comes before broad tool polish, because a personal agent is only useful when it
remembers and only affordable when memory carries the load.

| Milestone | Scope | What you get | Exit criteria |
|---|---|---|---|
| M0 Foundation | Workspace, config, logging, SQLite migrations, lock, command-line skeleton, test harness | A project that builds, validates settings, and has its first seam tests | First seam tests pass red-green; `hearth doctor` passes; invalid settings rejected |
| M1 Durable core | Entries, saves, tasks, submissions, recovery; fake provider | Work that survives a kill | Crash recovery suite passes |
| M2 Agent loop | Provider, streaming, tool loop, files, shell, web, message, local chat, live watch | A local agent that can answer and use tools | End-to-end answer with tools; usage ledger accurate; a tool-using run can be watched live |
| M3 Telegram | Polling, authorization, dedupe, attachments, final replies | The agent in your pocket | Single-user end to end; replay causes no duplicates |
| M4 Memory v1 | Standing memory, full-text search, tools, extraction, provenance, consolidation skeleton | An agent that remembers the basics | Recall evaluation baseline; no secret stores |
| M5 Token efficiency | Budgets, lazy tools, spill, prompt cache, compaction, memory flush | An agent that stays cheap | Section 14 targets on a 100-turn script |
| M6 Memory v2 | Better retrieval, diversity, temporal validity, episodes, decay | An agent that remembers well for years | Section 7.8 targets |
| M7 Background work | Scheduler, reminders, helpers, skills, notifications, skill review | An agent that works while you are away | Restart-safe reminder; a skill is created, promoted, revised, and archived across restarts |
| M8 Hardening | Backup/restore, docs, packaging, soak | A release you can trust | 1.0 acceptance list |

Every milestone exit includes its seam tests, written red-first. A milestone that
ships behavior without tests at an approved seam is not complete, regardless of
manual demos.

---

## 18. Open Questions

None. The 1.0 scope is fixed. A scope change needs a new draft.

---

## 19. Public Release

**In plain terms.** Hearth is published as source and binaries, and the default
install contains nothing personal.

- **No personal data in the tree.** Names, usernames, home paths, Telegram ids, chat
  ids, API keys, tokens, databases, logs, media, backups, and memory exports never
  enter the repository, examples, fixtures, screenshots, or commit history. Examples
  use `USER_NAME`, `123456789`, `~/.hearth`, and `example.com`.
- **Clean history.** The public repository starts from a history that has passed the
  scan; a private history is never published.
- **CI deny-list.** A required check fails on any private string or file. The list
  itself is kept outside the public tree, so it does not become the leak. GitHub
  withholds repository secrets from a fork pull request, so that run fails instead
  of reporting clean when the list is absent, and a fork run checks credential
  shapes only with the omission printed in the job log. The maintainer scan before
  merge covers the private strings.
- **Neutral defaults.** The default agent name is `Hearth`, and the default `SOUL.md`
  is a short generic template with Identity, Voice, and Rules.
- **License and docs.** MIT license, README, QUICKSTART with the BotFather steps and
  troubleshooting, SECURITY.md, and CONTRIBUTING.md.

---

## Appendix A. Defaults at a Glance

| Setting | Default | Where it is described |
|---|---|---|
| Home folder | `~/.hearth` | 3.1 |
| Settings file mode | `0600` | 3.2 |
| Database mode | `0600` | 11.1 |
| Save durability | `synchronous=FULL` | 4.4 |
| Read connections | 4 | 11.1 |
| Recall bound | 10 items of 200 characters | 7.4 |
| Context window | 128,000 tokens | 6.2 |
| Reserved reply | 32,000 tokens local reserve | 6.2 |
| Soft compaction | 65% of usable context | 5.5 |
| Hard compaction | 85% of usable context | 5.5 |
| Keep recent tokens | 20,000 | 5.5 |
| Inline tool output cap | 4,000 tokens | 6.3 |
| Telegram message limit | 32,768 UTF-8 characters rich; 4,096 fallback | 8.3 |
| Telegram download limit | 20 MB | 8.4 |
| Image long edge | 1,568 px | 8.4 |
| Image model | `openai/gpt-image-2.6-sunburst` | 10.1 |
| Transcription model | `whisper-large-v3-turbo` | 10.1 |
| Speech model | `eleven_v4`, OGG/Opus 48 kHz | 10.1 |
| Helpers at once | 4 | 5.4 |
| Helper depth | 2 | 5.4 |
| Helper retention | 30 days or newest 25 runs | 5.4 |
| ETA reminder | 1 second after expectation | 5.4 |
| Core tool schemas | 8 tools, 1,294 tokens, or 1,459 with media | 9.1 |
| Skills index | 50 active | 9.6 |
| Skill review | 04:30 in `identity.timezone`, 0 to 3 new skills | 9.6 |
| Learned skills | `skills/learned/` | 9.6 |
| Skill archive | 90 days unused | 9.6 |
| Search concurrency | 3 queries at a time | 9.5 |
| Search results per query | 5 | 9.5 |
| Search results returned | 10 at most | 9.5 |
| Search order | Exa, then Brave, DeepSeek, Firecrawl, TinyFish | 9.5 |
| Fetch order | Jina, then direct HTTP/HTTPS | 9.5 |
| Backups | 7 daily, 4 weekly | 11.3 |
| Idle compaction timer | 60 minutes | 5.5 |
| Idle compaction threshold | 100k tokens or 65% of usable | 5.5 |
| Idle memory target | 120 MB | 14 |
| Annual database target | 2 GB | 14 |
