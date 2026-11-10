# Hearth

Hearth is a single-user personal agent that lives in Telegram and keeps its own
durable record in a local SQLite database. It is written in Rust. One daemon owns
the database, runs the agent loop, and talks to one Telegram bot. There is no
Hearth account, no telemetry, and no hosted storage.

The full behavior and design are defined in [SPEC.md](SPEC.md); the storage
columns are defined in [SCHEMA.md](SCHEMA.md), with the migrations as the
executable authority. This tree is the pre-1.0 implementation.

## Project status

Hearth is pre-1.0. The current release targets Linux x86_64; aarch64 builds
from source. The public API and settings may change. The bounded soak and local
acceptance matrix pass; a seven-day real-time soak is still pending.

## What it does

- Keeps an append-only conversation record that survives crashes and restarts.
- Remembers facts, preferences, people, projects, and past work in a local
  library with full-text search, provenance, and maintenance.
- Reads your standing rules from `HEARTH.md` and project rules from `AGENTS.md`.
- Runs an agent loop with tools: files, shell, web search, web fetch, documents,
  media, browser, memory, skills, reminders, helpers, and Telegram files or
  voice.
- Speaks Telegram natively: text, images, documents, voice notes, albums, and
  rich replies.
- Provides a local line-based chat, live run watch, status, prompt audit, usage
  stats, and offline evaluations.
- Backs up to `~/.hearth/backups/` and installs as a systemd user service.

## Requirements

- Linux on x86_64 or aarch64.
- Rust 1.89 or newer to build. Release binaries need no toolchain.
- A Telegram bot token and your numeric Telegram user id.
- An OpenAI-compatible provider: base URL, API key, and model name.
- Optional: Groq for transcription, ElevenLabs for speech, OpenRouter for
  images, Camofox for browsing, and Brave, DeepSeek, Firecrawl, or TinyFish for
  search fallbacks. Exa search works without a key.

## Quick start

```bash
cargo build --release
./target/release/hearth init
./target/release/hearth doctor
./target/release/hearth service install
systemctl --user start hearth
```

For a foreground run instead of the service, use
`./target/release/hearth daemon`.

Then open your bot in Telegram and send a message. Hearth answers only the
allowed user ids.

For a terminal-only conversation:

```bash
./target/release/hearth chat
```

The home folder defaults to `~/.hearth`. `HEARTH_HOME` or `--home` moves it.
[QUICKSTART.md](QUICKSTART.md) covers BotFather, your numeric id, provider
settings, the service, backups, and troubleshooting.

## Architecture

Hearth is a Rust workspace with narrow crate boundaries:

| Crate | Responsibility |
|---|---|
| `hearth-config` | Strict settings parsing, validation, and environment references. |
| `hearth-core` | Conversations, entries, saves, tasks, submissions, prompt assembly, guidance, compaction, memory records. |
| `hearth-storage` | SQLite connections, migrations, snapshots, full-text search. |
| `hearth-model` | OpenAI-compatible streaming, tool calls, usage, pricing, media clients. |
| `hearth-memory` | Memory store, dedupe, standing memory, recall, extraction, consolidation, episodes. |
| `hearth-tools` | Files, shell, web search, web fetch, documents, media, browser, workspace confinement, spill. |
| `hearth-telegram` | Bot API polling, replies, attachments, the message tool. |
| `hearth-runtime` | Daemon, agent loop, scheduler, helpers, skills, maintenance, watch socket, CLI wiring. |
| `hearth-cli` | The `hearth` binary and its subcommands. |
| `hearth-eval` | Offline memory and token benchmarks and crash tooling. |

There is one writer thread for SQLite and a small read pool. Every durable change
goes through one save path with a monotonic sequence. Tools and model providers
are compiled capabilities, not a runtime plugin system.

## Security

> [!WARNING]
> Hearth runs tools without approval prompts and does not sandbox the shell. Read
> [SECURITY.md](SECURITY.md) before giving it data you care about.

Telegram is a third-party transport: message content and attachments pass through
Telegram.

## Documentation

- [QUICKSTART.md](QUICKSTART.md) for setup and first use.
- [SECURITY.md](SECURITY.md) for the trust model and secret handling.
- [CONTRIBUTING.md](CONTRIBUTING.md) for the development workflow.
- [AGENTS.md](AGENTS.md) for coding agents working in this repository.
- [SCHEMA.md](SCHEMA.md) for the storage columns and migration map.
- [SPEC.md](SPEC.md) for the binding product definition.
- [DECISIONS.md](DECISIONS.md) for implementation decisions and deviations.

## Support

If Hearth is useful to you, you can support its development on
[Ko-fi](https://ko-fi.com/rcsaquino).

<a href='https://ko-fi.com/rcsaquino' target='_blank'><img height='72' style='border:0px;height:72px;' src='https://storage.ko-fi.com/cdn/kofi2.png?v=3' border='0' alt='Buy Me a Coffee at ko-fi.com' /></a>
