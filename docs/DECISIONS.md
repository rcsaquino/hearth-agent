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
  excludes the identity and runtime system messages. (The runtime note now rides
  with the newest input message; see 2026-11-16.)
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
- There is no turn limit. A loop ends when the model answers without a tool call,
  when the context budget gate fails, or when the run is cancelled. `agent.max_turns`
  is retired: `a_long_tool_loop_is_not_cut_short_by_a_turn_limit` runs 125 tool
  rounds and requires all of them to complete, and `agent_has_no_turn_limit_setting`
  keeps the setting out of the schema. An unbounded tool loop is the accepted
  design, confirmed on 2026-11-16.
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
- Segment budgets follow 6.1. The fixed overhead is identity + operation + tools + standing + skills + runtime. The transcript takes the remainder. The prompt audit document records per-segment counts plus total and fixed. (The per-segment warning thresholds and the fixed-overhead target were removed on 2026-11-16; see that entry. The audit document still records the counts.)
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
- A helper runs on the configured `ContextBudget`, the same one the daemon builds
  for the main agent. `wall_seconds` (3600 default) cancels the run at the next
  agent boundary. There is no per-helper token budget and no separate hard
  total-token meter in v1.
- Helper agents skip post-run memory extraction; their working clutter must not
  become long-term memory.

## 2026-11-09: Skills (M7c-1)

- User-made skills live in `$HEARTH_HOME/skills/` as `SKILL.md` inside a named directory or as a flat `<name>.md`. They are read-only to Hearth. Learned skills live in `skills/learned/<name>.md` with frontmatter, versions in `skills/learned/.history/<name>/vN.md`, and a `CHANGELOG.md`.
- Frontmatter is parsed conservatively without a YAML dependency. Missing closing fences, unknown statuses, and non-numeric counters are errors. User skills may omit frontmatter; the file name and first body line provide the name and description.
- The prompt skills index contains active skills only, capped at 300 estimated tokens and 50 active skills across user and learned. Learned skills over the cap are archived by lowest success rate, then least recently used; user skills are never archived. The index is the L4 prompt layer, the M5a `skills` segment, and the M5c `skills` prefix part. (The token cap was removed on 2026-11-16; see that entry.)
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

## 2026-11-16: The Memory tab groups by pin state

- `inspect::list_memories` orders by one `CASE` group key: current pinned rows
  first, current unpinned rows second, and rows with a closed window last. Within
  a group the keys stay `updated_at` descending and `rowid` descending. The old
  order had two groups because every unpinned row and every retired row shared
  the recency key, so a recently updated retired row could lead a current fact.
- The group key lives in the query, not in `MemoryView::filtered`. The query
  applies the 1,000-row limit, so sorting only the returned vector could not
  bring back a current row the limit had dropped. Filtering preserves the query
  order and never moves an active row below an inactive one.
- The status flags, glyphs, `(retired)` label, and detail pane are unchanged. A
  row that was pinned before its window closed is inactive, not pinned, because
  the query already clears the displayed pin for it.
- Sorting by content, kind, subject, or status text was rejected: the only keys
  are the group and the existing recency order.

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

## 2026-11-13: Live session export in the Prompt tab

- The Prompt tab is now one scrollable session document instead of the two-card
  audit. The newest saved exchange renders in full with the stable layers first:
  system identity, guidance, standing memory, skills, request tools, then relevant
  memory, every transcript item, the runtime note, and the saved response. Usage
  is shown only when the exchange has a linked usage record; today the inspect
  view has no reliable link, so the row is omitted. Tools are a top-level request
  field, but placing them with the fixed layers makes
  the stable prefix and its cache behavior easy to see. Sections are not collapsed
  and tool results are not clipped; `r` still shows the raw exchange JSON.
- Up/Down scroll one line, PageUp/PageDown move a page, Home/End (or `g`/`G`) jump
  to the top or tail, `[`/`]` walk older loaded snapshots, and `f` toggles live
  follow. Scrolling away pauses follow; End or reaching the bottom resumes it. The
  header names the snapshot, live state, prompt and prefix hashes, token counts,
  and cache peers.
- The view refreshes when the open Prompt tab sees an `assistant_reply` or
  `run_settled` event. A refresh that arrives while one is already running is
  coalesced into one follow-up read, so the final saved answer is not missed. A
  paused reading position survives the reload, and the scroll range is clamped
  after every cache rebuild.
- Rendered sections are cached by saved prompt hash, request time, response entry,
  raw mode, and width. Wrapping preserves leading indentation instead of flattening
  code and JSON, and schema fields include argument descriptions.

## 2026-11-13: Memory phase 0, bounded recall, and the hybrid target

- The approved recall target is 100% obtainability, not 100% automatic
  injection: every stored memory must be reachable through the indexes or a
  tool, while automatic recall remains a bounded cache. The benchmark targets
  automatic recall@5 of 0.95 or better and one-tool recovery for misses.
- Automatic recall now uses `memory.recall_max_items` and
  `memory.recall_max_tokens` from settings instead of a hardcoded limit. The
  default budget is 8 items and 900 tokens. Each item is clipped to a 240-character
  snippet, and tags and ids are omitted from the injected text. The model reads
  full content through `memory_search`.
- New and edited long-term memories are limited to 2,000 characters and 8 tags
  through the tools. The 20,000-character storage cap remains for archived or
  imported rows. Standing bullets are also limited to 2,000 characters through
  the tool path. (Superseded on 2026-11-16: every memory has one 150-character
  limit, and recall defaults to 700 tokens and 10 items. See that entry.)
- Credential-shaped content is rejected before it reaches the long-term or
  standing store, using the shared `hearth_core::secrets` matcher.
- The daemon status exposes `memory_recall` with the mode, configured and active
  embedding flags, item cap, and token cap. Overview shows `hybrid` only when
  embeddings are active; otherwise it shows `lexical` or `lexical fallback`.
- Embeddings remain default-on with lexical fallback. The selected model is
  `bekko-embedding-v1-a8m` at 384 dimensions with INT8 vectors and a local model
  directory. The backend is not wired yet, so the status currently reports
  `lexical`.
- The next retrieval phase is hybrid FTS5 + local vectors with reciprocal-rank
  fusion, no remote embedding calls, and a hard fallback to lexical recall when
  the model is missing or slow. Selective extraction is deferred to explicit
  cues, compaction, or idle review; per-turn extraction stays rejected.
- The stale SPEC memory section was replaced with the current two-store design
  and the approved target. The old nightly consolidation status remains in the
  status JSON for compatibility but is no longer shown in Overview.

## 2026-11-13: Hybrid memory, local embeddings, and selective extraction

- The primary embedding model is `bekko-embedding-v1-a8m`, 384 dimensions, run
  locally through ONNX Runtime. Query vectors are computed on the turn path;
  memory vectors are precomputed at write time and stored as INT8 BLOBs in the
  memory database. The model is MIT licensed. The pinned files are fetched on
  first use by a release build, or explicitly by
  `scripts/fetch-embedding-model.sh`; `HEARTH_OFFLINE=1` disables the download.
  A missing or broken model selects lexical recall, not a daemon failure.
- The long-term memory schema is now version 2. `memory_vectors` stores
  `memory_id`, `model`, `dims`, `content_hash`, `vector`, and `updated_at`.
  Existing version 1 databases gain the table in place. Add and edit index the
  text; delete removes the vector; edits remove the old vector before reindexing;
  startup backfills missing vectors in a background thread.
- `MemoryStore::search` keeps the lexical modes (`all`, `phrase`, `literal`) and
  uses reciprocal-rank fusion for the default `any` mode when an embedder is
  attached. The vector side is a linear INT8 scan over a cached vector set. On the
  target i3-7100T, a8m query encoding measured about 2.5 ms for a short query,
  6 ms for 37 tokens, and 16 ms for 123 tokens; a 200k-row, 384-dimension vector
  scan measured about 205 ms, and a 10k-row scan about 10 ms. This keeps normal
  recall below 100 ms and the 200k floor under the 500 ms acceptable bound. A
  paraphrase smoke ("Who is Bob's boss?" against "Alice manages Bob") found the
  manager memory in about 3.5 ms.
- Automatic recall now renders 240-character snippets, omitted tags and ids, and
  stops at the configured item and token caps. Overview shows `hybrid` only when
  the embedding backend is active, otherwise `lexical` or `lexical fallback`.
- Selective extraction runs after delivery only when the latest user message
  contains a durable cue. It sends a bounded conversation tail to the configured
  model, accepts at most three one-line memories as JSON, records usage, and
  writes through the normal store checks. It is not on the reply path and is not
  a per-turn extractor. Compaction runs the same extractor over its source range
  before summarizing, so durable details survive the summary.
- Nightly backups now include same-date `memory-YYYYMMDD.db` and
  `standing-YYYYMMDD.md` sidecars. Restore copies them back when present, so a
  main-database restore no longer leaves the memory library behind.

## 2026-11-13: Transcript FTS and reachability guard

- `entries_fts` is an FTS5 external-content index over `entries.content_json`,
  with insert/delete/update triggers and a rebuild migration for existing
  transcripts. `memory_sessions` uses it for the `any` and `all` keyword modes;
  `literal` keeps the case-insensitive substring scan. If the index is absent or
  the query has no token match, search falls back to the scan so an old or
  unusual database does not lose reachability.
- The JSON itself is indexed. The tokenizer ignores punctuation and searches the
  decoded text values; this avoids a second shadow text column and keeps writes
  to the transcript unchanged.
- `hearth-eval/tests/memory_reachability.rs` stores 128 unique facts plus a
  vector-only fact, checks every direct term, browses every page, and verifies the
  vector-only paraphrase. It is deterministic and runs without the embedding
  model.
- Startup vector maintenance removes orphaned `memory_vectors` rows before the
  resumable backfill.

## 2026-11-14: Home recovery from settings.json

- `settings.json` is the only bootstrap authority. After taking `hearth.lock` and
  before opening the database, the daemon recreates the onboarding layout when it is
  missing: the eight folders, `SOUL.md` from `identity.agent_name`, and an empty
  `HEARTH.md`. Existing files and folders are never overwritten, so the pass is
  idempotent and quiet on a healthy home.
- The repair lives in `hearth_runtime::restore_home`. `init` and the repair share
  one layout definition (`home::scaffold`), so onboarding and recovery cannot drift.
- Repair is daemon-only. The TUI must not open a database writer, and the lock owner
  is the single process allowed to write the layout, so a systemd unit and a
  TUI-spawned daemon cannot race. The TUI already tolerates every missing artifact
  (skills list, memory views, identity editors), so nothing is user-visible before
  the daemon starts.
