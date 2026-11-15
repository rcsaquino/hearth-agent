# AGENTS.md

## Project overview

Hearth is a local-first, single-user personal agent written in Rust. One daemon
owns a SQLite record, runs the agent loop, and talks to one Telegram bot. The
binding product definition is [SPEC.md](docs/SPEC.md); implementation choices and
deviations are in [DECISIONS.md](docs/DECISIONS.md); the storage columns are in
[SCHEMA.md](docs/SCHEMA.md), with the migrations as the executable authority.

This file is for coding agents working in this repository. Hearth also reads
`AGENTS.md` files from the workspace and its ancestors as project guidance, so
keep this file accurate, free of secrets, and free of live personal data.

## Ground rules

- Never print, log, commit, or echo a token, key, password, private path, real
  name, Telegram id, or personal database. Use the placeholders `USER_NAME`,
  `123456789`, `~/.hearth`, and `example.com` in examples and docs.
- Publishing is a separate approval. `origin` is public, so a push that adds
  commits needs explicit instruction, and force-pushing or rewriting published
  history needs its own. Run `scripts/secret-scan.sh` before a push that grows
  the history.
- Do not make live Telegram, provider, ElevenLabs, OpenRouter, Groq, search, or
  Camofox calls in tests. Use local fake servers. A live smoke test needs explicit
  approval before it runs.
- Do not change the configured model or provider, or add a paid service, without
  approval.
- Subagents using the parent's model need no approval. Any other subagent does.
- Do not restart the development daemon or touch `.devhome/` live data unless the
  task is specifically about that home. `.devhome/` is gitignored.
- Do not commit `target/`, `dist/`, databases, media, logs, backups, or generated
  artifacts.
- Do not modify an existing migration after it has been applied. The migrations
  are the schema authority; add a new migration instead.
- Keep the application synchronous. There is no async runtime, no tokio, and no
  background executor beyond the explicit threads in the storage and runtime
  crates.
- `unsafe` code is forbidden by the workspace lint.
- Keep the shell and file tools confined as designed. Do not widen a root or add
  an absolute-path escape to make a test pass.

## Build and test

Rust 1.89 or newer, edition 2024. The workspace has ten crates.

```bash
cargo build
cargo test --workspace
./scripts/ci.sh
```

`./scripts/ci.sh` is the commit gate: it runs `check-docs.sh`, `cargo fmt
--check`, clippy with `-D warnings`, and the workspace tests. Run it before every
commit.

Additional checks:

```bash
./scripts/test-scripts.sh          # docs, scan behavior, package tarball, checksum
./scripts/acceptance.sh            # crash, scheduler, fake Telegram, benchmarks, soak
./scripts/soak.sh                  # bounded restart/backup/restore cycles
./scripts/check-docs.sh            # required docs and placeholders
./scripts/secret-scan.sh           # tree and history credential scan
```

`scripts/secret-scan.sh` uses a private deny-list outside the tree, named by
`HEARTH_PUBLIC_DENYLIST`. When it is absent the public scan exits 2 by design;
the acceptance script supplies an empty temporary list and still runs the
credential-pattern scan. Never add the deny-list to the repository.

Benchmarks, from the workspace:

```bash
cargo test -p hearth-eval --test memory_bench -- --nocapture
cargo test -p hearth-eval --test memory_reachability
HEARTH_EMBEDDING_MODEL_DIR=~/.hearth/models/bekko-embedding-v1-a8m \
  cargo test -p hearth-eval --test memory_bench the_hybrid -- --ignored --nocapture
```

The memory benchmark must keep one-call recovery at or above 0.98, automatic
recall at or above 0.92, digest tokens at or below 300 at p95, retrieval at or
below 100 ms at p95, and zero secret leaks. `HEARTH_BENCH_VERBOSE=1` prints every
question the digest missed.

Useful focused runs:

```bash
cargo test -p hearth-core --test s1_prompt
cargo test -p hearth-telegram --test s5_telegram
cargo test -p hearth-runtime --test s6_secrets
cargo test -p hearth-runtime --test service
cargo test -p hearth-cli --test s8_telegram_daemon
cargo test -p hearth-cli --lib
cargo test -p hearth-cli --test daemon_lifecycle
```

## Build and release

```bash
cargo build --release
./scripts/package.sh x86_64-unknown-linux-gnu
./scripts/test-scripts.sh
```

`scripts/package.sh` writes `dist/hearth-<version>-<target>.tar.gz` with the
binary, docs, unit file, and SHA-256. The default target is
`x86_64-unknown-linux-gnu`; build aarch64 only when the target toolchain is
installed. `packaging/hearth.service` is the user unit; the TUI Settings tab
writes and enables it. Do not enable or start the unit against the host in tests,
and do not push or publish the tarball without explicit approval.

## Test-first workflow

Every behavior begins as a failing test at a public seam. Use a test-only
`test:` commit that fails for the expected reason, then a separate `feat:`,
`fix:`, or `refactor:` commit. Do not combine the test and the implementation in
one commit, and do not add tests after the code as a catch-up. If a test passes
on its first run, say so in the report instead of claiming a red/green cycle.

The suites are:

- S1 core session, saves, prompts, compaction, and skills.
- S2 storage contract and migrations.
- S3 provider adapter and streaming.
- S4 memory, extraction, and consolidation.
- S5 Telegram, media, and the message tool.
- S6 agent loop, budgets, secrets, steering, and lazy tools.
- S7 crash recovery, scheduler, helpers, maintenance, and skill review.
- S8 operator surface, daemon end-to-end, packaging, and acceptance.

Integration tests use real SQLite. Mock only system boundaries: HTTP, the clock,
randomness, and the filesystem where a temp home is not practical. Keep
expectations independent of the implementation.

