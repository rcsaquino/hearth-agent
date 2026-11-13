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

## 2026-11-09: M7a scheduler and reminders

- The scheduler claims only task kinds present in its handler registry, at most one due task per tick, with a 60 second lease and a heartbeat before and after the step. It starts after startup recovery and effect recovery.
- Sleeping tasks are created atomically with `create_sleeping_task`; recovery wakes due asleep tasks and the scheduler claims only pending tasks. `claim_task_for_kind` prevents the scheduler from touching generation, compaction, or memory-extract tasks.
- The reminder task kind is `reminder`. Its input carries chat id, thread id, message, repeat interval, and first wake. Its Telegram effect key is `reminder:<task_id>:<scheduled_wake>`, which makes the send at-most-once across retries and restarts.
- An uncertain send is marked unknown and the task is retried once with backoff; the next step sees the unknown effect and settles the task done with delivery unknown instead of sending again. A permanent API rejection fails the task.
- Recurring reminders re-sleep to now plus the repeat interval, coalescing missed runs into one send. One-shot reminders finish done.
- The `schedule` tool is registered only on the Telegram path, where a notification transport exists. Local chat has no schedule tool.
- The reminder effect payload carries `reminder: true` so answer-recovery does not resend it as a recovered reply.

## 2026-11-08: M7b helpers

- A helper is a durable task of kind `helper` attached to its own child conversation
  (`transport = helper`, `parent_id` set). The brief is submitted as the helper's
  only user input; the parent transcript never enters the helper prompt.
- Helpers run in a pool with `tasks.max_concurrent` workers (4 by default). Spawn is
  rejected beyond that or beyond `tasks.max_depth` (2). The task tool is present
  only while the current depth is below the maximum, so a depth-2 helper cannot
  spawn another.
- The report is the helper's final text. It is written to
  `spill/helper-<task-id>.txt` at mode `0600` and saved as an assistant entry in the
  parent conversation with source `helper:<task-id>`. One settle notice carries the
  outcome and report path.
- The settle notice and the ETA reminder are `telegram.send` effects keyed
  `helper.notice:<task-id>` and `helper.eta:<task-id>`, so each is at most once
  across retries and restarts.
- A kill cancels the helper's provider work at the next boundary, aborts the task,
  and sends no settle notice. It never deletes the parent record.
- Retention keeps helper runs newer than `retention_days` or within the newest
  `retention_runs` (30 days, 25 runs by default); older helper conversations are
  archived, not deleted, and the tasks remain readable.
- `token_budget` is enforced as the helper's context-window cap (never above the
  configured model window); `wall_seconds` (3600 default) cancels the run at the
  next agent boundary. There is no separate hard total-token meter in v1.
- Helper agents skip post-run memory extraction; their working clutter must not
  become long-term memory.

## 2026-11-09: Skills (M7c-1)

- User-made skills live in `$HEARTH_HOME/skills/` as `SKILL.md` inside a named directory or as a flat `<name>.md`. They are read-only to Hearth. Learned skills live in `skills/learned/<name>.md` with frontmatter, versions in `skills/learned/.history/<name>/vN.md`, and a `CHANGELOG.md`.
- Frontmatter is parsed conservatively without a YAML dependency. Missing closing fences, unknown statuses, and non-numeric counters are errors. User skills may omit frontmatter; the file name and first body line provide the name and description.
- The prompt skills index contains active skills only, capped at 300 estimated tokens and 50 active skills across user and learned. Learned skills over the cap are archived by lowest success rate, then least recently used; user skills are never archived. The index is the L4 prompt layer, the M5a `skills` segment, and the M5c `skills` prefix part.
- The `skill` tool loads, saves, and archives. Load records uses and last_used for learned skills. Save and archive only apply to `skills/learned/`; a user skill rejects both. Save increments the version, copies the old version into history, and appends the changelog. New learned skills are drafts; the review in M7c-2 promotes, revises, quarantines, and archives them.
- `hearth skills list|show|history|diff|stats` is read-only. The diff is a line comparison of two preserved versions.
- The tool is registered in local chat, the Telegram daemon worker, and the prompt-audit tool set. Helper subagents keep their restricted tool subset and do not get the skill tool.
- Skill review and the nightly maintenance schedules are M7c-2, not this slice.
- Unrelated test fix: the memory as-of test could place the new fact on the same microsecond as the observation boundary under load; the test now sets valid_from strictly after the boundary.

## 2026-11-09: Nightly maintenance and skill review (M7c-2)