- `restore_home` never opens the database. Creating and migrating `hearth.db` stays
  the daemon's write through `Harness::open`.
- A record minted from zero migrations logs one warning that Telegram may replay
  recent updates. No offset sentinel is added: a fresh setup starts at offset zero
  too, a sentinel would be new state that itself needs recovery, and draining the
  pending updates would silently drop unread messages.
- `DoctorError::DatabaseMissing` now says the daemon creates the database on start
  instead of telling the user to run `hearth` from inside `hearth`.

## 2026-11-14: A unit manages only the home it names

- `service::status` reports a unit as installed for a home only when the unit's
  `HEARTH_HOME` names that home. Before, any unit on the machine made every home
  take the systemd path, so a recovered home holding a copied `settings.json` waited
  30 seconds for a daemon the unit was never running.
- The unit path calls `systemctl --user restart` instead of `start`. A unit that is
  still active can hold a daemon whose home was wiped underneath it, which is the
  recovery case this record adds, and `start` is a no-op there. Restart re-reads the
  home and settings; on an inactive unit systemd treats it as a start. The path only
  runs after the status probe fails, so a healthy daemon is still attached, never
  restarted.
- Unit `Environment=` parsing accepts our own quoted `HEARTH_HOME` line, a combined
  assignment line, and a path that contains spaces. A unit without a home is not
  installed for any home, so the TUI spawns its own daemon instead of guessing.

## 2026-11-14: Memory rebuilt as one store and a four-stage cascade

The tree held two memory systems. `hearth-memory` owned
`memory/long-term.sqlite` and `memory/standing.md` with five tools and a
lexical-plus-vector recall path. `hearth-core` and `hearth-storage` owned a
second, richer `memory_items` design that only tests wrote. This file documented
the second; the daemon ran the first. `memory_bench`, which this file and
`AGENTS.md` both name, was deleted with the earlier system, so the live store had
no measurement at all. The rebuild keeps one implementation and deletes the rest.

**One database.** Memory is a table in `hearth.db`. A statement shares the one
writer and one backup with the entry it came from. It does not share that entry's
transaction: memory is written after the turn, and `source_entry` is a soft
reference. Rolling a conversation save back because a memory write failed would
lose the user's message, which is the worse failure. Migration 0008 drops
`memory_items`, `memory_events`, the old `memory_fts`, and the old vector table,
and creates `memories`, `memories_fts`, and `memory_vectors`. Neither dropped
table had a production writer, so nothing is lost.

**No stored importance.** The old design stored an importance score and used it
for standing promotion, decay, and ranking. A stored importance is a number
nobody can calibrate, and producing it wants a model call at write time. Ranking
is computed at read time from evidence instead, and the only explicit control the
model has is `pinned`, whose meaning is unambiguous.

**No tags.** `subject` plus `kind` replaced a tag array. Tags were a second
vocabulary the model had to invent, they cost schema tokens on every request, and
the entity channel reaches the same rows with more precision.

**Append, never overwrite.** A replaced statement keeps its row and its window
closes. This follows the measured experience of mem0's single-pass ADD-only
extraction (docs.mem0.ai, April 2026 algorithm) and Zep's bi-temporal edges
(arxiv 2501.13956): overwriting at write time destroys the history that makes
temporal questions answerable, and reconciliation is where context gets lost.
Reconciliation here is explicit. The write result names an overlapping statement
about the same subject, and the model passes `supersedes` on the same turn.

**Zero memory model calls.** Writing, reconciling, ranking, and rendering are
deterministic. The agent is the model, and it is already making the turn's call.
The per-cue extraction call is replaced by a four-pattern deterministic extractor
that runs after delivery for microseconds. Compaction folds extraction into its
existing summary request as a second JSON key, which removes a whole second
request over the same range.

**Four tools, not five.** `memory_save`, `memory_recall`, `memory_forget`,
`memory_transcript`. No store selector, no `s_`/`m_` prefix convention, no tag
array. The family measures 767 schema tokens against 1,410. The shape is
superseded by the 2026-11-17 one-tool entry. The store and the handlers do not
change.

**Retrieval is a recommendation cascade.** Write-time map, candidate generation,
ranking, and a budgeted slate. Three cheap channels feed one pool of 128: FTS5
(strict then broad), subject exact and prefix, and INT8 vectors behind a 0.40
cosine floor. Fusion is reciprocal rank at k=60. Ranking is linear over six
features: fused relevance, subject match, term coverage, kind prior, still
current, and recency. The fill sorts by score per token, so a short decisive fact
is not displaced by a long one. This is the structure the recommender framing of
long-term memory recommends (arxiv 2610.34673), and the features are the ones its
ablation found load-bearing: pool position, age, and shared keys outweigh a
cross-encoder.

**Term coverage is the highest-value feature.** It compares with a prefix in
either direction, which is the cheap stand-in for a stemmer and is what lets
"queued" reach "queue". Without it, a question whose words are spread across a
statement and its neighbours lands on a tie.

**The digest reports coverage.** The uncached position before the newest user
message is the most expensive place in the prompt, so the digest is terse and
defaults to 300 tokens. A terse digest that hides what it left out teaches the
model that the subset is the whole store, so it names the hidden counts per
subject and the call that fetches them. Measured at 116 tokens at p95. (The
default is 700 tokens and 10 items since 2026-11-16; see that entry. The current
benchmark measures 139 tokens at p95.)

**An as-of read skips the vector channel.** The vector index holds current
statements only, so mixing today's meaning into a question about March answers the
wrong question. As-of reads stay lexical and entity-based.

**The vector floor is 0.40.** Without a floor the top vector hit enters fusion
with the same weight as the top keyword hit even when it is unrelated, which
measured as a real regression: paraphrase rose but temporal and contradiction
fell. At 0.40 the hybrid path reaches 0.99 recoverable and 0.97 automatic
against 0.99 and 0.945 for lexical alone.

**Rules are rows, not a file.** `kind = rule` and `pinned = 1` replaces
`standing.md` with its lock file, atomic replace, symlink check, priority
eviction, archive callback, and startup repair. The budget is enforced at the
write and the error lists the current rules, so nothing is silently evicted.
Ordering is by creation so an edit does not reorder the block.

**The fixture is a history, not a pile.** The committed benchmark planted two
live statements for one question in several places, which no ranking can resolve
and no conversation produces. The fixture now gives every topic consecutive
windows, and its `as_of` values were moved into the window of their expected
statement. The benchmark asserts the coherence it relies on.

**Measured result, lexical then hybrid, on 195 questions.** Recoverable in one
call 0.99 and 0.99; automatic 0.933 and 0.954; paraphrase automatic 0.65 and 0.85;
digest tokens p95 139; latency p95 6.4 ms and 11.3 ms on a 200-statement store.
Contradiction 15 of 15, duplicate writes 0, secret leaks 0. Every stored statement
is reachable by id, subject, direct term, and enumeration, which is asserted
separately. Six of the thirteen automatic misses are topics the fixture gives two
live answers to; the rest are paraphrases.

**Residuals.** Two of the 195 questions need more than one call, both paraphrases
with no shared vocabulary. The hybrid measurement needs the local model; without
it the paraphrase category falls to 0.65. The 4,000-row distractor run keeps
recall and latency inside the same targets, but a 200k store was not re-measured
after the rewrite, and the honest expectation is that the vector scan grows
linearly from about 10 ms at 4k.

**Reviewed before landing.** The subsystem was read end to end by an independent
reviewer on a different model once the suite was green, and it found five defects
that the tests had passed over. Two changed behaviour: the prompt layer and the
digest both emitted the `<rules>` and `<memory>` wrappers, so every live prompt
carried them twice, and the duplicate check matched on content alone, so a fact
promoted to a pinned rule returned the fact and the rule never entered the prompt.
Three were silent failures: the over-budget loop trimmed the rendered digest and
ate the closing tag and then the coverage line, the rules budget was checked
outside the write transaction, and provenance was a service-wide slot that stamped
helper writes with the user's entry. It also found three benchmark checks that
could not fail, one of them a supersession predicate that reduced to the
automatic-recall score, and it is why the transaction claim above reads as a soft
reference. Treat a green suite as evidence about what the tests cover, not about
the subsystem.

This entry supersedes the memory decisions of 2026-11-08 (M4a through M6e) and
the prompt layer names and memory limits in the 2026-11-09 entries. Those stay as
the record of what was decided then; section 7 of `SPEC.md` is what is true now.

## 2026-11-14: Publication and the remote (supersedes "no remote")

The 2026-11-08 ground rules said “No GitHub, no remote, and no hosted CI until
publication is explicitly approved.” Publication has already happened. `origin` is
`github.com/rcsaquino/hearth-agent`, the repository is public, it was created on
2026-11-11, and it carries the full history. The rule and the tree disagreed, and
a rule that everyone has already broken stops protecting anything: it only trains
the next reader to ignore the file. What remains is the part that is still true.

**A push is an approval, a rewrite is its own.** Adding commits needs explicit
instruction. Force-pushing or rewriting published history is never implied by a
request to publish, and it needs its own approval. `AGENTS.md` carries the same
wording, because that file is what a coding agent reads first.

**Scan before the history grows.** `scripts/secret-scan.sh` is the gate and
`HEARTH_PUBLIC_DENYLIST` names the list kept outside the tree. The list is absent
on this host, so only the credential-pattern scan runs; an empty list is a partial
check and is not evidence that history is clean.

**Known residual.** `aaa482e` committed `/root/.hearth-public-denylist`, a private
home path, into `AGENTS.md`. `787b331` removed it from the tree but not from
history, where it is still reachable and still public. It is a directory prefix
rather than a name, an id, or a credential, and the credential-pattern scan is
clean over the published history. Removing it means rewriting history and
force-pushing, which is the approval named above and not a step in this change.

## 2026-11-15: The unit and its enable link repair themselves

- `daemon.start` of `login` or `startup` now means both files exist: the unit and
  `default.target.wants/hearth.service`. The old behavior left the link to
  `systemctl --user enable`, so a test install, or an operator who deleted the
  files, had no path back except cycling the daemon controls.
- `install_at` writes the link itself, so the filesystem holds the whole state of
  "installed and enabled" and an install with `enable=false` still produces what
  systemd reads at boot.
- Three callers converge on the same pass: one synchronous pass at daemon start, a
  repeat every 60 seconds, and `daemon_client::ensure_with` before it decides how
  to start. `service::repair` re-reads `daemon.start` from `settings.json` on every
  pass, so a mode change is picked up without a restart.
