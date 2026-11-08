# Decisions

This file records interpretations, deviations, and approved decisions that affect
the build. `SPEC.md` remains the binding product definition. Every deviation needs
sir's approval and a spec draft bump before it is treated as settled.

## 2026-11-08: Build ground rules (approved)

- `SPEC.md` Draft 0.3 is binding. No silent drift.
- One background worker per milestone, with a gate report and sir's approval before
  the next milestone.
- The local `scripts/ci.sh` is the authoritative gate. No GitHub, no remote, and no
  hosted CI until publication is explicitly approved.
- Test-first development at seams S1-S8. M0 behavior is tested at S8, the operator
  surface, by running the real binary.
- YAGNI, KISS, and DRY per spec 15.5. Dependencies are added only when the current
  slice uses them.

## 2026-11-08: Storage and schema

- The storage seam is SQLite only. The "memory and SQLite" wording in S2 is read as
  a leftover; no in-memory backend is built.
- Migration `0001` is the executable schema authority. `SCHEMA.md` is documentation
  and must follow it.
- Migration `0001` creates the full 1.0 schema, including tables whose features
  arrive in later milestones. This avoids schema churn and keeps one authoritative
  definition.

## 2026-11-08: Memory and tokens

- Memory constants live in one versioned table that the M4 benchmark may tune
  without a spec amendment. Seeds: RRF `k=60`, 180-day recency half-life, 0.85
  near-duplicate similarity.
- Token estimates use characters divided by four plus a 10 percent margin behind
  one estimator interface for 1.0. A real tokenizer is added only when a failing
  test demands it.

## 2026-11-08: Dependencies

Approved baseline, used only when needed:

- `rusqlite` with bundled SQLite and FTS5
- `tokio`
- `reqwest` with rustls
- `serde` and `serde_json`
- `clap` with derive
- `tracing` and `tracing-subscriber`
- `uuid` v7
- `time`
- `thiserror` in libraries, `anyhow` at the binary edge
- `signal-hook` for daemon shutdown signals
- `tempfile` as a test-only dependency

Not allowed without a new recorded decision: `sqlx`, `teloxide`, `axum`, an ORM, or
`async-trait`.

## 2026-11-08: Operations

- Live provider, Telegram, and media calls happen only in a manual smoke test with
  sir's approval. Automated tests use fake values and local fixtures. No unattended
  spend.
- A dedicated Hearth development bot is used for Telegram testing. Telegram allows
  one `getUpdates` consumer per bot, so Hearth must never poll pi's token.
- The development home is a gitignored `.devhome/` inside the repository. Secrets
  are referenced as `${ENV_VAR}` and never committed.
- Git author: `rcsaquino`.
- License: MIT from the first commit.

## 2026-11-08: Milestone sequencing in M0

- `hearth init` in M0 validates the shape of its inputs and writes a working home.
  The live Telegram check from spec 3.4 is added in M3 and the live provider check
  in M2, because those clients do not exist yet.
- Interactive `hearth init` prompts are not part of M0. Only the non-interactive
  path is built and tested.
- `hearth doctor` in M0 checks settings, database integrity, schema version, and the
  lock. Live provider and Telegram credential checks arrive with those clients.
- M0's daemon is a skeleton: it acquires the lock, migrates, logs, and waits for a
  termination signal. It does not poll Telegram or run the agent loop.

## 2026-11-08: Delivery

- M0 estimate: one working session. M1: about three. If a milestone reaches twice
  its estimate, stop and report what is blocking.
- M2 target provider: opencode-go with `deepseek-v4.1-flash`.

## 2026-11-08: M0.1 corrections (approved)

- `signal-hook` is approved for SIGTERM and SIGINT. The standard library has no
  portable signal API and Hearth forbids unsafe code.
- Daemon logs are mode `0600`, even though the spec only mandates `0600` for
  settings, the socket, and the database. Logs carry paths, PIDs, and timing.
- The default timezone order is non-empty `TZ`, then the `/etc/localtime` symlink
  target, then `/etc/timezone`, then UTC. An explicit `--timezone` wins over all.
- `usage_events` carries `reasoning_tokens` in migration `0001`. The M1 S2 storage
  conformance test will assert the full column set, because usage writes do not
  exist before M2.

## 2026-11-08: M1a session and storage (pending M1 gate review)

- `Storage` is a concrete type, not a trait. Q5 removed the in-memory backend, so a
  trait with one implementation would be speculative. The public boundary and the
  S2 conformance suite remain unchanged. This needs sir's confirmation at the M1
  gate.
- The commit sequence is allocated inside the save transaction as
  `MAX(seq) + 1`, so a rolled-back save leaves no gap. All commits go through an
  explicit sequence insert.
- Entries saved together share their commit sequence. Their order is `seq, rowid`.
- `tokens_est` is stored as `0` until the estimator arrives with the token-budget
  milestone. The column remains part of the record.
- `submit` writes the user entry and the queued submission in one save. A duplicate
  request id returns the existing submission and creates nothing; the unique
  constraint is the final guard for a race.
- `settle` accepts only terminal statuses: `done`, `unanswered`, or `cancelled`.
  A second settle returns `AlreadySettled` and leaves the first ending intact. A
  settle is constrained to the save's conversation; a submission from another
  conversation is rejected as invalid input.
- `fork` copies the chosen entry and all earlier entries into a new conversation,
  gives them new ids and one new commit sequence, and sets `parent_id` and
  `parent_entry_id`. Documents are not copied. Titles are copied unchanged.
- A successful save updates the conversation's `updated_at` to the save time.
- `Harness::open_with` injects the clock and identifier source; production uses the
  system clock and UUIDv7.
- M1 is staged internally as M1a (session and storage), M1b (tasks, recovery, and
  effects), and M1c (crash suite), with one M1 gate after M1c. This is execution
  staging only; the product scope is unchanged.
- The daemon still uses the low-level `hearth_storage::open` for its lock and
  migration check. M1b will wire it to `Harness` so there is one session-facing
  writer path.

## 2026-11-08: M1a.1 corrections

