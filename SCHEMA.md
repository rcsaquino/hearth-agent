# Hearth Schema Reference

Companion to `SPEC.md`. The spec defines behavior and invariants; this file holds the
exact columns, tables, and indexes. When the two disagree, the spec wins. Memory row
fields are defined in `SPEC.md` 7.2.

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
- `memory_state`: standing-memory version and last consolidation time.
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
| `kind`, `version` | For example `generation`, `tool`, `compaction`, `memory.extract` |
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
| `kind` | `telegram.send`, `http`, `shell`, `file_write`, `tts`, and so on |
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
- `documents`: small versioned JSON state per conversation.
- `submissions`: messages handed in and their endings.
- `tasks`: resumable work.
- `effects`: outside actions and their outcomes.
- `usage_events`: every model call, its input, cache read, cache write, output, and
  reasoning tokens, latency, and cost.
- `telegram_updates`: deduplication of incoming updates.
- `memory_items`, `memory_fts`, `memory_events`:
  the memory library.

The exact DDL is the versioned set under `crates/hearth-storage/migrations/`
(`0001.sql`, `0002.sql`, `0003.sql`), applied by the matching migrations. The
migrations are the executable schema authority; if this document and a migration
disagree, the migration wins. `memory_items` also has a `fingerprint` column and
index for exact-duplicate detection, and `effects` has a `UNIQUE (kind, key)`
constraint. Migration 0002 rebuilds `memory_fts` with the porter stemmer. Migration 0003 adds
partial indexes for active standing rows and promotable candidates.

## Indexes

`entries(conversation_id, seq)`, `tasks(status, next_wake_at)`, `submissions(status)`,
`memory_items(status, kind)`, `usage_events(ts)`, `effects(status)`, plus the
partial `memory_items_active_standing` and `memory_items_promotable` indexes.