- Repair is additive. It creates a missing unit and a missing or dangling link, and
  it never rewrites an existing unit or deletes anything. Deletion stays a user
  action through the Settings tab or `apply_start_mode`, which also drops a
  dangling link now that one can outlive its unit. A hand-edited unit therefore
  survives. Two shapes are left alone on purpose: a unit whose `ExecStart` points
  at a binary a later install moved, and a unit file that names no `HEARTH_HOME`,
  which may belong to another home. Either one takes a start-mode change from
  Settings, which rewrites the unit, or deleting it so the next pass recreates it.
- Repair writes files only. systemd loads a unit on demand and reads the enable link
  when `default.target` comes up, so the pass must not depend on a reachable user
  manager. The first shape called `systemctl --user enable` after writing the files
  and reported a failure with the files already correct, which the end-to-end run
  caught.
- Linger for `startup` is checked against `/var/lib/systemd/linger/<user>` before
  `loginctl enable-linger` runs, so a healthy host does not spawn a process every
  minute.
- `StartModeReport.repaired` says whether a pass changed anything, which is what the
  daemon logs, and `ServiceReport.created` says whether the unit content changed.

## 2026-11-15: Timestamps render in identity.timezone

- `identity.timezone` was stored and shown but never used. Every stamp came from the
  host offset through `UtcOffset::current_local_offset`, so setting it changed
  nothing. One `Zone` type in `hearth-core` resolves an IANA name and formats the
  instant as ISO 8601 with the offset in force at that instant,
  `2026-02-14T09:41:05+08:00`.
- Dependency: `jiff` 0.2 with default features off and `std`, `tz-system`, and
  `tzdb-zoneinfo`. It resolves DST per instant from the system tz database. A
  hand-written TZif reader would have to reproduce that, including the POSIX footer
  that governs dates past the last transition. It pulls `jiff-core` and nothing
  else, and the bundled tz database stays out of the build.
- The zone is resolved where the stamp is rendered, from `settings.json` through a
  tolerant `hearth_config::identity_timezone`, instead of being captured at startup.
  The terminal keeps one `Zone` in `App` and refreshes it whenever it reloads
  settings, and the prompt view's cache key includes the zone name so a save cannot
  leave the old offset on screen.
- What follows the zone: the runtime note, memory `created` and `updated` in the
  Memory tab, the activity list and detail lines, the Overview activity, the Prompt
  audit stamps, the nightly backup date, and the backup and skill-review wall-clock
  slots.
- What stays UTC: every stored instant, including the `created_at`-style columns,
  task checkpoints, `last_skill_review`, and the skill frontmatter. The record must
  not shift when a display preference changes. The recall digest keeps its relative
  age because the digest is paid on every turn, and the runtime note in the same
  prompt is its absolute anchor.
- The runtime note is `<runtime>Current time: <ISO>.</runtime>` and nothing else.
  It is the one uncached line in every request, so the conversation id and the
  model name, both already implied by the request, no longer ride there. The tags
  mark it as context, and it is appended to the newest input message rather than
  sent as a separate system message.
- The transcript tool's citation stamp and a bare `as_of` date follow the zone too.
  The tools are rebuilt for every message, so the zone is resolved per run and a
  settings change applies on the next message with no new state. A bare `as_of`
  date names that whole local day, so its boundary is the start of the next local
  day. The store predicate is start-inclusive and end-exclusive, so that boundary
  means "the state at the end of that day". A start-of-day boundary would hide
  everything said on the day the model asked about, and today's date is exactly
  what the model already has from the runtime note. The tool description grew by one
  clause to say so, which moved the four schemas from 767 to 776 tokens.
- `usage.today` and the cache hit rate start at local midnight. The card says "today"
  and the zone is one settings read away, so a `+08:00` user no longer sees the
  counter reset at 08:00. The week and month rows stay rolling windows from now,
  which the zone cannot affect. This is the one place where a number, not just a
  rendering, depends on the zone, so changing the zone re-reads the current window
  under the new one. Stored usage rows keep their UTC instants.

## 2026-11-15: A pin belongs to a live statement

- A write that repeats a pinned rule, word for word, replaces the row and retires
  the rule. Retirement clears the pin, so the statement leaves the rules block and
  stops spending the rules budget. The same words in a different role are a
  demotion, and the new statement's role decides the pin.
- The rules read requires a live row even though retirement clears the pin. A store
  written by an older build can still hold a pin on a closed window, and injection
  must not trust that flag.
- Migration 0009 clears the pin on those older rows. The migration is the one-time
  repair; the read predicate is what stops a stale flag from injecting a rule
  before a home migrates.
- A write that arrives already retired cannot be pinned. The import path can set
  `valid_to`, and a pin on a row whose window is closed from the start would be the
  same illegal state by another route.
- The Memory tab marks a retired row with a hollow circle and `(retired)` in the
  detail pane, so the operator surface does not show a closed window as a live
  rule. The row is still kept: `memory_recall(id)` and an `as_of` read reach it.

## 2026-11-15: An unpinned write cannot retire a pinned rule

- This replaces the demotion sentence in "A pin belongs to a live statement." A
  write that repeats a live pinned rule without naming it as `supersedes`
  returns that rule unchanged. It does not close the row, clear the pin, or
  insert a second current copy.
- Only an explicit `supersedes` retires a pinned rule. The caller that names the
  row has decided to replace it. An automatic extractor, a summary pass, or a
  repeated statement has not.
- The cause was the fingerprint lookup after the exact-duplicate check. The
  exact check includes kind and pin, so an unpinned copy fell through to a
  fingerprint-only lookup that closed the pinned row. The write now refuses to
  close a pinned row it was not asked to replace.
- The rules read still requires a live row, and retirement still clears the pin.
  Migration 0009 remains the repair for stores written before that rule.

## 2026-11-15: A submission with no message is refused

- The rule "a turn needs a request" lives at the submission seam. `Session::submit`
  refuses content that carries no request, using `carries_request` next to `parse_blocks`, so the definition of what counts as content is not repeated in a
  caller. The predicate is false for whitespace text and for thinking alone, which
  is exactly what `message_for_entry` would drop from the wire.
- The refusal comes after the `request_id` dedupe lookup, so a repeated request id
  still returns the original submission and never starts a second run (SPEC 4.3).
- The refusal is `CoreError::Invalid`, which is the existing vocabulary for input
  the record will not accept. No caller branches on the variant: the helper pool
  logs it and fails the task.
- The Telegram poller still refuses an update with nothing to answer before it
  reaches `submit_telegram`, because it owns the offset decision. That check makes
  the record refusal unreachable from Telegram today, so no replay loop exists; the
  refusal at the record seam is what keeps the invariant true for any later
  caller.
- An expanded runtime note was rejected. The note stays one clock line, and a turn
  is not answered when it carries no request.

## 2026-11-15: A fork pull request runs the scan it can run

- `scripts/public-scan.sh` still refuses to run without a deny list. The CI wrapper
  now refuses too, instead of writing an empty one, so a missing secret is a failed
  job rather than a clean report.
- GitHub does not expose repository secrets to `pull_request` runs from forks, so
  the workflow passes `HEARTH_FORK_PR` from `github.event.pull_request.head.repo.fork`.
  That one case writes an empty list, prints why to stderr, and runs the
  credential-pattern half. A push or a same-repository pull request without the
  secret fails.
- The alternative, treating an empty list as clean everywhere, is what made the
  check decorative. The alternative of failing every fork PR would block outside
  contributors on a secret they cannot have.

## 2026-11-15: Redaction cuts the token, not the word

- `secrets::redact_secrets` finds the credential's byte range inside a word and
  replaces only that range. A prefix counts only at a token start, so `task-id`
  and `risk-free` are ordinary text, and the punctuation around a real token
  survives, so a saved prompt blob stays parseable and the Prompt tab keeps it.
- A `key=value` or `key:value` pair replaces only the value and keeps the key and
  the separator. A `Bearer` token replaces only the token after it.
- A settings document that does not deserialize is reported as the section and
  field that failed, found by removing candidates and retrying the parse. The raw
  serde message carries the unexpected value, which can be a credential pasted
  into the wrong field, so it is never shown.
- A provider key that still holds a `${NAME}` reference counts as unset.
  `hearth_config::is_resolved` is the one test, used by both the search and fetch
  chains, so a placeholder never becomes a credential.

## 2026-11-15: The memory block honours its own budget

- `recall_within` computes the coverage lines before the statement fill and
  reserves their measured cost. The fill's per-line estimate is an estimate, so a
  final loop drops the lowest-ranked statement, then the longest coverage subject,
  until the rendered digest fits the configured budget. The alternative, a fixed
  reserve, let a long subject spend several times the budget.
- The rules check and the rules render measure the same line through `rule_line`,
  and a row that does not exist yet is costed with `store::ID_SAMPLE`, a
  placeholder with the shape of a real id. The old placeholder was twenty
  characters shorter, so every accepted rule undercounted and the block dropped
  rules the store had accepted.
- A pinned rule that names the row it replaces is costed without that row, because
  the replacement closes it in the same transaction. Refusing the replacement at
  capacity made a full block write-only by deletion.
- `memory_recall` with `as_of` and no topic now reads the store at that instant,
  and a recall by id renders `since` with `Zone::iso`. Both are the one timestamp
  rule in SPEC 6: every instant shown to the model is zoned ISO 8601.

## 2026-11-15: Bounds, guards, and ownership at the tool boundary

- A pooled read connection is owned by a guard whose `Drop` returns it, so a
  panic in a read closure no longer shrinks the pool toward a permanent block.
- A snapshot is vacuumed inside a 0700 staging folder and chmodded to 0600
  before the rename. SQLite creates the vacuum target with the process umask, so
  the old `.tmp` file was readable by every local user for the length of the copy.
- A write checks the deepest existing ancestor of its target before creating any
  directory. The old order created the parent first, so a symlinked directory let
  a refused write leave a tree outside the workspace.
- Error bodies are read through one 8 KiB cap. Only a few hundred characters
  reach a message, so buffering the rest let the peer choose the allocation.
  Document archives are read at most 32 MiB, a text file at most 16 MiB, and one
  uncompressed archive entry at most 16 MiB, which matters because deflate reaches
  roughly 1000:1.
- `ToolError::from(io::Error)` marks only a transient kind retryable. A
  deterministic failure such as invalid UTF-8 or a name that is too long now tells
  the model not to repeat the call.
- The browser applies the fetch tool's public-address guard to the URL the model
  names. The browser group exists for pages plain fetch cannot render, not for a
  second route to the host or a metadata service; `shell` is the deliberate route.