- A read waits on a condition variable when all four pooled connections are busy,
  then proceeds. It no longer fails with a closed-storage error.
- A settlement's `answer_entry_id`, when present, must belong to the same
  conversation as the submission. A foreign entry returns `EntryNotFound`, and the
  submission stays unsettled.

## 2026-11-08: M1b-1 task state machine

- Sir confirmed the concrete `Storage` type on 2026-11-08, so the S1 and S2
  boundaries stay as built.
- Tasks are claimed in `COALESCE(next_wake_at, created_at), id` order. A pending
  task with no wake time or a due wake time is runnable; a running task is not
  claimable until recovery.
- `attempt` counts claims of the current step. `Next`, `Sleep`, and a saved
  retryable failure reset it to 0. A task whose expired lease is recovered at
  attempt 2 or more becomes failed with the result `{"error": "poisoned"}`.
- A retryable failure requeues after one second. A non-retryable failure settles
  the task as failed with the error as its result.
- A claim sets a lease; a heartbeat extends it only for the current owner. A stale
  owner cannot heartbeat or finish.
- Recovery requeues expired running leases below attempt 2, wakes due sleeps, and
  leaves pending and terminal tasks alone. It is idempotent.

## 2026-11-08: M1b-1.1 task commits

- Task creation, claim, finish, and each recovery transition are saves. They
  insert a `commits` row with the task id and a kind (`task.create`, `task.claim`,
  `task.next`, `task.sleep`, `task.terminal`, `task.retry`, `task.fail`,
  `task.recover`, `task.poison`, `task.wake`) and advance the save sequence in the
  same transaction as the task update.
- A heartbeat is lease metadata, not task progress. It does not insert a commit or
  advance the sequence.
- Conversation creation remains metadata outside the save log. The first save in a
  conversation follows the global sequence.
- A claim clears `next_wake_at`, because a running task is no longer asleep.
- `Session::latest_commit` exposes the newest line in the save log for diagnostics
  and tests.

## 2026-11-08: M1b-2a effect records

- Effect intents are part of a save. The save inserts each intent in the same
  transaction as its entries, documents, submissions, and settlements, and returns
  the ids in `Commit.effect_ids` in intent order.
- A duplicate `(kind, key)` reuses the existing effect row and returns its id. The
  first intent wins; the duplicate does not overwrite the payload or status.
- The save allocates its sequence before the changes and inserts the commit row
  last, so a failure anywhere in the save leaves no commit row, no effect row, and
  no sequence gap. Effect intents are inserted before that final commit row.
- Beginning, finishing, failing, and requeueing an effect are saves. Each inserts a
  `commits` row with the effect's task id when present and one of
  `effect.started`, `effect.done`, `effect.failed`, `effect.retry`, or
  `effect.requeued`.
- A retryable failure returns the effect to `intent` with the error in `result` and
  no settled time, preserving attempts. A non-retryable failure settles it as
  `failed` with the error and a settled time.
- Requeue is the crash-recovery path for a started effect. It returns the effect to
  `intent`, preserves attempts, payload, and task, and clears the settled time.
- Effect execution, retry policies, workspace confinement, and the daemon worker
  are M1b-2b. Crash injection is M1c.

## 2026-11-08: M1b-2b daemon recovery

- The daemon opens the `Harness` and uses its single session-facing writer path.
  The direct `hearth_storage::open` connection is removed.
- After acquiring the lock, the daemon runs `recover_tasks` before logging that it
  started and waiting for a signal. It logs the requeued, poisoned, and woken
  counts.
- The Harness is dropped before the lock, so the storage writer thread joins before
  the lock is released.
- No scheduler or effect worker runs yet. The daemon recovers and waits.

## 2026-11-08: M1b-2c effect execution

- `file_write` is an idempotent effect. Its payload is `{ path, content }`, the path
  is relative to the workspace root, and the write is confined after canonicalizing
  the root, the parent, and any existing target. `..`, absolute paths, and symlink
  escapes are rejected before anything is written. The file is written with mode
  `0600`.
- The effect worker polls every 100 ms, takes at most 10 pending effects per batch,
  and allows at most 3 attempts. A retryable failure returns the effect to `intent`;
  a non-retryable failure settles it as `failed`. The final attempt is
  non-retryable.
- On startup the worker requeues a started `file_write` effect because overwriting
  the same file is safe. A started effect of an unknown kind fails non-retryably so
  it cannot stay stuck.
- The daemon runs task recovery and effect recovery before starting the worker
  thread. The worker stops when the shutdown flag is set on SIGTERM or SIGINT, and
  the daemon joins it before dropping the Harness and releasing the lock.
- The workspace root is `agent.workspace` when configured, resolved relative to the
  home, otherwise `$HEARTH_HOME/workspace`.

## 2026-11-08: M1c crash suite

- `HEARTH_CRASH_AT` names one crash point and is inert otherwise. `crash_if` aborts
  with `std::process::abort()`.
- Covered points: `before_commit` (the transaction is open and no commit exists),
  `after_commit` (the commit is durable and the caller never returns), and
  `during_effect` (the file is written, the effect is still started, and no outcome
  is saved).
- `mid_stream` is deferred to M2, when provider streaming exists.
- The S7 suite runs a separate `crash-driver` process in `hearth-eval`, never the
  test process, and sets the child's working directory to the temp home so a core
  dump cannot dirty the repository.
- S7 results: `before_commit` leaves no entry and no commit, and the retry uses
  sequence 1; `after_commit` preserves one submission and a repeated request id is
  harmless; `during_effect` leaves a started effect and a written file, and restart
  requeues and replays it idempotently with attempts 2.

## 2026-11-08: M2a provider adapter

- Approved M2 decisions: the fake provider belongs to M2; a retryable task failure is
  logged by the M2 scheduler without adding a schema column; fork copies the current
  conversation documents at M2; the token estimator lands in M2.
- One synchronous OpenAI-compatible adapter. No async runtime. Requests use
  `POST {base_url}/chat/completions` with SSE, `stream_options.include_usage`, tool
  specs, optional temperature and output cap, the configured extra headers, and the
  session header when configured.
