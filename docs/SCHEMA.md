# Hearth Schema Reference

Companion to `SPEC.md`. The spec defines behavior and invariants; this file holds the
exact columns, tables, and indexes. When the two disagree, the spec wins. The
memory store is documented below and in `SPEC.md` 7.

## Field tables

IDs are time-ordered UUIDv7 (unique identifiers that sort by creation time) unless
noted. Times are stored in UTC microseconds and displayed in the configured
timezone.

**Conversation.** One chat and its metadata. A conversation can be forked at a
chosen entry to start a related branch.

| Field | Notes |
|---|---|
| `id` | UUIDv7 |
| `parent_id`, `parent_entry_id` | Where a copied conversation started, nullable |
| `title` | Short, generated or user-set |
| `transport`, `transport_key` | `telegram` plus `chat_id:thread_id`, or `local` |
| `created_at`, `updated_at` | |
| `archived` | Boolean |

**Entry.** One permanent line in a conversation.

| Field | Notes |
|---|---|
| `id` | UUIDv7 |
| `conversation_id` | |
| `seq` | Save sequence, used for ordering |
| `kind` | `user`, `assistant`, `tool_result`, `system`, `summary`, `reset`, `notice` |
| `content` | JSON blocks: text, image reference, tool call, tool result, thinking |
| `tokens_est` | Estimate used for budgeting |
| `source` | Telegram message id, task id, or external reference |
| `prompt_hash` | Optional reference to `prompt_blobs` |
| `created_at` | |

Entries are append-only. A reset is an entry, not a deletion. The model sees the
newest reset or summary marker onward, subject to compaction.

**Document.** A small mutable record attached to a conversation.

| Field | Notes |
|---|---|
| `conversation_id`, `kind` | Primary key |
| `version` | Integer, incremented on every write |
| `json` | Valid JSON, no extensions |
| `updated_at` | |

Built-in document kinds:

- `agent`: model alias, thinking level, tool groups, extra instructions, workspace.
- `provider`: provider session identity, used for prompt-cache affinity.
- `live`: current run and task id, for status display only.
- `usage`: rolling counters, not the authoritative usage ledger.
- `last_consolidation`, `last_skill_review`: maintenance bookkeeping.
- `transport`: transport-channel state that is not in its own table.

A document update inside a save must supply the version it expects. A mismatch
aborts the save; the caller re-reads and retries.

**Submission.** A message handed in, with a receipt.

| Field | Notes |
|---|---|
| `id` | UUIDv7 |
| `conversation_id` | |
| `request_id` | Unique idempotency key |
| `kind` | `input` or `entry` |
| `content` | User input blocks |
| `status` | `queued`, `running`, `done`, `unanswered`, `cancelled` |
| `answer_entry_id` | Nullable |
| `error` | Structured, nullable |
| `created_at`, `settled_at` | |

A duplicate `request_id` returns the original submission and never creates a second
run. Telegram uses `tg:<chat_id>:<message_id>`. Background jobs use their own stable
keys.

**Task.** A piece of work that can pause and resume.

| Field | Notes |
|---|---|
| `id` | UUIDv7 |
| `conversation_id`, `parent_task_id` | |
| `kind`, `version` | For example `generation`, `tool`, `compaction` |
| `status` | `pending`, `running`, `asleep`, `done`, `failed`, `aborted` |
| `phase` | Current step name |
| `attempt` | Attempt counter for the current step |
| `input` | JSON, immutable |
| `checkpoint` | JSON, written with each transition |
| `result` | JSON, terminal |
| `lease_owner`, `lease_until` | Which process is working on it, and until when |
| `next_wake_at` | For sleeps |
| `created_at`, `updated_at`, `terminal_at` | |

**Effect.** A recorded action in the real world.

| Field | Notes |
|---|---|
| `id` | UUIDv7 |
| `task_id` | |
| `kind` | `telegram.send`, `http`, `shell`, `tts`, and so on |
| `key` | Idempotency key, unique per kind |
| `payload` | JSON |
| `status` | `intent`, `started`, `done`, `failed`, `unknown` |
| `result` | JSON, for example a Telegram message id |
| `attempts`, `created_at`, `settled_at` | |