- Two global recurring tasks are maintained: `memory.consolidate` at 03:30 local and `skill.review` at 04:30 local. The times are constants (not settings); the local offset comes from the host through `time`'s `local-offset` feature with UTC as the fallback. That feature pulled the transitive `num_threads` crate into `Cargo.lock`; no direct dependency was added.
- `ensure_maintenance_tasks` creates a task only when no open task of that kind exists. A missed night runs once on the next start, and the handler sleeps from the current time to the next slot, so several missed nights coalesce. A second start the same night creates nothing. Both tasks are silent in Telegram.
- `memory.consolidate` runs the M4c duplicate consolidation, the M6b stale archive, and episode creation, then records `last_consolidation` and sleeps until the next 03:30.
- `skill.review` records the outcome of every learned skill loaded during a settled run: a done submission is a success and resets the consecutive-failure counter; unanswered or cancelled runs are failures. A skill's own body is never evidence.
- Automatic lifecycle before the model: a draft with at least one success becomes active; two failures quarantine; 90 days unused or a success rate below half after four uses archives. The model receives the recent successful entries, the skill index and bodies, and standing memory, and returns at most three evidence-backed changes.
- A create requires evidence from at least three distinct successful submissions. All evidence ids must belong to a successful run; user skills are never edited. Applied and automatic transitions count toward `last_skill_review_changes`.
- The review records usage with kind `skill.review` and no conversation id because the maintenance task is global. It saves the review prompt blob and uses `last_skill_review` as the next cutoff; the first review looks back 30 days.
- Review failures are logged, the night is still recorded, and the task sleeps until the next night rather than retrying in a hot loop.
- `hearth status` exposes `last_consolidation`, `last_skill_review`, and `last_skill_review_changes`.
- The maintenance scheduler exists only on the Telegram daemon path. A daemon without Telegram does not create or run these tasks.

## 2026-11-09: Documentation, packaging, and the release scan (M8b)

- `README.md` is the technical overview; `QUICKSTART.md` is the operator path; `SECURITY.md` and `CONTRIBUTING.md` are the policy documents. No command is documented unless it exists.
- `scripts/package.sh` builds one target with `cargo build --release --locked`, stages the binary, `README.md`, `LICENSE`, and `QUICKSTART.md`, writes a versioned tarball under `dist/`, and writes a SHA-256 checksum. It requires the target to be installed and fails with the `rustup target add` hint otherwise. It does not publish.
- `scripts/public-scan.sh` is a release gate, not part of `scripts/ci.sh`. It reads a private deny list from `HEARTH_PUBLIC_DENYLIST` (default `~/.hearth-public-denylist`), scans every git-tracked file for literal entries and common credential patterns, and prints only the offending file name. It refuses to run without a deny list.
- `scripts/test-scripts.sh` checks both scripts: a temporary repository with a planted deny-list value and a planted credential pattern, a clean tree, and the host package contents and checksum.
- The aarch64 package is documented but not built on this host; it needs `rustup target add aarch64-unknown-linux-gnu` and a cross linker.
- Publication, the 7-day soak, and the M8 gate remain for M8c.

## 2026-11-09: M8b documentation, packaging, and the public scan

- The public docs are README, QUICKSTART, SECURITY, and CONTRIBUTING, with neutral
  placeholders (`USER_NAME`, `123456789`, `~/.hearth`, `example.com`). The
  packaging unit template is `packaging/hearth.service`.
- `scripts/check-docs.sh` verifies the required docs, the placeholders, the
  executable scripts, and the absence of credential-shaped values in the docs.
  It runs from `scripts/ci.sh`.
- `scripts/package.sh` reads the version from `cargo metadata`, builds the
  requested target, packages the binary, LICENSE, README, QUICKSTART, SECURITY,
  CONTRIBUTING, and the systemd unit, and writes a SHA-256 checksum. x86_64 is
  built and smoke-tested. aarch64 is not built on this host because the Rust
  target and `aarch64-linux-gnu-gcc` are absent; the prerequisite is
  `rustup target add aarch64-unknown-linux-gnu` plus the cross linker. No
  aarch64 artifact is claimed.
- `scripts/public-scan.sh` and its `scripts/secret-scan.sh` wrapper scan the
  tracked tree and git history for credential patterns and for a private
  deny-list kept outside the tree. The deny-list path comes from
  `HEARTH_PUBLIC_DENYLIST` (default `~/.hearth-public-denylist`). Matches are
  redacted, the deliberately fake test harness and fixture paths are excluded,
  and the deny-list itself never enters the tree.
- The prepared `.github/workflows/ci.yml` runs `scripts/ci.sh` and the secret
  scan. It is not pushed, and the deny-list is expected as a repository secret.
- The scan against a private list found the author email in `Cargo.toml` and
  `DECISIONS.md`, and the same metadata across the git history. The local tree
  keeps the chosen author metadata for now; the public release needs neutral
  author metadata and a clean-history rewrite, which is a separate decision. No
  credential or Telegram id was found.
- Backup, restore, service install, live doctor, and interactive init are M8a.
  The soak, mid-stream crash point, optional tools, and the release acceptance
  list remain for M8c.

## 2026-11-09: M8c-1 crash, credentials, and the bounded soak

- The `mid_stream` crash point aborts after a streamed delta and before the
  assistant entry is saved. The crash driver's `agent` command runs the real
  agent with a scripted provider; the S7 test expires the task lease, recovers
  the generation task, and retries the same submission. The admitted user entry
  survives, no partial answer is stored, and the retry produces exactly one
  answer.
- Credential detection is shared in `hearth_core::secrets`: `sk-`, `ghp_`,
  `xoxb-`, `akia`, private-key markers, `Bearer` tokens, bot-token shapes, and
  `api_key`/`apikey`/`token`/`password`/`secret` assignments. The memory writer,
  the core save path, the prompt blobs, the JSON spill, the agent's outcome
  text and notices, and the daemon log writer use it. Redaction replaces the
  whole credential token with `[redacted]` and preserves the surrounding text.