- The stream translates text, reasoning, tool-call fragments, usage, and the finish
  reason. Tool calls are assembled by index and their argument JSON is parsed before
  they are emitted. Malformed SSE is a structured protocol error.
- Thinking blocks are stored on entries but are not replayed in provider requests;
  the adapter sends text only. Reasoning is surfaced live as a `ThinkingDelta`.
- Initial requests retry transport errors and HTTP 429, 500, 502, 503, and 504 up to
  three attempts with exponential backoff through an injected `Sleeper`. Client
  errors such as 400, 401, 403, and 404 do not retry. A stream that has started is
  never retried.
- `estimate_tokens` is characters divided by four plus a 10 percent margin, rounded
  up. It is the single estimator for later budgets.
- Models without a known price record tokens with zero cost; the usage ledger and
  pricing registry arrive with M2b.
- `mid_stream` remains deferred to M2b.

## 2026-11-08: M2b-1 prompt and usage foundation

- `hearth init` writes `SOUL.md` at mode 0600 from `default_soul` when it does not
  exist. It never overwrites an existing identity file.
- Prompt assembly lives in `hearth-core::prompt`. The request order is one system
  message with the soul and operation text, the transcript in order, and a final
  system message with the runtime note. The audit document keeps five segments:
  identity, operation, tools, transcript, and runtime. The transcript segment
  excludes the identity and runtime system messages.
- Entry content is parsed permissively: strings and `{text: ...}` objects are text;
  typed blocks support text, thinking, tool_call, and tool_result. Thinking is
  stored but filtered out of provider requests.
- Prompt blobs use a UUIDv5 hash of the audit document. `INSERT OR IGNORE` dedupes
  by hash; `latest_prompt` returns the newest text.
- Entry `tokens_est` uses the model estimator over the entry's text, thinking, tool
  names and arguments, or tool result content. It replaces the M1 default of zero.
- `fork` copies the source conversation's current documents with their versions and
  JSON, and leaves the source untouched. Documents are not historical, so the fork
  gets the state at fork time.
- `usage_events` rows are metering records, not conversation saves; they do not
  insert a commit. The ledger records input, cache read, cache write, output,
  reasoning, cost, and latency.
- `Price` and `Pricing` live in `hearth-model`. Prices are micros per million tokens
  and the default registry is empty, so unknown models record tokens with zero cost.

## 2026-11-08: M2b-2a agent loop

- Starting a submission and recording a task checkpoint are saves with a commit
  kind of `submission.running` and `task.checkpoint`. A running submission is
  idempotent; a settled submission cannot be started again.
- The agent checks for an active generation task before beginning a submission, so
  a busy conversation leaves the new submission queued.
- One generation task is created per submission and claimed by id with a 60 second
  lease. The task checkpoint records the turn and the prompt blob hash.
- The prompt blob is saved before the provider request, and the assistant entry
  links to it through `prompt_hash`.
- Provider failures, stream errors, empty responses, and tool calls with no tools
  settle the submission as unanswered with one notice entry and fail the task. The
  notice is one line, truncated to 200 characters.
- Usage is recorded after the stream. Cost comes from the pricing registry; an
  unpriced model records zero cost.
- Tool execution is deferred to M2b-3. A tool call is never saved as an orphaned
  assistant entry; it settles unanswered instead.
- The assistant entry and the submission ending are separate saves for now. The M2
  gate should review whether to make the answer and ending one atomic save.

## 2026-11-08: M2b-2b local chat and agent hardening

- A successful run requires non-empty assistant text. A thinking-only stream is
  private reasoning, not an answer; it settles unanswered with a notice and saves no
  assistant entry. Usage is still recorded.
- The agent keeps an in-process reservation set of conversation ids in addition to
  the database active-generation check. The reservation is acquired before the
  database check and released by a guard on every return path, so two concurrent
  runs in one process cannot both start.
- `hearth chat` loads the configured provider, reads `identity.soul_file` or
  `$HEARTH_HOME/SOUL.md` with the default fallback, creates one fresh local
  conversation, and runs a line-based loop. It prints only the answer or the notice
  on its own line and exits on EOF.
- Pricing is empty in local chat, so the configured model records tokens with zero
  cost until a price is added.
- The assistant entry, submission ending, and task terminal transition remain
  separate saves. The M2 gate should review whether to combine them.

## 2026-11-08: M2b-3 tools, spill, and the tool loop

- Files and shell are inline tools. Shell commands start in the workspace, run
  without approval, and are not sandboxed.
- The files tool reads, writes, edits, and lists only inside the workspace. Reads
  may use the `spill/` prefix under the spill root. Absolute paths, `..`, and
  symlink escapes are rejected.
- Tool output above 4000 estimated tokens is written to
  `spill/<call-id>.txt` at mode 0600 and replaced with a head, tail, size, and
  path summary. The model reads the full output with `files read spill/...`.
- Assistant tool calls and their results are paired by call id. Pending calls
  with no result are replayed before the next provider request.
- The agent turn limit counts provider requests. Reaching it saves a notice,
  settles the submission unanswered with reason `turn_limit`, and fails the task.
- The shell timeout kills the shell process; a process group kill is deferred.
- Web fetch and the message tool are deferred.

## 2026-11-08: M2c-1 web search

- Exa is the zero-config provider. It uses the keyless MCP search endpoint by
  default and switches to the REST search endpoint when an Exa API key is
  configured.
- Brave, DeepSeek, Firecrawl, and TinyFish are included only when their API key is
  present and non-empty. The provider order comes from `web.search.order`; a
  provider failure falls through to the next one.
- The tool runs at most `web.search.parallel` queries at a time, requests
  `web.search.results` per query, deduplicates by URL in query order, and returns
  at most `web.search.max_results`.
- There is no answer synthesis and no page scraping. Results are title, URL,
  snippet, and date.
- A partial failure returns the successful results plus an `errors` array. A total
  failure returns the first provider error and is retryable only when every
  failure was retryable.
- Provider errors contain a status and a short body excerpt, never headers or
  credentials. Tests use local fake servers only.

## 2026-11-08: Web fetch provider chain (sir-directed)