- One image entry is estimated at 12,000 characters, about 3,300 tokens, which is
  the worst case for the 1568-pixel long edge the adapter sends. The old estimate
  counted the path and let an image-heavy turn pass every local budget.

## 2026-11-15: Recovery, replay, and delivery after an unclean stop

- `Session::recover_interrupted` is the startup recovery. A new process owns no
  `running` task, so lease expiry is irrelevant there; every such task is
  recovered. The old lease-based `recover_tasks` remains for anything that needs
  to requeue an expired lease during a live process.
- A generation task whose submission is still `running` sends the submission back
  to `queued` and is retired. The worker then answers the message again. A second
  interruption settles the submission `unanswered`, writes a `notice` entry, and
  stops retrying, which is the poison rule applied to a run.
- A replayed pending tool call runs only if it is read-only. `shell`, `message`,
  `schedule`, `browser`, and `media` act outside the database without an effect
  row, so a replay answers them with `not_replayed` and the model decides whether
  to re-issue. The alternative, an effect row per tool call, is the larger
  design and is not needed while the deny-list is short and explicit.
- A reminder whose send may have happened is not retried (at-most-once). The task
  still finishes `done` and the person gets a notice to reschedule if it did not
  arrive.
- A Telegram send error that is a rate limit, a server failure, or a transport
  failure leaves the effect `unknown`, which the recovery path can pick up. A
  permanent request error still fails it.
- After the answer entry is saved and the submission settled, a failure to
  settle an absorbed submission or to finish the task is logged, not returned.
  Delivery follows the answer, not the task row.
- The log file rotates at 10 MiB into one `hearth.log.1`. A single previous copy
  bounds the disk without an unbounded family of files.
- `session.recover_interrupted` and `prune_prompt_blobs` write SQL directly, like
  the other recovery paths, because they must run before any run exists.

## 2026-11-16: The rules budget is 800 tokens

- `memory.rules_max_tokens` defaults to 800. The settings default and the
  `MemoryConfig` default are one number, pinned by
  `the_rules_budget_defaults_to_800` and
  `the_memory_default_rules_budget_is_800`.
- The write check and the render share `digest::rules_fit`, so a rule admitted by
  the write is a rule the block shows. The notice line and the joining newlines
  are inside the budget.
- `RULE_CHARS` stays 400 characters as the rule line's read-side guard. The
  400-character rule cap is gone; the guard only protects an older row, and every
  new rule uses the 150-character content limit.
- The prompt rules warning copy, `SEGMENT_RULES`, was removed on 2026-11-16; see
  that entry.

## 2026-11-16: A helper runs on the configured budget

- The task tool no longer takes `token_budget`. A value below the reply reserve
  produced `usable = 0`, and the first turn then failed with the context-size
  message even though the conversation was empty and compaction could not help.
  The argument also capped almost nothing else: a helper starts a new
  conversation and the fixed segments are paid in full on every turn.
- A caller that still sends the argument is accepted and ignored, so an old model
  transcript or a remembered tool shape does not fail the spawn.
- `wall_seconds` stays. It bounds a run by time and fails as `Cancelled`, which
  names the real cause.

## 2026-11-16: The runtime note rides with the newest input

- A standalone trailing system message is the strongest instruction position. A
  continuation turn whose only new content was the clock was answered as though
  the clock were the request, because the note was the last message in the request.
- The note is now appended to the newest user or tool message: the newest user
  message on the first turn and after steering, the last tool result on a
  continuation. It stays one tag-wrapped line, paid fresh every turn, with no
  conversation or model identifier.
- The audit document marks the message that carried the note with a
  `runtime_note` string. The Prompt tab prints the note under that message, so an
  operator can see that a continuation added a tool result and no user message.
- A conversation with neither a user nor a tool message has nowhere to carry the
  note; it is dropped rather than put back in instruction position.

## 2026-11-16: The recall subject list is bounded

- Recall considers at most `MAX_KNOWN_SUBJECTS` (4,096) distinct subjects, longest
  first. The list supplements the lexical channel: a statement about a dropped
  short name is still found by its words. Keeping the longest names keeps the most
  specific ones.
- Migration 0010 indexes `(LENGTH(subject) DESC, subject) WHERE subject <> ''`.
  Without it the query sorted every distinct subject on the first recall after a
  write; at 100,000 subjects that was 124.5 ms at p95. With it the query is 5.1 ms
  and the cold recall drops from 957 ms to 623 ms.
- The remaining 617 ms at 100,000 is the FTS channel and the ranker, not the
  subject list. The documented 100 ms gate remains measured on the 4,200-row
  store.
- `cargo test -p hearth-eval --test memory_scale -- --ignored --nocapture` is the
  measurement. `HEARTH_SCALE_SIZES` selects the sizes.

## 2026-11-16: The 200,000-row retrieval number

- `the_200k_store_is_measured` seeds 200,000 noise rows with distinct subjects and
  the committed fixture, then times the 195 fixture recalls. It is ignored by
  default and run with
  `cargo test -p hearth-eval --test memory_bench the_200k -- --ignored --nocapture`.
- On the reference host (Intel i3-7100T, two cores and four threads, 7.6 GB,
  Linux) the lexical path is 9.1 ms at p50, 221.3 ms at p95, and 937.3 ms at p99
  on 200,170 rows. The 4,200-row gate is unchanged at 100 ms.
- The p95 is not the vector scan. It is the lexical channel when a question shares
  a word with the noise, which the synthetic noise makes common. The hybrid vector
  scan at 200,000 rows is still unmeasured because backfilling 200,000 vectors is
  not a routine check. Linear extrapolation from about 10 ms at 4,000 rows gives
  about 500 ms at 200,000, so the 100 ms gate is not claimed for the hybrid path
  at that size.

## 2026-11-16: The saved prompt marks the runtime note's carrier

- `build_prompt` attaches the note to the newest user or tool message. The audit
  document now records that placement instead of storing the note as a peer of
  the segment keys. The carrier message gains a `runtime_note` string, and the
  top-level `runtime` key is gone.
- The document transcript stays the stored conversation without the note, so
  `tokens.transcript` still measures the history and `tokens.runtime` measures
  the note once. A reader adds the marker to the message it belongs to.
- The Prompt tab reads the marker. The conversation view prints it under the
  carrying message.

## 2026-11-16: The skills index has no token cap

- `INDEX_MAX_TOKENS` is gone. `SkillLibrary::index` lists every active skill up
  to the 50-active count. Only the count archives a learned skill, by lowest
  success rate and then least recently used; user skills stay read-only.
- `SEGMENT_SKILLS` was removed with the other prompt warning thresholds on
  2026-11-16; only the 50-active count cap remains. See that entry.
- The replacement tests assert that 15 and 40 long-description skills are all
  indexed with zero archived, and that 60 skills still archive ten under the
  count cap.

## 2026-11-16: The Overview skills row reports the index cost

- The daemon status carries `skills.active` and `skills.index_tokens`, computed
  from the same `SkillLibrary::index` call the agent makes. The active count
  comes from `SkillStats` after that call.
- The Work & memory card shows `N active · M tokens` and no longer shows
  `last_skill_review` or `last_skill_review_changes`. The daemon keeps
  publishing those fields; the card is the only thing that stopped reading them.
- The card's standing count now reads `rules_bullets`, the key the daemon already
  published. The old `standing_bullets` read always fell back to zero.

## 2026-11-16: The Prompt tab reads as one conversation

- A turn is one conversation, not one saved request. The view reverses the
  newest-first exchange list, compares each request's transcript with the
  conversation after the previous request, and prints only the new suffix under a
  snapshot marker. The marker names the request number, time, prompt hash, and
  token total. The selected snapshot is emphasised, and `[`/`]` put its marker at
  the top of the viewport.
- The stable layers print once and again only when a prefix part changes.
  Relevant memory is per request under its marker. The runtime note prints under
  the message that carried it. A response follows its request's delta. A shorter
  common prefix prints a `history rewritten` note instead of hiding the change.
- A different `session_id` prints a conversation boundary, resets the merge, and
  reprints the system layers, so two conversations loaded in the same list do not
  read as one rewritten history.
- `s` collapses the system layers to their names and token costs. A spinner line
  above the key hints shows `working` while `PromptView::loading` is true or the
  status reports a running task. It is drawn outside the scrolled body, so it
  stays visible while paused.
- The render cache is keyed by the loaded exchange hashes and response entry ids,
  width, raw mode, system visibility, and zone, not by the cursor. Moving between
  snapshots does not rebuild or rewrap the document. The raw view prints every
  loaded exchange under its marker.
- The header is drawn outside the cache and names the model, session, selected
  snapshot, token total, prefix hash, and prefix peers. The card's key hint owns
  the view keys; the footer keeps scrolling and help.

## 2026-11-16: The Prompt tab reads as a conversation, with live text

- The snapshot controls are gone. The tab keeps no cursor over saved requests,
  no numbered or timestamped separator, no highlighted selection, no raw JSON
  mode, and no manual refresh key. A bare rule marks where one saved request ends
  and the next begins, and the header names only the newest request's model,
  session, token total, and prefix hash. This supersedes the marker, raw, and
  header bullets of the earlier "The Prompt tab reads as one conversation"
  decision.
- A message is enclosed by a role bar: `▌` opens its label row and `▏` runs down
  its body, in the role's style, with a blank line between messages. The bar
  continues across the parts of one message, so thinking, text, and a tool call
  read as one message. System layers, memory, and the runtime note are not
  messages and keep the flush-left label and two-space body. The bar costs the
  two columns the body indent already spent, so wrapping width is unchanged.
- A message block whose wrapped body exceeds 12 lines starts folded to its first
  line plus a stub naming the hidden count. `n` and `p` move a block cursor over
  the long blocks and Enter opens or closes the selected one. The first `n`
  while following starts at the first long block in the visible tail, not at the
  document top. Tab is not used because it already switches sections. System
  layers keep the `s` group fold and are never selected one block at a time.
- Block keys are the request's prompt hash plus the message index and the block
  index, so an older block keeps its key when a refresh appends a newer request.
  The set of open keys is part of the render cache key; the selected key is not,
  because the draw swaps in a prebuilt emphasized label row. Moving the cursor
  therefore rewraps nothing.
- Assistant deltas are published by the runtime, coalesced at 100 ms or 512
  bytes with a flush at stream end, so the model's hot path does not pay a socket
  write per token. The CLI appends them to a live turn and filters them out of
  the Activity feed; the feed keeps whole events.