## SQL schema

- `meta`: small bookkeeping values.
- `commits`: the save log and sequence.
- `conversations`: chats and their metadata.
- `entries`: the transcript.
- `prompt_blobs`: full prompt text, deduplicated by hash, for debugging.
- `entries_fts`: FTS5 index over `entries.content_json` for history search.
- `documents`: small versioned JSON state per conversation.
- `submissions`: messages handed in and their endings.
- `tasks`: resumable work.
- `effects`: outside actions and their outcomes.
- `usage_events`: every model call, its input, cache read, cache write, output, and
  reasoning tokens, and latency.
- `telegram_updates`: deduplication of incoming updates.
- `memories`, `memories_fts`, `memory_vectors`: the memory store, documented
  below.

The exact DDL is the versioned set under `crates/hearth-storage/migrations/`
(`0001.sql` through `0010.sql`), applied by the matching migrations. The
migrations are the executable schema authority; if this document and a migration
disagree, the migration wins. `effects` has a `UNIQUE (kind, key)` constraint.
Migration 0002 rebuilds the old memory index with the porter stemmer. Migration
0005 drops the unused `usage_events.cost_micros` column. Migration 0007 adds
`entries_fts` and rebuilds it for existing transcripts. Migration 0008 replaces
both earlier memory designs with `memories`, `memories_fts`, and
`memory_vectors`, and drops `memory_items`, `memory_events`, and the old
`memory_fts`. Migration 0009 clears the pin on rows an older build retired
without clearing it. Migration 0010 indexes the subject list in its longest-first
read order, so the recall bound does not sort the store on every turn.

## Indexes

`entries(conversation_id, seq)`, `tasks(status, next_wake_at)`,
`submissions(status)`, `usage_events(ts)`, `effects(status)`, and the memory
indexes listed below.

## Memory schema

Memory is a table in `hearth.db`, so a statement shares one writer, one
transaction boundary, and one backup with the entry it came from. There is no
second database file and no markdown store.

`memories`:

| Field | Notes |
|---|---|
| `rowid` | Integer primary key, also the FTS content rowid |
| `id` | `m_` plus a UUID, unique |
| `kind` | `rule`, `preference`, `person`, `decision`, `commitment`, `episode`, `fact` |
| `content` | One statement |
| `subject` | Normalised entity, empty when the statement names none |
| `terms` | Index expansion: head words plus the relation groups they belong to |
| `fingerprint` | Digest of the normalised content, for exact-duplicate refusal |
| `pinned` | Injects the statement as a rule; only with `kind = rule`, and only while `valid_to` is null |
| `source_kind`, `source_entry` | Provenance |
| `valid_from`, `valid_to` | The window the statement was true; `valid_to` is null while it is current |
| `superseded_by` | The statement that closed this window |
| `created_at`, `updated_at`, `revision` | Bookkeeping |

Indexes: `memories(subject, valid_to)` for the entity channel,
`memories(kind, valid_to)`, `memories(fingerprint)`, a partial
`memories(pinned) WHERE pinned = 1` for the rules block, and a partial
`memories(LENGTH(subject) DESC, subject) WHERE subject <> ''` for the bounded,
longest-first subject list.

`memories_fts`: FTS5 over `content`, `subject`, and `terms`, content-linked to
`memories`, with Porter stemming, diacritic removal, and two- and three-character
prefix indexes. Triggers keep it in step on insert, delete, and an update of an
indexed column only.

`memory_vectors`: one row per statement, holding `memory_id`, `model`, `dims`,
and an INT8 vector stored unit length, so cosine similarity is a dot product with
no per-row scale. A model or dimension change
invalidates the set; the daemon backfills missing vectors off the reply path.

Nightly backups are the single `hearth-YYYYMMDD.db` snapshot, because memory is
inside it.