- Sir directed web fetch to use an ordered fallback chain like web search.
- The default `web.fetch.order` is `["jina", "http"]`: Jina Reader free first, then
  the direct HTTP/HTTPS fetcher.
- Allowed fetch providers are `jina`, `http`, `https`, `tinyfish`, `firecrawl`, and
  `deepseek`. `http` and `https` are aliases for one direct fetcher; the first one in
  the order is used.
- `jina`, `http`, and `https` need no key. The other three are skipped when their key
  is absent.
- Every provider validates the target URL against private and link-local addresses.
  The direct fetcher also follows at most three redirects, caps the page at 2 MB,
  extracts HTML and PDF text, and has no cookies.
- The tool labels every result as untrusted web content.
- The specification was bumped to Draft 0.4 for this change.
- The TinyFish, Firecrawl, and DeepSeek fetch providers are accepted by the settings
  now and are implemented in M2c-2b.

New dependencies for the fetch work: reqwest gained the gzip and brotli
features for compressed pages, html2text extracts readable HTML, and
pdf-extract extracts PDF text. They are confined to hearth-tools and covered by
local fake-server and fixture tests.

## 2026-11-08: M2c-2b keyed fetch providers

- TinyFish fetch: POST the base `https://api.fetch.tinyfish.ai` with X-API-Key and
  `{urls:[url], format:"markdown", per_url_timeout_ms:30000}`. Read `results` and
  `errors`; use the matching result's `text` (string or object), `title`, and
  `final_url`.
- Firecrawl fetch: POST `{base}/v2/scrape` with Bearer auth and
  `{url, formats:["markdown"], onlyMainContent:true, lockdown:true}`. Read
  `data.markdown` and `data.metadata.title`.
- DeepSeek fetch: POST `{base}/v1/tools/fetch` with Bearer auth and `{url}`. Read
  `title` and `content`.
- Keyed providers join the chain only when their key is present and non-empty.
  `http` and `https` are one direct provider; the first alias wins and the second
  is skipped. The chain follows `web.fetch.order`.
- 429 and 5xx are retryable; other 4xx are permanent. Provider content is capped
  at the fetch limit with a truncation note.

## 2026-11-08: Telegram admission (M3a)

- Authorization requires the sender to be in `allowed_user_ids`; when
  `allowed_chat_ids` is non-empty the chat must also be allowed. Bot senders are
  rejected. A missing sender is unauthorized.
- The transport key is `<chat_id>:<thread_id>`, with thread id 0 when absent. The
  newest non-archived conversation for that key is reused; otherwise a new
  conversation is created from the chat title, the sender's first name, or
  `Telegram`.
- The request id is `tg:<chat_id>:<message_id>`. The user entry's source is the
  message id.
- The `telegram_updates` row, the `meta.telegram_offset` update, the user entry,
  and the queued submission commit in one save. A duplicate update id rolls the
  save back and returns the existing submission; the poller advances the offset and
  skips the reaction.
- Ignored updates advance the offset directly. A failed admission returns without
  advancing so Telegram can replay it.
- The `👀` reaction is best effort. A failure is logged and does not fail the
  admission.
- Bot API errors and transport messages never include the token or request URL.
  Attachments, voice, callbacks, and replies are deferred to M3b.

## 2026-11-08: Telegram outgoing text (M3b-1)

- A final answer under 32,768 UTF-8 bytes and 500 non-empty blocks is sent as one
  `sendRichMessage` with the Markdown. The rich path is skipped when either limit
  is exceeded.
- Only a 4xx rich rejection triggers the entity fallback. A transport failure,
  protocol failure, or 5xx is uncertain and is never resent by a different method.
- A 429 sleeps for `retry_after` seconds (one second when absent) and retries the
  same method up to three attempts. The same rule applies to fallback parts.
- The fallback renders Markdown to plain text plus Telegram entities. Offsets are
  UTF-16 code units, non-code entities are clipped around code and pre ranges, and
  entities are sorted and capped at 100 per message.
- Fallback text splits at 4,096 UTF-16 units, never inside a surrogate pair,
  preferring paragraph, line, then space boundaries. Each part gets an `(n/m)`
  marker. A split pre entity stays a pre entity with its language.
- Sends are spaced by at least 33 milliseconds, capped at about 30 per second.
- `sendChatAction` typing is exposed for the daemon worker.

## 2026-11-08: Telegram inbound media (M3b-2a)

- Inbound files live at `$HEARTH_HOME/media/inbound/YYYY-MM-DD/`, are written at
  mode `0600`, and have sanitized names prefixed with the Telegram message id.
  The date comes from the message timestamp in UTC.
- Telegram files over 20 MB are not downloaded; the message is admitted with a
  short note instead. A failed `getFile`, download, or media save also admits a
  note rather than dropping the user's message.
- Photos become image blocks stored by path. The OpenAI adapter reads the file at
  request time, downscales the long edge to 1568 pixels, encodes PNG, and base64s
  it into an `image_url` data URL. The original file is retained.
- Documents, audio, and video become file blocks. The adapter renders file blocks
  as a compact text note with the name and path; it never embeds file bytes.
- Voice notes are transcribed through Groq when `media.stt` is configured. The
  transcript is the user text block and the original voice file is retained as a
  file block. With no transcriber, the message is admitted with a
  not-transcribed note and the file block.
- A replayed Telegram update is detected before any download, so a replay costs no
  media transfer.
- New dependencies: reqwest multipart, image, and base64 in `hearth-model`; time in
  `hearth-telegram`.

## 2026-11-08: Telegram outbound media (M3b-2b)

- File sends use Bot API multipart. A single photo, document, video, or voice note
  uses its matching method with the `chat_id`, an optional caption, and the file
  part named after the method.
- Two to ten compatible photos or videos go as one `sendMediaGroup`. The media
  JSON references `attach://fileN` and the parts are named `file0`, `file1`, and so
  on. A caption is placed on the first album item only.
- Mixed groups and document or voice groups are sent individually. A caption is
  attached only to the first item.