- The core save redacts entry content, submissions, documents, task inputs,
  checkpoints, results, effect payloads, and conversation titles. The memory
  writer refuses credential-shaped content at both the service and core layers.
- Logs are written through a redacting writer; tests cover the memory rows, the
  prompt audit, the transcript, the spill, the provider-error notice, and the
  log file.
- `scripts/soak.sh` runs a bounded local soak with a temp home, no Telegram
  block, repeated daemon start/stop, status, backup, restore, doctor, and the
  memory benchmark. It never touches `.devhome`.
- `scripts/acceptance.sh` runs the S7 crash suite, the scheduler tests, the
  fake Telegram end-to-end suite, the memory and token benchmarks, the soak,
  and the secret scan. The private deny list for publication stays outside the
  tree; the local acceptance uses an empty list so the credential-pattern scan
  still runs. The seven-day soak remains a future real-time item.

## 2026-11-09: M8c-2a documents and media tools

- The `documents` group now ships a real read-only extractor for PDF, DOCX, PPTX,
  and XLSX. PDF uses the existing `pdf-extract`. DOCX and PPTX read
  `word/document.xml` and `ppt/slides/slide*.xml`; XLSX reads shared strings and
  sheet XML. These use new minimal dependencies, `zip` 2.6.1 and `quick-xml`
  0.37.5. `calamine` was not added; the small XLSX reader is enough for text
  extraction and keeps the dependency surface smaller.
- Document reads may use the workspace, spill, or inbound-media roots. Absolute
  paths are accepted only by the documents/media workspace; the ordinary file and
  shell tools still reject absolute paths. The inbound media root is read-only.
- The `documents` tool extracts text. It does not create or rewrite PDF, DOCX,
  PPTX, or XLSX files yet; spec 9.2's "read and create" remains partly open for
  the M8 gate.
- The `media` group ships image generation, transcription, and speech. Image
  generation uses an OpenRouter Images-compatible `{base}/images/generations`
  client with b64 or URL responses; transcription reuses the Groq client; speech
  reuses the ElevenLabs client. Generated media is written under
  `media/outbound` at 0600. The media group is available only when at least one
  media block is configured; the actions that lack a service return a clear
  `media_not_configured` error.
- The old placeholder `ToolGroups::default_groups` was removed. The runtime
  builds the browser placeholder, the real documents group, and the configured
  media group; the browser remains an honest empty placeholder.
- The 100-turn token benchmark uses an empty optional-group registry because it
  measures core prompt cost, not optional tools.

## 2026-11-09: M8c-2b browser and document creation

- The optional browser group now ships a real `browser` tool against the local
  Camofox REST API. The base URL comes from `CAMOFOX_BASE_URL` and defaults to
  `http://127.0.0.1:9377`; tests use a local fake server and no live browser.
  Actions are open, read, click, type, and screenshot. The session label is the
  non-sensitive `hearth`. A snapshot is capped at 100,000 characters, and
  screenshots are written inside the workspace at 0600. A missing service is a
  retryable `browser_unavailable`; a timeout is `browser_timeout`.
- The `documents` group now creates minimal PDF, DOCX, PPTX, and XLSX files from
  a title and text content. Output stays inside the workspace, parent directories
  are created, files are 0600, and an existing file is truncated and replaced.
  Creation is deliberately plain: one page, one slide, or one sheet with the
  title and lines. The generated files round-trip through the extractor.
- No new dependencies were added for either feature. The PDF writer is minimal
  and local; DOCX, PPTX, and XLSX reuse `zip` and `quick-xml`.
- Browser, documents, and media remain lazy groups: their schemas enter a request
  only after `tool_search` selects the group, and the core tool budget is
  unchanged.

## 2026-11-09: Feedback slice 1 (sir's direct request)

- `identity.user_name` is removed. It only fed the default `SOUL.md`, and a real
  `SOUL.md` supersedes it. This is a breaking settings change: a file that still
  has `user_name` is rejected as an unknown field. The development home was
  updated. The default soul is now name-neutral.
- `models.session_header` stays. It names an optional request header carrying the
  conversation id for provider session affinity, cache routing, or tracing. The
  init path still sets `x-opencode-session` for opencode-go URLs; the docs now
  explain the option.
- The Telegram poller no longer sends a seen reaction. The typing indicator is the
  only acknowledgement; a reaction added noise to every message.

- ElevenLabs speech keeps the model's text verbatim, including square-bracket
  audio tags. The message and media tool descriptions and the agent runtime note
  now teach sparse Eleven v4 tags such as `[warm, composed voice]` and `[softly]`
  when TTS is configured. No SSML.
- Guidance files: `HEARTH.md` is read from the home, then every `AGENTS.md` from
  the filesystem root down to the workspace, most specific last. Each file is
  capped at 16 KB and the combined text at 32 KB. The prompt gets one
  `## Project instructions` system message after the operation layer. The files
  are read at agent construction so edits apply on the next run. Nested
  `AGENTS.md` files below the workspace are not auto-loaded.
