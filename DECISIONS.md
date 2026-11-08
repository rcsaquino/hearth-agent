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