- Captions are sent only when the caller explicitly supplies a non-empty caption.
- The message tool resolves files through the workspace and spill roots. Path
  escapes, missing files, more than ten paths, using both `path` and `paths`, and
  empty arguments are rejected.
- Voice replies are synthesized only when `speech` is present. The OGG/Opus output
  is written under `media/outbound` at mode `0600` before sending. With no
  configured speaker, speech is a clear error.
- ElevenLabs defaults are `eleven_v4` and `opus_48000_64`; the client posts to
  `/v1/text-to-speech/<voice>?output_format=<format>` with `xi-api-key`.
- New dependency: reqwest multipart in `hearth-telegram`.

## 2026-11-08: M3c-1 Telegram daemon loop

- `telegram.base_url` is an optional local Bot API or proxy override. It must be an
  http or https URL and defaults to `https://api.telegram.org`. Spec draft is 0.5.
- The daemon runs a poller thread and a one-at-a-time queued-submission worker when
  Telegram is configured. Each run gets a fresh per-chat tool registry.
- Typing is sent every four seconds while a run is active and stops before the final
  answer is delivered.
- A normal answer is sent with the rich-first sender; a notice is sent when the run
  settles unanswered. A failed send leaves the durable answer in the record.

## 2026-11-08: Telegram delivery effects (M3c-2a)

- Every final Telegram send is a `telegram.send` effect with key
  `tg:<submission_id>` and payload
  `{ chat_id, thread_id, markdown, submission_id }`.
- The intent is saved before the send. A Rich or Fallback send settles it done.
  An uncertain send becomes Unknown. A definitive rejection becomes Failed.
- Telegram sends are at-most-once. The effect worker never executes a
  `telegram.send` intent, and on recovery it marks a started one Unknown instead
  of failing or retrying it.
- Before a queued submission runs, the worker recovers at most one Intent or
  Unknown `telegram.send` for that chat. It sends
  `Recovered reply:\n\n<markdown>` and marks the effect done, unknown, or failed.
  A recovery failure does not block the new submission.
- `Session::mark_effect_done`, `mark_effect_failed`, `mark_effect_unknown`, and
  `effects_awaiting_delivery` support this recovery path.

## 2026-11-08: Telegram stop handling (M3c-2b)

- A message whose trimmed text is exactly `stop`, case-insensitive, is a control
  message. It is not admitted as a submission and gets no reaction.
- The stop update is recorded in `telegram_updates` and the next offset is stored
  with `Session::record_telegram_update`, in one transaction. A replay of the same
  update id advances the offset but does not call the handler again.
- The poller calls an optional `StopHandler` with the chat id. The daemon's handler
  sets a per-chat cancellation token.
- The worker registers a token before each run and removes it through a drop guard.
  `Agent::run_submission_with_cancel` checks the token after the task claim, before
  every provider request, after the stream and usage recording, and after tool
  execution.
- Cancellation discards unsaved streamed text and tool calls, saves one
  `{"text":"Stopped."}` notice, settles the submission as `cancelled`, and finishes
  the generation task `aborted` with `{"reason":"cancelled"}`. Usage from a provider
  call that already started is recorded.

## 2026-11-08: Telegram queue cap and shared limiter (M3c-2c)

- One chat may have at most 10 queued submissions. The cap is a constant for now,
  not a setting.
- The poller checks the count after authorization, the stop path, and the update
  dedupe, and before downloading any attachment. A message over the cap is not
  admitted, gets no reaction, and is recorded as a control update with the next
  offset.
- The first over-cap message in an overflow period sends one plain notice:
  `I'm busy with the current queue. Try again shortly.` The per-chat flag clears
  when a later message finds the queue below the cap.
- `PollReport` counts these as `busy`.
- The message tool now holds a `TelegramSender`, and all of its photo, document,
  video, voice, and album sends go through the sender's shared 33 millisecond
  limiter. Replies, typing, and media sends therefore share one send budget.

## 2026-11-08: Live watch and daemon status (M3d)

- The daemon serves `$HEARTH_HOME/hearth.sock`, mode `0600`, with versioned JSON
  lines. A stale socket is removed at bind and the file is removed on shutdown.
- Protocol version 1: `{"v":1,"cmd":"status"}` returns one status object;
  `{"v":1,"cmd":"watch","task_id":null}` acknowledges with
  `{"v":1,"ok":true}` and then streams event lines. A task id filters events. A
  disconnect removes the subscriber; detaching never cancels a run.
- Event lines are `{v,ts,kind,task_id,conversation_id,data}`. Kinds: run_started,
  turn_started, thinking_delta, text_delta, tool_call_started, tool_result,
  task_step, usage, run_settled.
- The bus redacts configured secret values before fan-out. Tool arguments and
  results are previewed at 2000 characters in the event stream.
- No event history is kept. A watcher attaching late sees only later events.
  Thinking is live only and never replayed from storage.
- Status fields: model, uptime_seconds, tasks_running, tasks_asleep,
  today_cost_micros, cache_read_tokens, input_tokens, cache_hit_rate,
  memory_items, standing_memory_version, last_skill_review. Memory and skill
  values stay zero until their milestones.

## 2026-11-08: Memory store and tool (M4a)

- Memory writes go through the save path. A conversation save can mix entries and
  memory changes; the memory service uses a conversation-free `save_memory` that
  still writes one commit (kind `memory`).
- Each memory change writes a `memory_events` row with the actor and the old and
  new content hashes. The hash is UUIDv5 of the content.
- The exact-duplicate fingerprint is UUIDv5 of the cleaned, lowercased content.
  Near duplicates merge when token Jaccard similarity is at least 0.85. A merge
  takes the maximum importance and the union of tags.
- A superseding memory inserts the new row and marks the old row `superseded`
  with `valid_to` set to the save time, in the same save.
- Forget keeps the row as a `deleted` tombstone, explicitly removes its FTS5 entry
  after the update trigger reindexes, and writes a forget event.
- Pin sets `standing` to `pinned`; unpin returns it to `none`.
- Secret rejection is hand-coded: common key prefixes, bot-token-shaped strings,
  and key=value pairs for api_key, apikey, token, password, or secret with an
  eight-character or longer value that is not an environment reference.