- The live turn is painted after the cached document, never inside it, so
  streaming does not invalidate the cache. It shows thinking first and then
  text under the same role bars. The spinner replaces the `▌` on the part being
  written and its phase starts when the turn starts, not when the process did.
  `prompt.loading` and the status task count no longer animate anything: a reload
  is not a turn and a scheduled task is not this conversation.
- The saved document reloads on a `task_step` request event, which is emitted
  after the prompt blob is written, and on `run_settled`. It no longer reloads on
  `assistant_reply`, which fires before the response entry exists. A refresh
  already in flight still coalesces into one follow-up read.
- The footer is the only key legend. The card's hint row is gone and the
  transcript gets the row back. PageUp/PageDown stay bound but are not
  advertised, so the Prompt footer fits 80 columns.

Alternatives considered. Per-message bordered `Block` widgets would draw nicer
frames but cannot be stored as cached lines, so scrolling and the cache would
need a second layout model; the character gutter keeps one document. Plain
indentation with no bar lost the message boundary the issue reported. A delta per
token was rejected by the issue's own hot-path constraint. Keeping the snapshot
selection and adding a second cursor would give two ways to move through the
same document; the subtraction leaves one.

Tradeoffs accepted. The tab no longer shows a saved request's time or hash, or its
raw JSON; the prompt blobs remain in the database for audit. A folded block hides
its body until opened. Selecting a block pauses the live tail. A tool turn can
briefly drop the previous turn's live text while the next request is being
prepared, because the live turn resets at `turn_started` and the saved blob lands
just after.

## 2026-11-16: The Prompt tab is a chat window

- Every saved message is one foldable block. `Folds { mode, overrides }` replaces
  the part-keyed `expanded` set and the `show_system` flag. `e` sets `AllOpen`,
  `c` sets `AllClosed`, and Enter flips one block against its kind default.
  `n`/`p` visit every block, including short messages and context, which the old
  long-part cursor skipped.
- Context layers and memory are blocks, not a preamble. Each layer prints only
  when its prefix part changes. Memory prints only when it changes. `s` dies.
- Visual families: user and tool blocks are full-width background panels,
  assistant text is unbacked, context and memory are tinted receipts, and
  notices are thin rows. A header is `▌ ROLE` plus the loaded turn and the
  request time in the configured zone. This supersedes the lowercase role bars
  and the bare request rule of the two earlier Prompt-tab decisions.
- A folded block shows its header, its runtime note, and a hidden-count stub.
  Tool results and system transcripts keep one preview line; user, assistant,
  context, and memory blocks hide their bodies completely. A fold changes only
  the body, so `c` then `e` always recovers it. Context layers start folded;
  memory opens to six wrapped lines, a user message to twelve, a tool result and
  a system transcript to eight, and assistant text is open.
- Response thinking is excluded from the dedupe baseline through `wire_form`,
  because `message_for_entry` does not replay it. This fixes a false
  `history rewritten` note and a repeated answer on the request after a thinking
  response. The view also stops replaying saved thinking; thinking is live-only,
  matching spec 2.5.
- The cache is keyed by the exchange hashes and response entry ids, width, zone,
  and the whole `Folds`. The selected header is prebuilt and swapped on draw, so
  moving the cursor does not rebuild. The live block stays outside the cache.
  Selecting a block reveals its header and pauses follow only when the header is
  outside the viewport, so folding a visible block does not stop a live turn.
- The status row names the model, the loaded turns, the message count, the token
  total, the session, and the prefix hash of the newest saved request. Message
  headers carry the loaded turn and the request time; there is no separate
  request boundary line.
- Alternatives considered. Two-level folding kept independent thinking control
  but needed a second cursor and two fold keys; one block, one fold won. A group
  receipt for all context saved a few lines but could not name a changed layer or
  fold one layer; one block per layer won. A boundary line with time and tokens
  was proposed; message-header metadata gives the same audit anchor without a
  second row between every message. Replaying saved thinking was rejected
  because it made a response block differ from the message the model actually
  received.
- Tradeoffs accepted. Collapsing a block hides its body; `c` then `e` is the
  recovery. Saved thinking is not in the audit view. Context bodies start
  folded, so the first screen is the conversation, not the prompt. Markdown is
  rendered as plain text.

## 2026-11-16: The Prompt tab folds as a whole

- Two view flags replace the fold map and the cursor. `PromptView { collapsed,
  thinking_collapsed }` is the only fold state, and `Fold::{Chat, Answer,
  Thinking}` on each body part names the one flag that part obeys through
  `Fold::open`. `n`/`p`, `e`/`c`, per-block Enter, the override map, the block
  keys, the selection band, and the reveal-on-select code are deleted. Enter
  folds or opens the chat, so recovering from a fold takes one key instead of
  `c` then `e`.
- A folded section keeps five wrapped lines and a stub naming the hidden count.
  One function, `section_rows`, applies that rule to the saved chat and to the
  live turn, so the two views always agree. A section with five lines or fewer
  shows in full and gets no stub. A block with several parts previews each part,
  so a three-call message still shows the first five lines of every call.
- The agent's answer never folds with the chat, and saved thinking is shown
  before the answer. `t` folds thinking alone, saved or streaming. This
  supersedes the previous entry's tradeoff that saved thinking is not in the
  audit view. `spoken` still drops thinking when it decides a block's label, and
  `wire_form` still drops thinking from the merge baseline, so a response with
  thinking neither hides its calls nor makes the next request look rewritten.
- The view starts unfolded. Follow pins the newest rows, so the first screen is
  the conversation. A folded start would put five previews of every context
  layer above it, which is the problem the per-kind defaults solved with more
  state than it was worth.
- Alternatives considered. Keeping a cursor and per-block Enter kept fine
  control at the cost of two fold keys and a selection band; one fold key won.
  Hiding thinking outright when `t` is pressed was rejected because a five-line
  preview keeps the shape of the thought readable and reuses the same collapse
  rule. Splitting thinking and each tool call into their own top-level block
  would delete the part list, but it would repeat the `HEARTH` header and the
  turn time on every section, lose the `TOOL CALL · n calls` aggregate, and make
  the live turn change shape when it is saved; one block per message won.
- Tradeoffs accepted. A fold no longer opens one block, so reading one long
  layer means opening the whole chat. A fold does not move the scroll position,
  so a paused reader can land on different lines when the document length
  changes; Home and End recover. Markdown is still rendered as plain text.

## 2026-11-16: The clock note names its payload

- The one uncached prompt line is
  `<current_time>2026-11-16T11:55:16+08:00</current_time>` and nothing else.
  `runtime` named the layer, not the fact, so the tag now carries the meaning and
  the `Current time:` label and the sentence period are gone. The ISO string
  still uses `identity.timezone`. The L7 name, `tokens.runtime`,
  `PromptInput.runtime_note`, and the saved-document key stay as they are: they
  name the layer, and old blobs already use the key.
- `now` was rejected as less explicit than `current_time`, and keeping the
  `Current time:` label inside the tag was rejected as redundant. A period was
  rejected because the payload is data, not a sentence, and a copied time string
  should not carry one.

## 2026-11-16: The working row is anchored and one key folds the chat

- The Prompt tab reserves a second row below the status line, so the model and
  the turn counts do not touch the first block. The chat body starts one row
  lower. The margin belongs to the render path, not the cache, so it costs no
  rebuild.
- The live turn is a `WORKING` block, not a `HEARTH` block, because the label
  names what the turn is doing. `HEARTH` stays the saved assistant label.
- The working row is the last row of the live block, after a blank row below the
  text, so the spinner holds the tail of the card while the answer grows above
  it. Once the document reaches the card, follow pins the row; a continuation
  turn clears the live text and the row does not move. A row reserved at the
  bottom of the card was rejected because it costs a permanent blank row and a
  second viewport for the live text. A sticky row at the top of the card was
  rejected because it sits above the saved chat rather than beside the live text.
- The spinner is the eight-frame clockwise flux cycle, `⣾ ⣷ ⣯ ⣟ ⡿ ⢿ ⣽ ⣻`: a
  full braille cell with one dot missing, one frame per 100 ms. The classic
  ten-frame cycle was rejected because its single dot moves unevenly and reads as
  a flicker at this size.
- `thinking_collapsed` and the `t` key are gone. One `collapsed` flag drives the
  chat and thinking, so Enter folds or opens both and the footer stays `↑↓
  scroll · ↵ fold · f follow · q quit`. This supersedes the `t` contract in the
  earlier 2026-11-16 "The Prompt tab folds as a whole" entry. A separate thinking
  key was rejected because it bought only a shorter thinking block while the chat
  stayed open, at the cost of a second flag and a second key.

## 2026-11-16: Guidance files are read whole and never cut

- `MAX_GUIDANCE_FILE_BYTES`, `MAX_GUIDANCE_TOTAL_BYTES`, the truncation markers,
  and `Guidance.truncated` are gone. `HEARTH.md` and every `AGENTS.md` are read
  with `fs::read` and placed in the prompt exactly as written, so the instruction
  a person adds last cannot vanish silently. Bytes are recorded per file in
  `Guidance.files` as `GuidanceFile { path, bytes }`; the text and the file list
  travel together because the prompt and the budget failure both need them.
- The agent budget failure keeps the existing message when the strain is
  elsewhere, and names the largest guidance file and its size when the guidance
  alone is at least the usable window. Naming the file that is too large was
  chosen over a generic "too large" because the person is the one who can trim
  it. Naming every guidance file was rejected because the notice is capped at
  200 characters and the largest file is the one that buys the most room.
- `SOUL.md` was already unbounded and stays that way; the two file kinds now
  behave alike. The prefix cache cost of editing a large guidance file is real
  and stays the person's to accept. This supersedes the 2026-11-09 cap entry.

## 2026-11-16: Every segment budget is reported to the person

- `SEGMENT_BUDGETS` is the one table naming each budgeted segment and its
  budget. `TokenCounts::segments` reads that table, `over_budget` filters it, and
  the CLI and Doctor both consume it, so no surface can drift from the spec
  numbers. `TokenCounts::from_document` reads the counts a saved prompt recorded,
  which is what both read-only surfaces have.
- The Prompt tab prints a warning line above the layer it names, because the layer
  block already shows the count and the line adds the comparison in the same
  place. The Doctor prints one line per offending segment, plus a fixed-overhead
  line against the section 14 target; a clean prompt prints
  `prompt: ok (N fixed tok)`.