The fake-server pattern in this repo is std-only: a `TcpListener` on
`127.0.0.1:0`, bounded reads and waits, a `Drop` guard, and a stop flag with
nonblocking accept so a failed test cannot hang the suite.

## Architecture

| Crate | Responsibility |
|---|---|
| `hearth-config` | Strict settings parsing, `${ENV}` expansion, validation, and 0600 checks. |
| `hearth-core` | Conversations, entries, saves, tasks, submissions, prompt assembly, compaction, guidance, memory records. |
| `hearth-storage` | SQLite connections, one writer and a read pool, migrations, snapshots, FTS5. |
| `hearth-model` | OpenAI-compatible streaming, tool calls, usage, pricing, media clients. |
| `hearth-memory` | Statements, rules, the retrieval cascade, the digest, and four tools. |
| `hearth-tools` | Files, shell, web search, web fetch, documents, media, browser, workspace confinement, spill. |
| `hearth-telegram` | Bot API polling, replies, attachments, and the message tool. |
| `hearth-runtime` | Daemon, agent loop, scheduler, helpers, skills, watch socket. |
| `hearth-cli` | The `hearth` binary and the ratatui terminal application. |
| `hearth-eval` | Offline memory and token benchmarks and crash tooling. |

Key invariants:

- The only user-facing entry is `hearth`, which opens the TUI. `HEARTH_DAEMON=1`
  is the internal daemon child mode used by the TUI and the systemd unit. The TUI
  never opens a database writer; onboarding runs `init` once before any daemon.
- One writer thread owns SQLite writes; readers use a small pool. Every durable
  change goes through one save path with a monotonic sequence.
- Task transitions are saves. Heartbeats are not. Leases, retries, poison, and
  recovery must stay restart-safe.
- Telegram sends use at-most-once delivery effects. An uncertain send becomes
  Unknown and is recovered, never silently duplicated.
- Prompt layers have fixed budgets and a prefix hash. Changes to the identity,
  tools, guidance, rules, or skills must be visible in the Prompt tab
  and must name the changed layer.
- Guidance comes from `HEARTH.md` in the home and `AGENTS.md` from the filesystem
  root down to the workspace. Each file is capped at 16 KB and the total at 32 KB.
- Memory is one table in `hearth.db`. Never overwrite a statement: close its
  window and point `superseded_by` at the replacement. Recall is three cheap
  channels (FTS5, subject, INT8 vectors behind a 0.40 floor) fused by RRF k=60,
  ranked by a linear score over cheap features, then filled into a token budget.
  No memory operation calls a model. The digest reports what it left out.
- Optional tool groups are lazy. Their schemas enter a request only after
  `tool_search` selects them; the core prompt budget must not grow.
- Every timestamp shown to the model or the person is ISO 8601 in
  `identity.timezone`, resolved when it renders so a settings change applies
  without a restart. Every stored instant stays UTC. See `Zone` in `hearth-core`.
- A `login` or `startup` install owns the unit and its `default.target.wants`
  link. `service::repair` recreates either one when it is missing, and repair
  only ever adds: it never rewrites an existing unit and never deletes.
- A message is acknowledged by the typing indicator, not a reaction.
- ElevenLabs speech text is passed verbatim. The model is taught to use sparse
  Eleven v4 tags such as `[warm, composed voice]` and `[softly]`; no SSML.

## Style

- Apply YAGNI, KISS, and DRY. Build what the current slice needs, choose the
  smallest direct design, and keep one authoritative definition for each rule.
- New dependencies need a one-line justification in `docs/DECISIONS.md`. Prefer the
  standard library and an existing crate.
- Use `cargo fmt` and address clippy rather than suppressing it.
- Keep modules small and public interfaces narrow. The main seams are `Harness`,
  `Session`, `Storage`, `Provider`, `Tool`, and the transports.
- Errors are single-sentence and user-facing. They must not include secrets,
  request bodies, or file contents.
- Prefer clear names over comments. Comment why, not what.
- Update `docs/DECISIONS.md` for a deviation or a nontrivial choice. Bump the spec
  Draft when behavior changes. Do not leave a decision only in a commit message.

## Commits

Use short, imperative commits with a conventional prefix:

- `test: require a stale lease to be requeued`
- `feat: add the watch socket`
- `fix: bound the retry delay`
- `refactor: carry guidance through prompts`
- `docs: record the storage decision`
- `style: format the runtime`

One behavior per commit. Keep the test commit separate. Do not rewrite published
history. The author identity is configured globally; use it rather than adding a
second identity.

## Known residuals

- Memory recall is 0.933 automatic and 0.99 recoverable in one call on the lexical
  path, and 0.954 and 0.99 with the local encoder installed. Paraphrase is the
  weak category at 0.85 hybrid and 0.65 lexical.
- Two of the 195 benchmark questions need more than one tool call. Both are
  paraphrases with no shared vocabulary. Six of the thirteen automatic misses are
  topics the fixture gives two live answers to.
- Retrieval p95 is 6.4 ms lexical and 11.3 ms hybrid on a 200-statement store, and
  24 ms with 4k distractors. At 200,000 rows with 200,000 distinct subjects it is
  9.1 ms at p50 and 221.3 ms at p95 on the reference host, measured by the ignored
  `the_200k_store_is_measured`. The p95 is driven by fixture questions that share
  a word with the noise; the hybrid vector scan at that size is still unmeasured.
- `deepseek-v4.1-flash` is unpriced in this home; cost is recorded as unpriced
  rather than invented.
- The seven-day soak remains open; the first public release is 0.1.0 alpha. The bounded
  soak and the local acceptance matrix are green.
- The aarch64 package is built only when the target toolchain is present.
- `AGENTS.md` is also read as guidance, so the memory notes here and `SPEC.md` 7
  must stay in step.