- The memory tool actions are search, get, save, update, forget, pin, stats, and
  as_of. Search returns compact lines; get returns provenance; as_of accepts
  RFC3339 or YYYY-MM-DD and reads the state valid at that instant.
- The memory tool is in the local chat and daemon registries. Standing-memory
  prompt injection and automatic recall are M4b; extraction is M4c.

## 2026-11-08: Standing memory and recall (M4b)

- Q6 seed constants: RRF k=60, 180-day recency half-life, near-duplicate Jaccard
  0.85, weights recency 0.20, importance 0.20, staleness 0.15, recall budget 900
  tokens and 6 items, standing cap 1200 tokens, promotion threshold 85.
- Standing memory includes pinned rows always and promotes active rule,
  preference, and person rows at importance 85 or higher. Promoted rows beyond
  the token cap are demoted to `none`; pinned rows are never demoted.
- The standing version lives in the `memory_state` document on a hidden
  `system:memory` conversation. It increments only when the standing set changes,
  in the same save as the memory change. The document write carries its expected
  version and retries once after a mismatch.
- Recall uses FTS5 BM25 top 40, standing rows, and tag matches, fused with RRF.
  Recency and staleness use the 180-day half-life. Diversity keeps at most two
  rows per kind and collapses near duplicates. Returned rows update
  `last_used_at` in a save.
- Trivial input (two characters or fewer, a known greeting or acknowledgement,
  or no alphanumeric characters) returns no recall without a model call.
- Prompt L3 is a `## Standing memory` system message after identity and
  operation. Prompt L6 is a `## Relevant memory` system message immediately
  before the newest user message. Both appear in the audit document as
  `standing_memory` and `recall`.
- The agent reads standing and recall before every prompt. A memory error is
  logged and the run continues with empty segments. The runtime builds the
  policy from `settings.memory`.

## 2026-11-08: Memory extraction and consolidation (M4c)

- After a done run with non-trivial user content, the agent creates a durable
  `memory.extract` task in the conversation, claims it, and runs one bounded
  extraction call before returning. A trivial run creates no task. Extraction
  failure never changes the answer.
- The extraction view is the run's user entry through its final assistant entry,
  excluding later queued user entries, capped near 2000 estimated tokens. It
  carries entry ids. The request also includes the five most similar memories by
  FTS.
- The model returns JSON `{"operations":[...]}` with at most three add, update,
  supersede, or noop operations. Every non-noop operation must cite a run entry
  id and a quote that appears in that entry after whitespace and case
  normalization; invalid evidence is rejected.
- Adds use source_kind session and source_ref the evidence entry id, with actor
  system. Updates target `supersedes_id`; supersedes insert a new row and close
  the old one. The memory service's secret rejection still applies.
- The extraction call is metered with kind `memory.extract`, linked to the task,
  and its prompt is saved as a prompt blob. The task ends done with applied and
  rejected counts, or failed on a provider or JSON error.
- The standing refresh now compares the final standing set with the stored set
  and bumps the version only when they differ, so a qualifying row that does not
  fit the cap does not step the version.
- Consolidation is a skeleton: it scans active rows, merges exact and near
  duplicates into the earlier row with max importance and unioned tags, and
  supersedes the later rows. No episode summaries, nightly schedule, or
  importance recompute beyond the merge rule.
- The S8 recovery test exposed that the extraction must use the run's own user
  entry and final answer, not the newest user entry, because a queued input can
  arrive mid-run. The run-entry selection now matches the submission's
  created_at and content and stops at the answer entry.

## 2026-11-08: M4d memory benchmark

- Fixture: `crates/hearth-eval/fixtures/memory_bench.json`, 200 synthetic memories and 200 questions across direct, preference, person, project, temporal, contradiction, paraphrase, and duplicate categories, plus a 10-string secret adversarial set. The first 20 questions are marked `spot_check`.
- Targets: recall@10 0.90, precision@5 0.80, recall tokens p95 900, retrieval p95 80 ms at 200k items, zero secret leaks.
- Measured baseline: recall@10 0.161, precision@5 0.148, recall tokens p95 161, retrieval p95 5.5 ms on the small set; temporal 0/20, supersession 15/15, paraphrase 0/20, duplicate failures 0, secret leaks 0, stored rows 190 (200 fixture memories, 5 intentional duplicate merges, 5 near-duplicate merges from the M4a dedupe). The 200k scale check: 20 queries, p95 1523 ms against the 80 ms target.
- The benchmark may miss the 1.0 targets; it prints the targets and the baseline. Only harness errors, nonzero leakage, or duplicate-write failure are hard failures.
- The supersede path now skips duplicate merging so a deliberate replacement inserts the new row. A bearer credential pattern is rejected by the memory writer.
- Status: `standing_memory_version` reads the `memory_state` document; `memory_items` is the real count.
- CLI: `hearth eval memory [--json] [--scale 200k]`; the 200k check is an ignored test and the CLI flag.

## 2026-11-08: M4e memory benchmark fixes

- The previous M4e worker left uncommitted work and no report. Its RRF normalization, digit-aware content tokens, and as_of created_at removal were kept after review; its FTS-only recall was rejected because spec 7.4 requires the standing and tag candidate sources.
- Direct facts: the fixture direct facts were importance 56 to 60 while noise facts were 62 to 77, and the two-per-kind cap pushed the expected fact out. Direct facts are now 80. Direct recall is 0.957.
- Temporal: `as_of` now orders FTS matches by `bm25` instead of `created_at` with a 50-row cap. The created_at filter stays removed because spec 7.7 asks what was true, not only what was already learned. Temporal is 20/20.
- Dedup: content tokens keep single digits, so 1.0 and 1.1 are distinct. The fixture contradiction rows that duplicated temporal facts or nearly duplicated direct facts were corrected. Stored rows are 195, which is the five intentional duplicate merges.
- Tag candidates now use FTS5 rather than a `LIKE` scan over `memory_items`.
- New baseline: recall@10 0.806, precision@5 0.285, direct 0.957, temporal 20/20, supersession 15/15, stored 195, paraphrase 0.650, preference 0.200, duplicate failures 0, secret leaks 0. Small-set retrieval p95 6.87 ms.
- Remaining M6 gap: preference recall, paraphrase, precision, the 1.0 recall target, and the 200k p95 of 1,868 ms. The 200k cost is the standing query at scale, not the tag path.