- The model is not told. It cannot edit `SOUL.md` or its own tool schemas, so a
  note would spend prefix tokens on a fact only the person can act on. The log
  line stays for diagnosis. Reporting the same numbers in the system prompt was
  rejected for that cost.
- The warning thresholds and this surface were removed on 2026-11-16 because a
  threshold that never refuses is not a budget. The counts remain. See that
  entry.

## 2026-11-16: A notice names the request it settled

- The notice entry that settles a failure or a stop now carries the prompt hash of
  the request it answers and a machine `reason` alongside its `text`. That is the
  link the Prompt tab needs, and it needs no new column: `entries.prompt_hash`
  already existed, and the content is a JSON object either way. A timestamp join
  to `submissions` was rejected as a guess, and a new column was rejected because
  the entry is the record of the settlement, not the submission.
- `inspect::prompt_exchanges` now reads `entries.kind` and reports three outcomes
  per request: an assistant `response`, a `notice`, or neither. An assistant entry
  wins over a notice for the same hash, so a stop that follows a saved tool call
  never hides that call's answer.
- The Prompt tab shows `no reply · <cause>` only for a notice. The old line covered
  a running turn, five kinds of failure, and no failure at all; the new rule keeps
  the cause in the record instead of inferring it from a missing reply.
- The budget refusal saves the prompt it never sent, so the audit has a request to
  attach its notice to. The crash recovery path reads the failed task's checkpoint
  and links its poison notice to the same request.

## 2026-11-16: An uncounted skill has no rate

- The Skills tab prints `(unused)` when `Skill.uses` is zero and a percentage
  otherwise. The rate field is eight characters wide and the name and scope are
  padded to the widest row, so a percentage and `(unused)` end at the same edge.
  A user skill can never leave zero, because `record_use` and `record_outcome`
  resolve through `find_learned`, and a learned skill starts at zero. Printing
  `0%` stated a measurement that did not exist and could not be told apart from
  a skill that failed every run.
- `Skill::success_rate` is unchanged: the index cap still sorts learned skills by
  it, and a computed zero is the right value there. The placeholder is a display
  rule at the render site, which already has `uses`.
- The em dash this replaced read as missing data, a divider, or a glyph that
  failed to render, and the tab has no legend that explains it. `(unused)` states
  the fact in place and keeps the distinction the dash was chosen for.

## 2026-11-16: The Skills tab only reads

- The Skills tab keeps selection, reading, and scrolling. `e`, `n`, and `v`, the
  in-tab new-skill prompt, the version-history view, `EditorKind::Skill`, and the
  `skills/<name>.md` write path are gone. The library is changed by editing the
  files or by Hearth's skill loop; a second write route in the TUI duplicated the
  first and could only drift from it.
- Removing `v` removed the only reader of `SkillsView.version` and `versions`, so
  those fields and `show_history`/`leave_history` went too. `SkillLibrary::history`
  and `version_body` stay: the files remain versioned on disk, and the library API
  is what the tests and the skill loop use.
- Editing `SOUL.md` and `HEARTH.md` from Settings is a different route and stays.

## 2026-11-16: The skills list filters by scope

- `SkillsView` keeps the full library in a private `all` and derives `skills` from
  `SkillFilter`. The visible list stays the index the selection refers to, which is
  the invariant the body depends on; a filter that only hid rows would let the
  body show a skill that is no longer listed. `filtered()` over the full library
  was rejected because then two accessors would describe the same row.
- `[` cycles back and `]` cycles forward through both, user, and learned. The
  selection index is clamped to the new length and the body reloads, so switching
  scopes never leaves a stale body. Keeping the same skill selected across a
  scope change was rejected: the wanted row is usually the first one in the newly
  filtered list, and name-matching adds state for no clear gain.
- The header appends `showing <mode>` and the empty state comes from the filter, so
  a filtered empty list cannot be read as an empty library.

## 2026-11-16: Thinking owns a block and a fold key

- `derive` splits each message into thinking and spoken blocks. Thinking becomes a
  `THINKING` block with its own header, gutter, turn, and request time; the spoken
  blocks carry the `HEARTH`, `TOOL CALL`, `YOU`, or `TOOL RESULT` label without it.
  `response_block` and `message_block` no longer receive thinking, so a `HEARTH`
  block cannot open with it and a `TOOL CALL` block cannot contain it. The live
  turn builds the same block and paints it through the same `Painter`, so saved and
  streaming thinking look alike.
- `t` folds every thinking block and Enter folds the chat. This reverses the
  2026-11-16 decision that removed `t`: thinking is now a block of its own, so a
  fold that moves it independently is the point, and the chat fold no longer
  touches it. A per-block cursor was rejected because the Prompt tab has no cursor
  by design, and reusing Enter was rejected because it cannot fold thinking
  without folding the reply.
- `PromptView.thinking_collapsed` is a second fold flag, and the cache key carries
  it so a thinking fold rebuilds the cached lines. (The key and flag were removed
  on 2026-11-16; see that entry. Thinking keeps its block.)

## 2026-11-16: The prompt segment warnings were removed

- `SEGMENT_IDENTITY`, `SEGMENT_TOOLS`, `SEGMENT_RULES`, `SEGMENT_SKILLS`,
  `SEGMENT_RECALL`, `SEGMENT_RUNTIME`, `FIXED_OVERHEAD_TARGET`,
  `SEGMENT_BUDGETS`, `SegmentCost`, `TokenCounts::segments`, and
  `TokenCounts::over_budget` are gone, with the agent log line and the Prompt tab
  and Doctor notices that read them.
- A threshold that never refuses is a second definition of the word budget.
  Every enforced limit stays: `ContextBudget`, `memory.rules_max_tokens`,
  `memory.recall_max_tokens`, `check_rules_budget`, `rules_fit`,
  `recall_within`, and `INDEX_MAX_ACTIVE`. The per-layer counts stay in the saved
  prompt document, and the Prompt tab still shows them beside each layer.
- The Doctor prints only `prompt: ok (N fixed tok)`. It no longer names an
  over-budget segment or a fixed-overhead target. Reporting the counts without a
  comparison was kept because the cost is still useful in a health report; a
  second warning surface was rejected because it told the reader a request failed
  when the request was sent in full.
- The Prompt tab no longer prints `▲ <segment> over budget`. Keeping the notice
  and removing only the numbers was rejected: the notice would still read as an
  error.
- This supersedes the warning surface in "Every segment budget is reported to the
  person" and the prompt copies of the budgets in "M5a context budgets", "The
  rules budget is 800 tokens", and "The skills index has no token cap".

## 2026-11-16: Every memory is capped at 150 characters

- `store::CONTENT_LIMIT` is the one new-write limit: 150 characters counted after
  trimming. It applies to a fact, a preference, and a pinned rule alike.
  `RULE_LIMIT` is gone, so a rule no longer has a second size. The error names the
  actual length and the limit and says to split the statement.
- The store is the final gate. The extractor, the tool path, the legacy import,
  and any direct service write all reach `store::validate`. A refused write saves
  nothing, and the tool returns `invalid_input`. The `memory_save` schema names
  the same 150-character limit.
- `digest::LINE_CHARS` is 150, so a newly written memory is shown whole. Only a
  row written before the limit is clipped, with an ellipsis.
- `MemoryConfig` defaults to 700 recall tokens and at most 10 items, and the
  settings defaults match. Ten 150-character rows fit in the ordinary case. The
  700 is a maximum, not a reservation: a long subject or coverage line can still
  reduce the count.
- `extract::MAX_CHARS` is 150 and `MESSAGE_CHARS` is a separate 1,600, so a long
  user message is still scanned for a shorter statement.
- `legacy::read_bullets` no longer pre-filters by length. An over-limit bullet
  reaches the store, is refused, and counts as skipped; the old files are kept so
  the import can retry.
- Existing rows above 150 are not rewritten or deleted. The strict check applies
  to new writes, and the rules render keeps `RULE_CHARS` at 400 as its read-side
  guard.
- The 800-token rules block stays a separate aggregate budget. A 150-character
  rule consumes its share of that block, and `check_rules_budget` still refuses a
  write that would overflow it.
- Superseded on 2026-11-17 by the 200-character limit and count-only recall; see
  that entry. The store, extractor, digest, schema, and settings moved together.

## 2026-11-16: The Prompt tab starts folded under one key

- `PromptView` has one `collapsed` flag, default true. `Fold::Thinking` stays only
  to choose the thinking ink; `Fold::open` treats `Chat` and `Thinking` alike.
  `thinking_collapsed`, `toggle_thinking`, and the `t` binding are gone, and the
  cache key carries the one fold state.
- The first render previews every foldable section, thinking included, with the
  five-line stub. Enter opens the whole view; the next Enter folds it again. A
  short section still shows in full and gets no stub. A refresh, a new exchange,
  or a tab switch does not reset the chosen fold.
- The agent's answer and the live reply never fold. Enter folds the transcript
  around them, not the text the person is reading. The live thinking block folds
  with the chat, so the folded start shows its preview and the full live reply.
- The footer loses the thinking hint and keeps `↑↓ scroll · ↵ fold · f follow ·
  q quit`.
- This supersedes the `t` key and second flag in "Thinking owns a block and a fold
  key", restores the one-key contract in "The working row is anchored and one key
  folds the chat", and reverses the start-unfolded choice in "The Prompt tab folds
  as a whole". The thinking block itself stays as Draft 0.40 decided.
- A separate thinking key was rejected again: it moves apart from the reading
  state the person actually changes, and it costs a second flag and a cache-key
  member. Keeping the open start and making Enter fold both was rejected because
  the first screen stays a wall of context instead of an outline.

## 2026-11-17: Spill lives in the temp folder

- The spill root is `$TMPDIR/hearth/spill`, or `/tmp/hearth/spill` when `TMPDIR` is
  unset, empty, or relative. A relative value would put a `spill/` folder in the
  daemon's working directory, which is the same fault as an empty one.
- `home::HOME_DIRECTORIES` no longer contains `spill`, so `init` and
  `restore_home` stop creating it. The root is created on first use by
  `spill::ensure_root`, which both `Agent::spill_output` and the helper report
  write call. Nothing assumes it exists at startup.
- `spill::ensure_root` creates the root and its `hearth` parent at mode `0700`.
  It refuses a symlink, a path that is not a directory, and a directory not owned
  by the current user, because `create_dir_all` would accept any of those and the
  directory's owner could replace a file the model reads as tool output. The
  effective uid comes from the metadata of `/proc/<pid>`, which the kernel owns
  under that uid, so no `unsafe` libc call is needed.
