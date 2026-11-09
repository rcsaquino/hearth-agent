# AGENTS.md

## Project overview

Hearth is a local-first, single-user personal agent written in Rust. One daemon
owns a SQLite record, runs the agent loop, and talks to one Telegram bot. The
binding product definition is [SPEC.md](SPEC.md); implementation choices and
deviations are in [DECISIONS.md](DECISIONS.md); the storage columns are in
[SCHEMA.md](SCHEMA.md), with the migrations as the executable authority.

This file is for coding agents working in this repository. Hearth also reads
`AGENTS.md` files from the workspace and its ancestors as project guidance, so
keep this file accurate, free of secrets, and free of live personal data.

## Ground rules

- Never print, log, commit, or echo a token, key, password, private path, real
  name, Telegram id, or personal database. Use the placeholders `USER_NAME`,
  `123456789`, `~/.hearth`, and `example.com` in examples and docs.
- This repository is local. Do not add a remote, push, open a pull request, use
  authenticated GitHub tooling, or publish anything unless the maintainer
  explicitly authorizes it.
- Do not make live Telegram, provider, ElevenLabs, OpenRouter, Groq, search, or
  Camofox calls in tests. Use local fake servers. A live smoke test needs explicit
  approval before it runs.
- Do not change the configured model or provider, add a paid service, or create a
  subagent without approval.
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

Benchmarks, after a build:

```bash
./target/debug/hearth eval memory --json
./target/debug/hearth eval tokens --json
```

The memory benchmark must keep recall@10 at or above 0.90 and zero secret leaks.
The token benchmark must keep the fixed overhead, cache share, save, assembly,
and recall timings within the section 14 targets when no guidance file is
present.

Useful focused runs:

```bash
cargo test -p hearth-core --test s1_prompt
cargo test -p hearth-telegram --test s5_telegram
cargo test -p hearth-runtime --test s6_secrets
cargo test -p hearth-cli --test s8_telegram_daemon
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
installed. `packaging/hearth.service` is the user unit; `hearth service install`
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
| `hearth-memory` | Memory store, dedupe, standing memory, recall, extraction, consolidation. |
| `hearth-tools` | Files, shell, web search, web fetch, documents, media, browser, workspace confinement, spill. |
| `hearth-telegram` | Bot API polling, replies, attachments, and the message tool. |
| `hearth-runtime` | Daemon, agent loop, scheduler, helpers, skills, watch socket, CLI wiring. |
| `hearth-cli` | The `hearth` binary and subcommands. |
| `hearth-eval` | Offline memory and token benchmarks and crash tooling. |

Key invariants:

- One writer thread owns SQLite writes; readers use a small pool. Every durable
  change goes through one save path with a monotonic sequence.
- Task transitions are saves. Heartbeats are not. Leases, retries, poison, and
  recovery must stay restart-safe.
- Telegram sends use at-most-once delivery effects. An uncertain send becomes
  Unknown and is recovered, never silently duplicated.
- Prompt layers have fixed budgets and a prefix hash. Changes to the identity,
  operation, tools, guidance, standing memory, or skills must be visible in the
  prompt audit and must name the changed layer.
- Guidance comes from `HEARTH.md` in the home and `AGENTS.md` from the filesystem
  root down to the workspace. Each file is capped at 16 KB and the total at 32 KB.
- Memory is lexical: FTS5 with porter stemming, RRF k=60, 0.85 Jaccard dedupe, and
  a 180-day half-life. Do not add embeddings without a spec decision.
- Optional tool groups are lazy. Their schemas enter a request only after
  `tool_search` selects them; the core prompt budget must not grow.
- A message is acknowledged by the typing indicator, not a reaction.
- ElevenLabs speech text is passed verbatim. The model is taught to use sparse
  Eleven v4 tags such as `[warm, composed voice]` and `[softly]`; no SSML.

## Style

- Apply YAGNI, KISS, and DRY. Build what the current slice needs, choose the
  smallest direct design, and keep one authoritative definition for each rule.
- New dependencies need a one-line justification in `DECISIONS.md`. Prefer the
  standard library and an existing crate.
- Use `cargo fmt` and address clippy rather than suppressing it.
- Keep modules small and public interfaces narrow. The main seams are `Harness`,
  `Session`, `Storage`, `Provider`, `Tool`, and the transports.
- Errors are single-sentence and user-facing. They must not include secrets,
  request bodies, or file contents.
- Prefer clear names over comments. Comment why, not what.
- Update `DECISIONS.md` for a deviation or a nontrivial choice. Bump the spec
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
history in this local repository. The author identity is already configured; use
it rather than adding a second identity.

## Known residuals

- Paraphrase recall is 0.650 on the synonym-only benchmark cases; the design is
  lexical.
- A genuine standing-memory change can cold-build above 80 ms; the normal 200k
  path is about 5 ms.
- `deepseek-v4.1-flash` is unpriced in this home; cost is recorded as unpriced
  rather than invented.
- The seven-day soak and the public-release decision remain open. The bounded
  soak and the local acceptance matrix are green.
- The aarch64 package is built only when the target toolchain is present.
- The old dev-home memory row saved before the provenance fix still has the old
  `assistant/null` source. New rows are correct.