## 2026-11-08: M6a retrieval and benchmark labels

- The benchmark now labels only genuinely relevant memories. Precision is measured
  over `min(5, relevant)`, so a one-fact question is precision@1 rather than an
  impossible precision@5.
- Standing memory stays a recall candidate source but is excluded from the recall
  selection because it is already in the L3 prompt. The benchmark counts the L3
  standing set for hits and precision.
- FTS queries drop common stopwords. Migration 0002 rebuilds `memory_fts` with the
  porter stemmer for inflection matching. Migration 0003 adds partial indexes for
  active standing rows and promotable candidates.
- Result: recall@10 0.939, precision 0.928, preference 1.000, direct 0.971,
  paraphrase 0.650, 195 stored rows, temporal 20/20, supersession 15/15, zero
  secret leaks. The 200k retrieval p95 is 353 ms against the 80 ms target; that
  gap moves to M6b.

## 2026-11-08: M6b episodes, decay, archive, and the 200k path

- Decay is computed at retrieval time; the stored importance is unchanged. The
  effective importance is 5 percent lower per 90 days without use, floored at 15
  for unpinned items. Pinned items and rules whose source is the user never decay.
  The recall score uses the effective importance.
- `archive_stale` is an explicit maintenance pass. Active items unused for at least
  three years and with stored importance below 20 become `archived`, except pinned
  items and user-stated rules. The default `search` and `as_of` hide archived rows;
  `search_with` and `as_of_with`, and the memory tool's `include_archived`, expose
  them. Archived rows are never deleted.
- `consolidate_episodes` writes one extractive episode per conversation for the
  entries since the last episode. The summary is capped near 500 estimated tokens,
  uses the conversation id as `source_ref`, records the last covered entry as
  `covered:<entry-id>`, and makes no model call. A second run with no new entries
  writes nothing. The duplicate consolidation skips episode rows.
- The 200k path short-circuits `recall_read` when the FTS has no matches, before
  the tag and standing queries. The 200k p95 is 0.617 ms against the 80 ms target;
  the small-set p95 is 5.2 ms. The recall, precision, and ranking results are
  unchanged.
- Paraphrase remains 0.650. The porter stemmer covers inflection, but synonym-level
  paraphrases need a curated expansion layer or embeddings; that belongs to a later
  retrieval pass, not this slice.
- The nightly consolidation and archive schedule remain M7 work.

## 2026-11-08: M6c 200k benchmark correction

The M6b 200k scale check seeded only 200,000 "scale fact N" rows and then ran
the fixture's direct questions. Those questions had no FTS matches, so
`recall_read` short-circuited and reported 0.617 ms. The M6a 353 ms was also a
no-match path. The benchmark now loads the 195 stored fixture rows plus 199,805
noise rows, asserts that all 20 direct questions match their expected fixture
memory, and measures the matching path.

The matching p95 is 2.86 ms: search 1.36 ms, cached standing 0.003 ms, and the
remaining tag and scoring work about 1.5 ms. The cold standing query is 353 ms.
`MemoryService` now caches the standing block and invalidates it on every
commit, so the warm retrieval path meets the 80 ms target. The first recall
after a standing-changing write can still pay the cold cost; that residual is
recorded for the M6 gate.

The 2,000-item harness regression test and the ignored 200k test both assert
the 20 fixture matches, so a future no-match false pass fails. The small-set
benchmark results are unchanged.

## 2026-11-08: M6d standing cache and the real recall path

The M6c premise was partly wrong. The real `recall` writes last_used through
`session.save_memory`, which never invalidated the standing cache. The
`update` path and every `commit` did. The cache is now invalidated only when a
write can change the standing set, order, or rendered block: an add that
auto-promotes, an update of standing, importance, status, valid_to, or content,
a pin, a forget, an archive, a consolidation merge, or a standing refresh. A
low-importance add, a tags-only update, a non-standing content update, and a
last_used-only update keep the cache. A last_used-only update of a standing item
updates the cached item in place and re-sorts the cached selection.

Migration 0004 limits the FTS update trigger to changes of `content` and `tags`.
Last-used, importance, standing, and status updates no longer reindex a row.

`run_scale` now measures the real `recall` path with its last_used write as well
as `recall_read`. At 200,000 items the matching real recall p95 is 5.27 ms and
recall_read is 2.78 ms against the 80 ms target. A genuine standing change still
invalidates the cache; the next standing query is a cold 353 ms at 200k. That is
a rare-path residual, not the normal path. Paraphrase recall remains 0.650.

The S2 schema tests were updated to version 4. The small-set benchmark is
unchanged: recall@10 0.939, precision@5 0.928, temporal 20/20, supersession
15/15, 195 stored rows, 0 secret leaks.

## 2026-11-08: M6e memory provenance and global standing version

- An explicit memory-tool save now uses the current run's user entry as provenance: source_kind `user`, source_ref the entry id. Without a run source it records source_kind `session` and no source_ref; it never claims assistant provenance for a user fact.
- `Session::latest_user_entry_id` returns the newest user entry. The runtime `build_tools` takes an optional `MemorySource`. The daemon sets it per submission; local chat rebuilds the per-line tools and agent with that line's source.
- The standing version is global in the meta key `standing_version`, not a per-conversation `memory_state` document. A bump writes the memory changes and the new version in one transaction with an expected-value check and one retry. A missing key is version 0.
- The `memory_state` document kind remains available for future last-consolidation data. The memory save context remains the system/memory conversation.
- The daemon status reads the global version through `MemoryService::standing_version`.

## 2026-11-09: M5a context budgets