- `Workspace` treats a missing spill root as a missing file, not `missing_root`.
  `Workspace::read_roots` skips a missing spill or media root, so a wiped `/tmp`
  cannot break an absolute read that lands under the workspace. A write through
  the tool to `spill/...` is still refused.
- The model-facing text is unchanged: it still says `spill/<name>.txt`. The
  keyword names the root, not the home.
- One root is shared by every home, and spill names are call ids. Two daemons can
  collide, and `files list spill/` lets one home's model enumerate another's
  output. This is accepted for the single-home product model. Tests pass their own
  `TempDir` as the root and never write to `/tmp/hearth`.
- `/tmp` is tmpfs on the reference host and `systemd-tmpfiles` clears it, so a
  spill may vanish. `packaging/hearth.service` is a user unit, which cannot use
  most file-system namespaces, so `PrivateTmp` is not a present risk. If a future
  unit enables one, the root is empty after every start and invisible to the TUI;
  that is acceptable for scratch data.
- The home-table row and the recovery list in SPEC 3.1 and 3.5 are removed, and
  SPEC 9.4 names the temp root. This supersedes the `spill/` home row in the
  2026-11-08 "M2b-3 tools, spill, and the tool loop" entry.

## 2026-11-17: The content limit is 200 and recall is bounded by count

- `store::CONTENT_LIMIT`, `digest::LINE_CHARS`, and `extract::MAX_CHARS` are 200.
  The store is the final gate, and the tool schema, the extractor, the import, and
  every direct write name the same limit. `digest::RULE_CHARS` stays 400 as a
  read-side guard and `extract::MESSAGE_CHARS` stays 1,600.
- `MemoryConfig` no longer has `recall_max_tokens`, and
  `memory.recall_max_tokens` is removed from settings, validation, the settings
  form, the status report, and the Overview card. A settings file that still sets
  it is refused as an unknown field, and `settings_from_value`'s `failing_field`
  names `memory.recall_max_tokens` so an installed home can delete the line.
- `MemoryService::recall_within`, `digest::line_cost`, `digest::coverage_cost`,
  `digest::trim_coverage`, and the agent's recall shrink loop and `digest_floor`
  are gone. `recall` and `recall_at` take no budget.
- `rank::fill` orders by score and keeps `recall_max_items`. It no longer divides
  the score by a line cost, so a long statement keeps its slot at equal evidence.
- The bound is a count: ten items of 200 characters, plus at most two coverage
  lines of three subjects. The worst case measured 1,143 estimated tokens and ten
  ordinary full rows measured 682. Raising `recall_max_items` raises the ceiling,
  and its clamp stays 1 to 64.
- The benchmark still reports automatic recall 0.928 lexical and 0.954 hybrid,
  one-call reachability 0.990, and digest p95 139 tokens after the ordering
  change. The run is the acceptance record; no new gate is added.
- This supersedes the recall reserve and render trim in the 2026-11-15 "The memory
  block honours its own budget" entry and the 700-token part of the 2026-11-16
  "Every memory is capped at 150 characters" entry.

## 2026-11-17: The digest row is a delimited header

- `digest::line` renders `kind | subject | age: statement`. The kind is always
  present, including `fact`, so every memory row starts with a token from
  `Kind::code`; `pref` and `commit` stay the short codes because the separator now
  marks the field. An empty subject renders `-`, so the field count never changes.
  There is no bullet: the newline is the row boundary, and `clip` removes
  whitespace so a statement cannot contain one.
- `digest::is_memory_line` is the one predicate that tells a memory row from a
  coverage line or the rules notice, and the row-count tests use it.
- A whole row carries no id. A clipped row appends its id after the ellipsis,
  because `memory_recall(id)` is the only way to read the rest. `digest::tool_line`
  always appends the id, because the recall tool is where addressing belongs. The
  related-statement hint in `memory_save` uses the same tool line.
- The rules block renders `content` per line with no fields, no id, and no bullet.
  A rule is read, not addressed; `memory_recall(kind = rule)` returns the ids.
  `store::ID_SAMPLE` is gone, so `rules_fit` and `rules_cost` measure the line that
  is actually rendered.
- `clip` returns `Clipped { text, truncated }`, so the id is appended from the
  same decision that cut the text rather than from a second length check.
- The ten-row digest measures 611 estimated tokens and the worst case 1,071, down
  from 682 and 1,143 when the id was in every row; the benchmark p95 falls from
  139 to 97. The id's fixed cost was most of the difference. A bullet was rejected
  because it adds characters to every row and marks no boundary the newline does
  not already mark.

## 2026-11-17: A memory id is twelve Crockford characters

- `store::new_id` returns `m_` plus ten characters of Crockford base32 (50 bits)
  from a v4 UUID. The alphabet drops `I`, `L`, `O`, and `U`, so a copied or
  retyped id is less likely to be wrong. Existing 34-character ids are not
  rewritten, because `superseded_by`, `memory_vectors.memory_id`, and saved
  transcripts still point at them.
- `store::normalize_id` trims and lowercases before `get`, `forget`, and a
  `supersedes` target. The lookup is exact; a prefix is not an id.
- `store::reserve_id` checks the generated id inside the write transaction and
  retries a taken id up to eight times. One writer thread owns every insert, so
  the check is the same as catching the primary-key conflict and no row can share
  an id.
- The recall tool keeps one id per row at the end of the line, the digest carries
  an id only on a clipped row, and the rules block carries none. A new id costs 12
  characters instead of 34, so a 20-row `memory_recall` answer drops by about 121
  estimated tokens.
- The `memory_recall` schema names the 12-character shape and says case does not
  matter.
- This supersedes the "Stable `m_` id" row in SPEC 7.2 and the 34-character ids in
  the 2026-11-14 "Memory rebuilt as one store and a four-stage cascade" entry.

## 2026-11-17: A memory must name its actor

- `actor::pronoun` finds the first first, second, or third person pronoun in a
  statement, split on non-alphanumeric boundaries so a contraction still counts.
  `store::validate` refuses any content that carries one, with `invalid_input`,
  and the error names the pronoun and the replacement: `the user` for first
  person, the assistant's name for second, and the name from the message for
  third. The store is the final gate for the tool, the extractor, the legacy
  import, and a direct service write.
- The check is deterministic and calls no model. A rewrite at write time would
  cost tokens, add latency to a save, and could invent detail. The model already
  has the message when it calls `memory_save`, so it can retry with a name.
- `extract` rewrites first person (`my` to `the user's`, `me` to `the user`, and a
  leading `I` or `we` with the verb agreed to the singular) and drops a candidate
  whose remaining pronoun it cannot resolve. Third person is kept only when the
  statement already names its subject, so `Alice likes apples` is stored and `he
  likes apples` is not. `acceptable` runs the pronoun check on every candidate as
  a second gate.
- The `memory_save` content description states the same rule as prose: one
  standalone statement, understandable without prior context, references
  clarified only as far as ambiguity requires. Naming the actor is the concrete
  case of that rule, so the description keeps one clause for it, since the store
  refuses a pronoun on every write. `SUMMARY_INSTRUCTION` asks the same for each
  compaction statement. The four memory schemas measure 823 tokens, under the
  830-token budget. The earlier fragment wording cost 24 fewer tokens and read as
  a rulebook; the guard moved rather than shaving the other three schemas to buy
  room for the words the model uses most.
- A rule keeps its imperative shape: `Never deploy on Friday` has no pronoun and
  is accepted. A rule that does carry a pronoun is refused like any other
  statement.
- Existing rows are not rewritten, hidden, or deleted. A vague row still renders;
  the owner can replace it with `supersedes` or remove it with `memory_forget`,
  and the replacement is held to the new rule.

## 2026-11-17: The store takes a memory in any wording

- The actor rule is removed. `store::validate` no longer inspects pronouns, and
  `actor.rs` is deleted with its refusal text and its three-person table. The
  gate cost a refused write on a word the user had already said, and the model
  had no way to know which words were refused except by hitting it.
- The schema says nothing about it either way. The `memory_save` content
  description ends at "do not add unnecessary details", and
  `SUMMARY_INSTRUCTION` no longer asks for a named actor. Telling the model to
  name an actor is the encouragement half of the rule, so it goes with the gate.
- The extractor stops refusing too. It no longer drops a candidate whose pronoun
  its rewrite cannot resolve, and `acceptable` no longer tests for one. Its own
  templates still compose `the user's ...`, because that is the voice of the
  deterministic net rather than a rule the agent is held to; a statement it
  cannot resolve is stored as written.
- What the store still refuses is unchanged: empty content, over 200 characters,
  NUL, a subject over its limit, credential-shaped content, and a pin that is not
  a rule. Those are properties of the row, not of the writer's grammar.
- The four memory schemas measure 811 tokens against the 820-token guard, down
  from 823 before this change.
- No benchmark moved, measured on both sides of this change. Automatic recall is
  0.928 on the 200-statement fixture and 0.938 with 4,170 rows, recoverable in one
  call is 0.99, and digest tokens are 97 at p95, the same before and after.
  Retrieval reads `subject` and the FTS5 terms, so a wording rule could not move
  it.
- This supersedes the 2026-11-17 "A memory must name its actor" entry, which
  stands as the record of why the rule was tried.

## 2026-11-17: Remove the settings compatibility shims

- `identity.soul_file`, `models.pricing`, and `tasks.eta_reminder_minutes` are no
  longer stripped or ignored at load. An unknown key is a schema error, the same as
  a typo.
- `https` is no longer an alias for the direct fetcher in `web.fetch.order` or
  under `web.fetch.providers`.
- The TUI no longer writes a `daemon` section when one is absent. `save_daemon`
  and the service install path remain the only writers of that section.

## 2026-11-17: Remove the document record and the fork

- `documents`, `Document`, `DocumentWrite`, `Save::document`, `Session::document`,
  `Session::fork`, and `CoreError::DocumentVersion` had no production caller. They
  are removed, and migration 0011 drops the `documents` table.
- The conversation `parent_id` and `parent_entry_id` columns stay: helper
  conversations use them for provenance.

## 2026-11-17: Media is operator managed

- The SPEC no longer promises age-based media pruning or a broken-reference
  report. No code walks `media/inbound` or `media/outbound`, and deleting a file
  the record still references would break a transcript that cannot be rewritten.
  The operator removes old media when disk or backup size matters.

## 2026-11-17: Browser navigation is best effort

- The direct fetch client filters addresses at connect time with
  `hearth_model::net::public_resolver`. Camofox makes its own connection, so
  Hearth cannot apply that resolver to a browser tab.