- `SPEC.md` is bumped to Draft 0.8 to record this feedback slice. The user's direct
  request is the approval; no unapproved deviation is introduced.

## 2026-11-11: Shell timeout process group

- The shell tool now places the command in its own process group and kills the
  group on timeout. This closes the previously deferred process-group item: a
  grandchild that inherited the capture pipe can no longer delay a timed-out
  command. The group kill uses the system `kill` command, so no new dependency is
  added. The timeout regression test uses `sleep 5 & wait` so it exercises the
  grandchild case deterministically.

## 2026-11-11: Friendly setup, local chat, and service uninstall (sir-directed)

- `hearth init` prints the `@BotFather` bot-token steps and the `@userinfobot`
  numeric-id steps before asking for the token and id.
- Interactive init offers five provider choices: DeepSeek, OpenCode, OpenCode Go,
  Command Code, and another OpenAI-compatible endpoint. A preset supplies the base
  URL for the first four; the last choice asks for one. After the API key, init
  prints a provider-specific key hint.
- Init then calls the OpenAI-compatible `GET {base_url}/models` with a 15-second
  timeout, prints the sorted unique ids, and accepts a number or a typed name. A
  failed or empty list falls back to a model-name prompt. Non-interactive init is
  unchanged and makes no model-list call.
- The timezone prompt is gone. Interactive init stores `TZ`, the `/etc/localtime`
  target, `/etc/timezone`, or UTC. An explicit `--timezone` still wins.
- The image block asks for an OpenRouter key and names OpenRouter. The
  transcription and speech prompts name Groq and ElevenLabs and link their key
  pages.
- `hearth chat` renders a banner, a `you>` prompt, streamed answer deltas, tool
  names, and a per-turn duration, token, and cost line when both stdin and stdout
  are terminals. Piped mode stays answer-only. Commands are `/help`, `/new`, and
  `/exit` (or `/quit`).
- `hearth service uninstall` stops, disables, and removes the user unit, then runs
  `daemon-reload`. It keeps the home and database. `uninstall_at` takes a flag so
  tests never touch the host.
- No new dependencies. `list_models` reuses the existing blocking reqwest client,
  and the chat UI uses the standard library and ANSI escapes.

## 2026-11-11: Operation layer removed (sir-directed)

- The fixed `## Operation` system text is removed. The first system message is
  `SOUL.md` alone.