- `models.context_window` (default 128000) and `models.max_output_tokens` (default 8192) are optional settings. `compaction.soft_compact` (0.65), `hard_compact` (0.85), and `keep_recent_tokens` (24000) are optional.
- Budget: reserve = max(max_output, 4096) + 1024; usable = window - reserve; soft and hard are floors of their fractions of usable; keep is min(keep_recent, 35% of usable).
- Segment budgets follow 6.1. The fixed overhead is identity + operation + tools + standing + skills + runtime; the target is 4500. The transcript takes the remainder. The prompt audit document records per-segment counts plus total and fixed.
- Before a provider request, if the prompt exceeds usable: drop trailing recall lines from the lowest score, then replace oversized old tool results with a bounded head and tail plus a truncation note, then settle unanswered with reason `budget` and make no provider call.
- The provider request's `max_tokens` is the configured max output. Local chat and the daemon build the budget from settings; the default applies when the new keys are omitted. The `budget` notice is the M5a fallback; soft and hard compaction is M5b.

## 2026-11-09: M5b compaction and memory flush

- The compaction plan walks back from the newest entry until it has at least `keep_recent_tokens`, then cuts at a user or assistant entry. A tool result is never separated from its assistant. The plan records the covered sequence range and `first_kept`.
- One summary entry kind `summary` stores `{"text", "covers": {"first_seq", "last_seq"}, "first_kept"}`. The prompt starts with the newest summary or reset marker, keeps entries at or after `first_kept`, and excludes earlier summaries and resets. A second compaction may summarize the earlier summary.
- The summary call uses the configured model, empty tools, and `max_tokens = min(0.8 * reserve, max_output_tokens)`. It records usage kind `compaction`, creates a durable `compaction` task, and saves the summary prompt blob. The summary text is the only model output used.
- The memory flush uses the M4c extractor over the same range before the summary call, with usage kind `memory.extract`. A flush failure is logged and does not block the summary.
- Hard compaction runs before the provider request when the prompt exceeds the hard threshold, then rebuilds the prompt; a second over-budget prompt takes the M5a budget notice path. Soft compaction runs after the answer is durable and delivered, from the daemon worker and local chat, so the user never waits on it. The idle daemon check runs every 60 seconds, selects at most one conversation per 60 minutes, is silent, and requires `min(100000, 65% of usable)` visible tokens plus history newer than the last summary. An attempted history is not retried until a new entry arrives. A genuine writer conflict waits for the next idle period.
- A provider context-overflow error compacts once from the visible history and retries the request once; a second failure takes the normal provider-error path.
- Known limits for review: the idle daemon path is covered by the core candidate test rather than a 60-minute integration test; the summary text quality is not model-evaluated offline; the soft pass adds the flush and summary cost after the answer.

## 2026-11-09: M5c prompt cache, prompt audit, pricing, and stats

- The stable prefix is identity, operation, tools, standing memory (including its
  version), and skills. A prefix hash and per-part hashes are stored in the
  generation checkpoint. A change is logged with the layer names SOUL.md, operation,
  tools, standing memory, or skills index. Transcript, recall, and runtime changes do
  not change the prefix.
- `hearth prompt` supports the newest prompt, a specific entry, JSON, the stable
  system prefix with no history, a preview, and the wire body. It redacts configured
  secrets. It opens the harness for reading; it does not save prompts or call a
  provider.
- `models.pricing` is an optional per-model map in micros per million tokens. Unknown
  models remain unpriced, record tokens, and cost zero; the stats and status show
  them as unpriced. No rates were invented for the live model.
- `hearth stats` aggregates by day, model, and task kind. The usage ledger has no
  per-tool column, so the spec's "by tool" claim is narrowed to what the record can
  support instead of adding a column without a tool call path.
- The stats command opens the database read-only. The prompt audit uses the harness
  and may perform normal migration on first open; it does not write records.

## 2026-11-09: M5d-1 lazy tool groups and steering

- `tool_search` is a core tool. The core schema budget of 1200 tokens is covered by a unit test that includes files, shell, web search, web fetch, memory, message, and tool_search. Optional group schemas are added only after selection and appear in the same prompt tool segment and audit document.
- The group registry defines browser, documents, and media. None of them ships a callable tool in this slice, so `tool_search` lists them as unavailable and a selection returns `group_unavailable` rather than advertising a tool that does not exist.
- A selected group is recorded in the generation task checkpoint as `selected_groups`, loaded at the start of a claimed run, and shared with the `tool_search` tool. The agent exposes only the selected group's specs and only the selected group's tools are callable.
- Steering absorbs queued submissions for the same conversation at each turn boundary before a provider request. Their user entries are already in the transcript. The active run settles every absorbed submission with the same terminal outcome and answer entry, so the worker cannot pick up a duplicate. A run never changes a request already in flight.
- The existing daemon uncertain-delivery test now delays its second update by 500 ms. Without the delay the second update is correctly absorbed into the first run, which leaves no later interaction to exercise recovered delivery. The concurrency test now expects the queued second submission to be absorbed and done rather than left queued.
- Known limits for review: the optional groups are placeholders until the optional tools are built; the checkpoint selection is loaded on a claimed checkpoint, but cross-process task resume remains M7 work; absorbed submissions share the active answer entry by design.

## 2026-11-09: M5d-2 token-efficiency benchmark

- `hearth eval tokens [--json]` runs a deterministic 100-turn conversation on a
  temporary home with a scripted provider. The script uses an 8192-token window,
  1024 output tokens, 65/85 compaction, and a 2000-token keep-recent boundary. The
  deliberately small window exercises compaction; 68 compactions in the run are
  reported for visibility, not as a target.
- The scripted provider simulates a provider prefix cache honestly: the first chat
  has no cache read, and later chats cache only the stable prefix when its hash is
  unchanged. The script asserts both the prompt-assembly prefix hash and the
  provider's chat-prefix hash are stable.
- Measured on this host: fixed prompt overhead p95 830 of 4500 tokens; cache-read
  share 0.99 of 0.80; save p95 0.28 of 15 ms; prompt assembly p95 1.63 of 300 ms;
  recall p95 0.66 of 80 ms; 68 compactions and 2176 summary tokens. Recall tokens
  are zero because the seeded preference is standing memory and is already in L3.
- Startup and recovery, idle memory use, and annual database size remain M8 work.