- The URL check before `POST /tabs` stays, but SPEC 9.5 and SECURITY.md say it is
  best effort and that the local Camofox process and its resolver are the
  boundary. Pinning the resolved address would require a Camofox API that accepts
  an address and a Host override, which it does not have.

## 2026-11-17: the tool set is flat and there is no browser

- `browser` is gone. It was a REST client for a Camofox service that Hearth never
  installed, started, or checked, so every install advertised a tool that could
  not work and every call failed with `browser_unavailable`. The client also
  answered `click` and `type` with success without reading the response body, so
  a click that landed on a consent banner was reported as a click.
- `tool_search` and the lazy group registry are gone with it. Removing the two
  groups left one tool behind a 158-token indirection that also cost a per-run
  selection set, a checkpoint field, and a `tool_groups` input on `task`. `media`
  is an ordinary registry entry that appears only when a media service is
  configured, which is the condition that made its group unavailable before.
- `documents` is gone on review. It was the only path to a PDF, DOCX, PPTX, or
  XLSX already on disk, including a Telegram attachment, and `web_fetch` reads one
  only from a URL. The trade is accepted rather than overlooked: `pdf-extract`
  stays in the tree for the fetcher, so this removes the tool and not the
  dependency, 151 schema tokens leave every request, and the agent now tells the
  user it cannot read an attachment. A user who needs it installs a converter and
  drives it through `shell`.
- The core schema budget is measured, not assumed. The registry measures 1,611
  tokens, and 1,776 with image, speech to text, and text to speech all configured.
  One guard of 1,800 covers the widest run.
- `docs/SCHEMA.md` listed tool groups on the stored `agent` document. No settings
  key ever existed for them, and the field is removed.
- Browser work belongs to whatever the user installs and drives through `shell`.
  SPEC 9.2 and SECURITY now name that boundary instead of the Camofox process, and
  the tests no longer mention a browser.

## 2026-11-17: One memory identity and one statement per write

- A memory write is refused when its content holds more than one statement. The
  splitter in `hearth-memory/src/text.rs` is shared with the extractor and is
  conservative: a word in a small title list, a one-letter initial, a decimal, a
  repeated punctuation mark, an ellipsis, or a lowercase continuation is not a
  boundary. The refusal names the statement count and asks the caller to split
  the write. Existing compound rows are not rewritten. The audit reports them.
- The extractor gives a remembered first-person statement the subject `user`,
  runs the preference classifier before the possessive one, and writes both
  relation orders as `Alice is the user's sister`. These are the forms the store
  already receives from a direct preference write.
- The duplicate comparison folds US and UK spellings and the preference verbs
  `prefer`, `like`, and `love`, and it refuses to call a negation a duplicate of
  its affirmative. The fold is in `content_tokens`, so it affects comparison and
  overlap only. The stored fingerprint keeps the words the caller wrote. Changing
  it would strand every existing row.
- The near-duplicate check is not limited to two non-empty subjects. A subject is
  indexed and is read in full, while a subjectless write is compared against the
  newest 64 current rows. That catches the empty-subject variant without scanning
  the whole store on the write path.
- A write whose source is `extract` cannot close a pinned row, a row with a
  subject when the extract has none, or a non-`fact` row with a `fact`. It returns
  the earlier row. This keeps the stronger provenance the agent or the user
  supplied.
- `memory_save` now reports a related row as a separate statement and no longer
  recommends `supersedes` in the reply. The schema already teaches replacement,
  and a non-replacing behaviour must not invite one.

## 2026-11-17: Capture reads the entries the run answered

- Migration 0012 adds `submissions.user_entry_id`, the user entry saved with the
  submission. `Session::save` links a submission to the user entry in the same
  save, and the backfill links an existing row by conversation, content, and
  creation instant. The link is durable, so a restart between the answer and
  capture still resolves the run's own words.
- `RunOutcome.user_entry_ids` carries the submission's entry plus every absorbed
  submission's entry, oldest first and deduplicated. The daemon captures each
  one. A queued message that was not absorbed is captured by its own run later.
- The daemon's memory tool source uses the same submission entry instead of the
  newest user entry, so provenance matches the message the run answered.
- Capture of the same entry twice still writes nothing, because the store's
  exact and near checks see the earlier row.

## 2026-11-17: The consolidation audit

- `MemoryService::consolidate` runs one write transaction. Canonical means the
  same fingerprint under the same subject. The survivor is the pinned row first,
  then the higher `Kind::prior`, then the most recently updated, then the smaller
  id. Every other row in the group gets `valid_to`, `superseded_by`, and a cleared
  pin. Nothing is deleted and no content is rewritten.
- The pass reports near-duplicate pairs, one content under different non-empty
  subjects, and compound rows. It changes none of them. Canonical groups are
  found over the whole store with a grouped query. The near and compound reads
  take the newest 10,000 current rows, and each sample list stops at 20 examples.
- The report and the `last_consolidation` instant are written in the same
  transaction as the closes, as the meta keys `last_consolidation` and
  `last_consolidation_report`. The report is JSON. A crash rolls the pass back,
  and the crash suite proves it at `HEARTH_CRASH_AT=memory.consolidate`.
- `memory.consolidate` is a global nightly task at 03:30 local, registered by
  `ensure_maintenance_tasks` in both the Telegram daemon and the maintenance-only
  scheduler. The handler publishes a `memory.consolidated` event with the counts
  when a bus is attached, then sleeps to the next slot.
- There is no startup pass. The nightly task is the automatic path. A duplicate
  created before it runs is repaired by the next pass. The digest collapses a
  canonical duplicate and reports the hidden count in the meantime, so the prompt
  does not pay for the same statement twice.
- The status payload gains `memory.pinned`, `memory.active`, `memory.retired`, and
  `memory.audit` with the last report's counts. `memory_items` stays for the
  Overview and its existing test.
- `hearth-cli` now depends on `hearth-memory` in the normal build, because the
  Memory tab renders `Breakdown` and the audit report. The crate was already a
  test dependency. No external crate was added.
- The Memory tab reads `inspect::memory_state`, which counts through a read-only
  connection rather than the loaded page, so a store larger than the 1,000-row
  page reports its full size. The title names pinned, active, and retired. A
  search adds a separate match count. A report candidate carries `⚠` in the list
  and an `audit` label in the detail pane.

## 2026-11-17: One memory tool with four actions

- `memory_save`, `memory_recall`, `memory_forget`, and `memory_transcript` are
  now one tool, `memory`, with a required `action` of `save`, `recall`, `forget`,
  or `transcript`. The shape mirrors `files`, which is the operator's request.
- The properties are flat. `subject`, `kind`, and `limit` serve save and recall.
  `id` serves recall and forget. `query` serves recall and transcript. The
  content, kind, id, and subject descriptions are unchanged, and each action
  runs the same handler code as the tool it replaces.
- The combined schema measures 491 tokens, down from 811 for the four. The core
  set measures 1,294 tokens, and 1,459 with every media service configured, so
  the core guard moves from 1,800 to 1,470 and the memory guard from 820 to 500.
- `replay_safe` reads the action. A replayed `save` or `forget` is refused like
  `shell` and `message`, because it changes the store. A replayed `recall` or
  `transcript` runs again because it is read-only.
- The digest coverage line names `memory(recall, subject)` and
  `memory(recall, as_of)`. The status key `memory_recall` keeps its name because
  it describes the recall subsystem, not a tool.

## 2026-11-18: Telegram albums, quoted rich messages, and one delivery per answer

- The poller groups updates that share a chat and `media_group_id` into one
  admission. The batch is read again for a 500 ms quiet window, up to 2 s, before
  the group is admitted, because Telegram can deliver an album's members across
  two `getUpdates` responses. The group keeps the first member's request id and
  update id, combines the caption from every member once, and appends each
  member's photo or file block in message order. The offset advances past the
  highest member id.
- The album wait blocks the single poller thread, not a worker. It only runs when
  the batch contains an album, so a text-only poll keeps one `getUpdates` call.
- `Message.rich_message` is flattened to plain text for reply context, for the
  incoming body, and for the content check. Hearth sends `sendRichMessage`, so a
  reply to the bot carries `rich_message.blocks` instead of `text`; the previous
  reply context read only text and captions and dropped the quote. The flattening
  is generic over `text`, `caption`, `blocks`, `items`, and `cells`, which covers
  paragraphs, headings, preformatted text, lists, tables, quotations, and styled
  spans. A rich message with no text is described as a rich message.
- `done_answers_without_delivery` returns one row per answer entry. An absorbed
  submission is settled `done` with the active submission's answer, so the old
  per-submission `tg:<id>` check reported it as undelivered and the startup
  recovery sent the answer again. The query now only returns the earliest settled
  submission for an answer, and treats the answer as delivered when any
  submission sharing it has a `telegram.send` effect. That keeps the effect keyed
  by submission while making the shared answer owe at most one delivery. A crash
  before the delivery intent still derives the intent from the record.
- No schema change. The tests are
  `an_answer_shared_by_an_absorbed_submission_is_delivered_once`,
  `a_delivered_answer_is_not_sent_again_after_a_restart`,
  `a_reply_to_a_rich_message_keeps_the_quote`, and the two album tests in
  `s5_inbound`.

## 2026-11-18: A durable album queue

- The poller stages each album member in `telegram_albums` and
  `telegram_album_members`, in the same write as the update row and the offset.
  The group's `ready_at` is `now + 1 s`, reset on every member. The poller no
  longer sleeps on an album.
- `poll_once` submits every due album before it reads the next batch. The
  submission and the deletion of the staged rows are one save, so a crash either
  leaves the album staged or leaves the submission and no rows. If a submission
  already exists for the request id, the flush clears the rows.
- While an album is pending, `poll_once` asks `getUpdates` with a zero timeout
  and `run_poller` sleeps 100 ms between calls. Ingestion stays live. This is the
  part `pi-telegram` gets from its 100 ms queue worker; Hearth keeps one poller
  thread and no side queue file.
- The quiet window resets without a hard cap, like `pi-telegram`. Telegram bounds
  an album at 10 members, and the allowed user is single, so the wait is bounded
  in practice.
- A repeated caption is included once, by exact text. Telegram normally sends one
  caption per album; the dedupe covers its repeated fragments.
- Rejected alternatives. A second album worker thread avoids the poller tick but
  shares the client and media store across threads. A `queue.json` side file
  copies message content and adds a second durability boundary. A longer sleep in
  `collect_updates` keeps the old poller stall. The staged table uses the existing
  writer and keeps the offset as the single admission boundary.
- Migration 0013 adds the two tables. `SCHEMA_VERSION` moves to 13. No existing
  migration changes.