- Rationale: the text duplicated the default soul ("plain and brief", "use tools
  when they help", "never reveal secrets") and the `message` tool description
  (send files, voice only when asked). Local chat has no message tool, so that
  sentence was inapplicable there. Secret handling is enforced in code.
- `SEGMENT_OPERATION`, the `operation` field in `TokenCounts`, the prefix parts,
  the audit document, the token benchmark totals, and the `hearth prompt --system`
  output are removed. The spec keeps L2-L7 for tools through runtime, and project
  guidance becomes L1.
- `request_from_document` still replays an `operation` field when it appears in a
  prompt blob saved before this change, so `hearth prompt --wire` stays faithful to
  the stored record.

## 2026-11-11: Default thinking and XML prompt sections (sir-directed)

- `models.thinking` is optional: `default`, `off`, `low`, `medium`, or `high`.
  Absent or `default` sends no thinking field and leaves the provider unchanged.
- Interactive init asks for the level immediately after the model list. The
  `--thinking` flag covers the non-interactive path, and init validates the value
  before writing anything.
- The OpenAI-compatible adapter sends `reasoning_effort` for `low`, `medium`, and
  `high`. `off` sends `reasoning_effort: none`, except on DeepSeek base URLs,
  where the adapter sends `thinking: {"type": "disabled"}` because DeepSeek's
  OpenAI format disables thinking with its own toggle. The logic lives in the
  adapter body, so chat, the daemon, doctor, and `hearth prompt --wire` agree.
- The prompt section wrappers are XML tags rather than Markdown headings:
  `<project_instructions>`, `<standing_memory>`, `<skills>`, `<relevant_memory>`,
  and `<summary_of_earlier_conversation>`. The default `SOUL.md` template uses
  `<identity>`, `<voice>`, and `<rules>`. Existing `SOUL.md` files are user content
  and are not rewritten.
- `request_from_document` rebuilds the XML wrappers for saved prompts; a saved
  `operation` segment is still replayed when present.

## 2026-11-11: Short placeholders are not redacted

- Exact-value redaction of configured credentials, in both `hearth prompt` and the
  daemon event bus, now ignores values shorter than 8 characters. A value of `-`,
  `none`, or `1` is a placeholder; replacing every occurrence corrupted ordinary
  prompt and event text.
- The threshold is `hearth_core::secrets::MIN_SECRET_LENGTH`, shared with the
  existing credential-shape checks.

## 2026-11-11: Trimmed CLI (sir-directed)

- The top-level commands are `init`, `daemon`, `status`, `prompt`, `doctor`,
  `service`, and `skills`. `chat`, `watch`, `stats`, `backup`, `restore`, and
  `eval` are removed.
- `hearth daemon` now has subcommands: `start` runs the long-lived agent (the old
  bare `hearth daemon` behavior) and `watch` follows live events (the old bare
  `hearth watch` behavior).
- The systemd unit runs `daemon start --home`.
- The chat and stats runtime modules are deleted. Backup and restore remain library
  functions; the daemon still writes nightly backups, but there is no CLI for them.
- Benchmarks run through `cargo test -p hearth-eval --test memory_bench` and
  `--test tokens_bench`; the acceptance and soak scripts use those. The CLI no
  longer depends on `hearth-eval`.
- An already installed unit keeps its old command until
  `hearth service install --force` rewrites it.

## 2026-11-11: The TUI operator surface (sir-directed)

- The only user-facing entry is `hearth`, which opens a ratatui terminal
  application. `init`, `daemon`, `status`, `prompt`, `doctor`, `service`, and
  `skills` subcommands are removed; there are no user-facing flags or positional
  arguments. `HEARTH_HOME` still selects the home.
- `HEARTH_DAEMON=1` is the internal daemon child mode used by the TUI and the
  systemd unit. A TUI-spawned daemon is started through `setsid` (with a direct
  spawn fallback) so it survives the window closing. The PID comes from
  `hearth.lock`.
- First run opens an onboarding wizard and writes the home through the existing
  `init` path: Telegram, provider, API key, model list, thinking, agent name,
  optional media, start mode, and keep-running. `init` is silent and returns an
  `InitReport`; the interactive prompt path and `init_with_io` are removed.
- Conversations stay in Telegram. The TUI is a control plane: Overview, Activity,
  Doctor, Skills, Prompt, and Settings. Activity uses the daemon socket; Prompt
  reads the newest saved prompt read-only; Skills reads the skill library
  read-only; Doctor calls `doctor_with`; Settings edits the daemon section (superseded
  by the TUI customization surface below, which edits every setting).
- `settings.daemon.start` is `tui`, `login`, or `startup`; `keep_running` is a
  boolean. `save_daemon` updates that section atomically with mode 0600 and
  preserves the rest of the file. Startup uses `loginctl enable-linger`.
- The systemd unit sets `HEARTH_DAEMON=1` and `HEARTH_HOME` and runs the binary
  with no arguments. Service commands capture output so they never corrupt the
  TUI. A service-managed or attached daemon is never stopped by TUI exit.
- New dependencies: `ratatui` 0.30 and `crossterm` 0.29. Ratatui 0.30.2 requires
  Rust 1.88; the workspace is 1.89. Both are MIT-licensed and already needed for
  a terminal interface.
- Removed along with the commands: `prompt_audit`, `request_from_document`, the
  `doctor` print wrapper, and the `skills` CLI formatter. The runtime prompt
  assembly, skill library, doctor checks, and backup/restore remain.
- `scripts/soak.sh` and `scripts/acceptance.sh` start the daemon with
  `HEARTH_DAEMON=1` and move status/doctor/backup checks into cargo tests.

## 2026-11-11: TUI usability and thinking levels

- The onboarding tagline is "A durable personal agent, built with Rust." The
  shared theme uses a slate/cyan palette with readable muted text, brighter
  borders, a distinct field fill, and a high-contrast selection row. No view sets
  a foreground to `DarkGray` or relies on the terminal's default border color.
- Onboarding fields are labelled rows with their own fill and focus state. The
  cursor column is measured from the rendered prefix with `unicode-width`, so it
  stays on the insertion point while typing and for long values. The wizard
  enables bracketed paste, accepts crossterm paste events, and tells the user to
  paste with Ctrl+Shift+V or Ctrl+V where the terminal forwards it.
- Thinking levels add `xhigh` and `max` after `high`. They are sent as
  `reasoning_effort` values verbatim; availability stays a provider concern. The
  config and init validators, onboarding list, review, and docs share the same set.
- Activity coalesces adjacent text and thinking deltas per task, shows local
  timestamps and type badges, and renders a readable event detail instead of raw
  JSON. Overview replaces the vague "Work" card with "Tasks & memory", removes the
  instructional card, and adds a separate Memory card. The Daemon card and the
  Settings daemon panel both show the configured thinking level.
- The Prompt tab renders each saved prompt layer with a header, token count, and
  text gutter, including tools, relevant memory, and transcript. Raw JSON remains
  available.
- Stopping a TUI-owned daemon waits for both the socket and the writer lock to be
  released. The daemon keeps its status socket open until the poller, worker,
  scheduler, helpers, and effect worker have joined, then removes the socket and
  releases the lock. `ensure_with` waits for an existing lock before spawning so a
  quick close and reopen cannot race the previous daemon's shutdown.
- New direct dependencies: `time` for local event timestamps (already used by the
  runtime) and `unicode-width` for exact cursor placement (already in the lockfile
  through ratatui). Both are existing workspace crates, not new services.

## 2026-11-11: TUI customization surface

- The TUI is the complete customization surface. No file editing is required for
  normal use. A built-in text editor with line numbers, bracketed paste, `Ctrl+S`,
  and an unsaved-change guard edits `SOUL.md`, `HEARTH.md`, `settings.json`, and
  user skill files. Writes are atomic, private, and mode 0600.
- The Files tab reads and edits the resolved `SOUL.md` (including
  `identity.soul_file`) and the home `HEARTH.md`. `f` switches files, `e` opens the
  editor, and `o` reloads. The view styles XML sections, Markdown headings, and
  bullets so the files are readable without an external editor.
- The Settings tab keeps the daemon controls and adds "All settings…" and an `e`
  shortcut. The full-screen form presents every settings section with friendly
  labels, descriptions, masked secrets, friendly choices for start mode, thinking,
  and log level, and inline editing. Missing default sections are shown from the
  documented defaults. Optional media services can be added from the form; a
  service is kept as an unsaved draft until its required values are complete.
  Missing advanced maps and brand-new keys remain available through the raw
  `settings.json` editor. Every edit is parsed as `Settings`, validated, and only
  then written. The running daemon picks up changes after a restart.
- The Memory tab is read-only and uses a new `inspect::list_memories` query over a
  read-only SQLite connection; the TUI still never opens a writer. It shows kind,
  status, standing, importance, source, tags, timestamps, and the full content, and
  supports incremental text filtering.
- The Skills tab keeps the read-only learned library but can create a new user skill
  and edit an existing user skill. Learned skills stay agent-managed. A new skill
  starts from a small Markdown template and is created only when saved.
- The tab bar grows to eight entries and wraps to two rows on narrow terminals.
  The help overlay documents the customization shortcuts.
- The daemon status now includes a `usage` object with `today`, `week`, `month`,
  and `total` periods. Each carries input, output, cache-read, and cost totals; the
  legacy today fields remain for compatibility. The Overview Usage card presents
  the four periods as a compact table.
- Tests cover the editor cursor and save path, all-settings form labels and
  validation, optional media setup, friendly setting values, memory inspection and
  rendering, file editing, user-skill creation, and the new tab routing.

## 2026-11-11: Doctor live checks send a session id

- The Doctor live provider check now sends a generated session id. Providers that
  configure `models.session_header`, such as OpenCode Go's
  `x-opencode-session`, require the header on every request. The daemon and
  compaction/review paths already supplied one; the Doctor path did not.
- A fake-server regression test asserts that a configured session header reaches
  the provider request.

## 2026-11-11: Settings defaults and web provider form (sir-directed)

- `identity.soul_file` defaults to `SOUL.md`. A settings file that omits it still
  resolves to `$HEARTH_HOME/SOUL.md`, and `init` now writes the field explicitly so
  the default is visible. The Files tab already resolved the same fallback.
- `agent.max_turns` defaults to 120 instead of 30. The settings form uses the same
  default, so a missing value and a fresh home agree.
- The all-settings form no longer exposes `web.search.order` or `web.fetch.order` as
  comma-separated text. Each known provider is a row: Enter turns it on or off,
  Shift+Up and Shift+Down move it, and the last enabled provider cannot be turned
  off. This makes coloring outside the provider list or deleting the chain by a typo
  impossible from the form. The raw JSON editor remains available for advanced use.
- Web search and fetch API keys have masked rows for every supported provider even
  when the `providers` map is absent. Saving a key creates the nested provider object;
  the existing validation still accepts providers without a key and skips them at
  runtime. The Exa row notes that a key is optional.
- Editing any setting opens a bordered "Tip" tooltip with field-specific guidance,
  examples, and valid ranges. Vague numeric and path settings now name their units
  and limits, and secret rows explain that blank means keep the current value.
- Tests cover the two defaults, init writing `SOUL.md`, provider toggling and
  reordering, the last-provider guard, web key creation, and the editing tooltip.

## 2026-11-11: Model requests keep provider defaults (sir-directed)

- Hearth no longer sends `max_tokens` on any model request. This covers the main
  agent loop, compaction summaries, memory extraction, skill review, and the live
  doctor check. The provider and model keep their own output limit.
- No sampling parameters are sent: temperature, top-p, top-k, penalties, stop, and
  seed stay absent. The only generation control Hearth sets is the reasoning level
  from `models.thinking`, as `reasoning_effort` or DeepSeek's `thinking` toggle
  for off.
- `models.max_output_tokens` remains as a local reserve for the context budget. It
  is not sent to the provider; renaming the schema would break existing settings, so
  the form label and docs now call it an output reserve.
- The budget formula is unchanged: reserve = max(configured_output, 4096) + 1024,
  usable = window - reserve. It still prevents Hearth from filling the whole context
  with the transcript before a reply has room.
- `ChatRequest.max_tokens` remains an optional adapter field, so a future caller or a
  recorded fixture can still send it. No production runtime path sets it.
- Tests assert that main, extraction, compaction, skill-review, and doctor requests
  carry `None` for `max_tokens` and that the doctor wire body has no `max_tokens`
  field.

## 2026-11-12: Inline settings, compact saves, clearer Overview, faster close

- The Settings tab is now the last tab and renders the complete form inline. No
  `e` shortcut or separate daemon panel is needed. The form keeps daemon start and
  keep-running rows, adds start, stop, and restart actions, and adds Edit SOUL.md
  and Edit HEARTH.md actions. The Files tab still reads and edits both files.
- Changed rows carry a `*` marker. `r` resets the selected row to its canonical
  default; order rows restore the default provider order, and optional values with
  no default are removed. Fields still validate before a reset is committed.
- `hearth_config::default_settings_value` and `hearth_config::compact_settings`
  are the single authority for defaults and omission. Settings saves, init, and
  `save_daemon` write only values that differ from the defaults. Required model
  fields (`kind`, `base_url`, `api_key`, and `model`) stay even when they match a
  default so the file still deserializes. `models.thinking = "default"` is stored
  as absent.
- The daemon-section migration no longer writes a default daemon section when no
  unit is installed. A start-mode change from the form still applies the service
  mode asynchronously; other changes say a restart is needed.
- Overview now uses five cards: Runtime, Usage, Work & memory, Connections, and
  Recent activity. Usage includes cache reads and hit rate; Runtime names the
  provider, context, turn limit, and workspace; Work includes helper limits,
  memory budgets, consolidation, and skill-review changes; Connections lists
  Telegram, search order, fetch order, and media services.
- The Telegram long poll is 2 seconds with a request timeout two seconds above the
  poll timeout, so an idle daemon joins its poller promptly on SIGTERM. TUI exit
  now sends SIGTERM to a daemon it spawned and returns without waiting for the
  writer lock. The explicit Stop and Restart actions still wait for a clean stop.
- Tests cover inline Settings rendering, file actions, changed markers, reset,
  default-order reset, compact settings output, compact init output, tab order, the
  new Overview cards, and nonblocking TUI exit.

## 2026-11-12: Settings and onboarding polish

- The timezone row opens a picker. The options come from the system zoneinfo tree
  (`/usr/share/zoneinfo`, `/etc/zoneinfo`, or the macOS path) plus the current value
  and UTC; a small common-zone list is the fallback when the tree is absent.
- The footer shows the selected row's full tooltip before editing, not only while
  editing. The short `help` string remains on the row for callers that do not render
  the footer.
- The Files tab is removed. `SOUL.md` and `HEARTH.md` are still edited from the
  Settings actions, which are the only remaining entry points.
- `models.context_window` is labeled local, like the output reserve. It is never
  sent to the provider.
- models.dev is the optional source for the context window and token prices. The
  settings action fetches the public `https://models.dev/api.json` catalog, matches
  the configured `models.base_url` and model id (model ids alone are ambiguous
  across providers), and fills only the context window and `models.pricing`. It
  leaves the local output reserve unchanged because that value is a reserve, not
  the provider's output ceiling. Onboarding looks up the selected model in the
  background and carries any result into `InitOptions`; a failure or missing entry
  falls back to the existing default. No lookup blocks startup or a conversation.
- `http` and `https` remain aliases for the one direct fetch provider so existing
  settings files keep parsing, but the settings picker offers only Direct HTTP.
- `tasks.eta_reminder_minutes` was unused. It is replaced by
  `tasks.eta_reminder_seconds` (default 1) and the helper reminder now waits
  `expected_seconds + eta_reminder_seconds` before sending. The old key still
  deserializes as an ignored compatibility field; it is not shown and not used.
- Resetting a draft media service prunes an empty service object and shows its
  Add action again instead of leaving an invisible empty section.
- Onboarding skips the key/model rows of a disabled optional service, and Up/Down
  move between enabled rows only.

## 2026-11-12: Default budgets, lookup defaults, and tab order

- `models.max_output_tokens` defaults to 32,000 and
  `compaction.keep_recent_tokens` defaults to 20,000. Both remain local budgeting
  values; the output reserve is still never sent as an API output cap.
- The models.dev lookup now also replaces the form's default baseline for the
  fetched context window and prices. The fetched values are written to the document,
  are not marked as changes, and `r` resets them to the fetched values. The last
  lookup is cached at `$HEARTH_HOME/cache/models.dev.json` for the current base URL
  and model, so the reset baseline survives reopening Settings and restarting; the
  onboarding lookup writes the same cache after init. When no matching cache exists,
  `r` on the context window or the current model's prices fetches models.dev and
  applies the result. The output reserve is not taken from the catalog because it is
  a local reserve rather than the provider's output ceiling.
- The Settings footer adds one blank line above the selected row's tip.
- The tab order is Overview, Activity, Memory, Skills, Prompt, Doctor, Settings;
  numeric shortcuts follow that order.

## 2026-11-12: Prices are USD per million

- `models.pricing` values are now USD per million tokens in settings and in the
  Settings form. The internal `Price` type and usage ledger still use integer
  micros so cost arithmetic stays exact; the config conversion happens once in
  `pricing_from_settings`.
- `PriceAmount` is an untagged config value: a decimal is USD, while an integer is
  a legacy micros-per-million value. This keeps existing settings and old cached
  lookups correct without a version field. The form normalizes legacy integers to
  USD on open.
- Onboarding, the models.dev lookup, and the lookup cache write USD values. The
  cache converts back to micros when loaded so the reset baseline keeps exact
  internal prices.
- Help text and docs show examples such as `0.30`, not `300000`.

## 2026-11-12: Settings cleanup and a fixed soul path

- The Settings form no longer repeats the key hints; the shared footer carries
  `↑↓`, `enter`, `r`, and `q` once. The form footer keeps the blank line and tip.
- Resetting a media row keeps the row visible. Media fields with no static default
  become an explicit null in the form instead of being removed, so the service rows
  do not vanish while the user is editing; required fields still validate and revert.
- The `Web search and fetch` header is removed because it sat immediately above the
  `Search order` header. The web rows start directly with `Search order`.
- `identity.soul_file` is gone. SOUL.md is always `$HEARTH_HOME/SOUL.md`; the
  daemon, file editor, and prompt loader no longer consult a configurable path.
  `hearth_config::load` and the settings form strip the old key, and
  `compact_settings` drops it, so existing homes keep loading without a manual edit.

## 2026-11-12: Prompt audit, activity time, and cost accounting

- `latest_prompt` skips prompt blobs that are not JSON. Compaction, memory
  extraction, and skill review also store plain-text prompts in `prompt_blobs`;
  selecting the newest row blindly made the Prompt tab (and its Overview banner)
  fail with `saved prompt is not JSON`.
- The Prompt view stores the rendered scroll range on the app view and clamps
  Down/PageDown/End to it. Switching raw/system resets the scroll to the top, and a
  newly loaded prompt does the same.
- Overview recent activity shows local `HH:MM:SS` before the event kind.
- Cost accounting no longer charges cached prompt tokens twice: OpenAI prompt
  tokens include cached tokens, so the fresh part pays the input rate and the
  cached part pays the cache rate. When a reasoning price is configured, the
  reasoning subset is removed from output before both are priced. The internal
  ledger remains micros; the displayed cost is still USD.

## 2026-11-12: Remove pricing and cost tracking

- `models.pricing` is gone from the settings schema and the Settings form. A legacy
  `pricing` block is stripped on load and omitted on save by `compact_settings`.
- `Price`, `Pricing`, `PriceConfig`, `PriceAmount`, and the models.dev cost parsing
  are removed. The models.dev lookup now fills only the context window and output
  limit.
- Usage events keep input, cache read, cache write, output, reasoning, and latency.
  `cost_micros` is removed from `NewUsage`, `UsageEvent`, `UsageTotals`, the SQL
  statements, and the daemon status JSON.
- Migration 0005 drops `usage_events.cost_micros`; `SCHEMA_VERSION` is 5.
- The TUI no longer shows a cost column in Usage or cost metadata in Activity.
  Onboarding and the lookup cache no longer carry prices.

## 2026-11-12: Tool pairing, clean memory text, and daemon controls

- A steering user entry can be saved between an assistant tool-call entry and its
  tool-result entry. Prompt assembly now pulls matching results directly after the
  assistant call, keeps the steering entry after the result, and drops unresolved
  calls or orphan results. The audit transcript uses the same repaired order, so
  an OpenAI-compatible provider never receives an unanswered `tool_calls` message.
- Guidance keeps the home `HEARTH.md` text as the unlabeled base layer and labels
  each project `AGENTS.md` with `### Project rules`. Absolute file paths are no
  longer written into the prompt.
- Standing and relevant memory render only the remembered text, one `- ` bullet per
  item. IDs, kinds, dates, tags, and importance stay in the memory store, tool
  output, extraction prompt, and Memory tab, where they are useful, and no longer
  spend prompt tokens. Recall budgets count the rendered bullet.
- The Settings daemon controls follow the live state: Start while stopped, Stop and
  Restart while starting, running, or stopping. An attached daemon can be stopped
  and a start-mode change restarts the daemon so the chosen mode owns it. The
  settings form rebuilds those rows after every state change, and the in-memory
  daemon section follows a migrated `settings.json` section so saving another
  setting does not re-apply the start mode.
- The Prompt audit drops tree guides, XML tag labels, and closing-tag rows. Sections
  use a compact accent heading, indented bodies, role labels, and spacing between
  messages; the raw JSON view still shows every tag and field.
- Daemon settings migration no longer rewrites the installed systemd unit. It
  writes the missing `daemon` section and leaves the existing unit alone, so
  opening the TUI from a test or ad-hoc build cannot capture the unit and point
  it at the wrong executable or home.

## 2026-11-12: Prompt two-card audit

- The Prompt tab always shows the latest saved request and uses two bordered
  cards like Activity. The left card is a one-line list of wire items in exact
  request order: system identity, guidance, standing memory, skills, every
  transcript item with relevant memory immediately before the newest user, the
  runtime note, the request `tools` field, and the saved response. The right card
  always shows the selected item in full.
- Up/Down select, and PageUp/PageDown, Home/End, and the scroll keys move the
  detail. Tool definitions show compact `args` fields such as
  `path: string (required)` and enum choices instead of pretty-printed schema
  JSON. Tool-call arguments render as `key=value · key=value`. Raw JSON remains
  available with `r`.
- The row model is cached by request, width, response revision, and older request
  for change markers. The selected detail is cached separately by selection.
  Rendering slices only visible left rows and visible detail lines and never asks
  Paragraph to rewrap the whole prompt, so selection and scrolling stay
  responsive on large prompts.
