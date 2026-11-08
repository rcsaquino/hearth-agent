# Hearth

Hearth is a single-user personal agent that lives in Telegram and keeps its own durable record in a local SQLite database. It is written in Rust and runs on one machine.

## Principles

1. The durable record is the truth. Messages, submissions, tool calls, effects, and memory rows are stored before they are acted on.
2. The model is a reasoning engine, not the memory. Facts live in SQLite; the prompt is assembled from the record.
3. Everything is a crate with a contract: harness, session, provider, tool, transport, memory.
4. Efficiency is architecture: budgets, compaction, prompt caching, and lazy tools.
5. The single user is the trust boundary: one allowlisted Telegram id, no multi-user tenancy.
6. The agent survives interruption: tasks, leases, crash points, and recovery.
7. It runs on one machine, forever: no hosted services and no telemetry.

## Crates

| Crate | Responsibility |
|---|---|
| `hearth-config` | Settings loading and validation |
| `hearth-core` | Sessions, submissions, tasks, and the harness contracts |
| `hearth-storage` | SQLite access, migrations, snapshots, and verification |
| `hearth-model` | OpenAI-compatible provider adapter and SSE parsing |
| `hearth-memory` | Extraction, recall, consolidation, and the memory tool |
| `hearth-tools` | File, shell, web, message, schedule, skill, and task tools |
| `hearth-telegram` | Telegram transport, media, and rich replies |
| `hearth-runtime` | The agent loop, daemon, scheduler, service, backup, and init |
| `hearth-cli` | The `hearth` command |
| `hearth-eval` | Offline memory and token evaluations |

## Requirements

- Linux (x86_64 or aarch64), glibc.
- Rust 1.89 or newer and Cargo.
- A Telegram bot from BotFather.
- An OpenAI-compatible provider endpoint and key.
- Optional: Groq for speech, ElevenLabs for voice, an image provider for pictures, Exa, Brave, DeepSeek, Firecrawl, or TinyFish for search.

## Build and test

```bash
cargo build --release
./scripts/ci.sh
```

`scripts/ci.sh` runs formatting, clippy with warnings denied, and the full test suite. The release package is built with `./scripts/package.sh`.

## Quick start

See [QUICKSTART.md](QUICKSTART.md) for BotFather, provider, init, doctor, service, Telegram, backup, and troubleshooting steps.

## Commands

| Command | Purpose |
|---|---|
| `hearth init` | Create a home and a working settings file |
| `hearth doctor [--live]` | Check settings, database, lock, Telegram, and provider |
| `hearth daemon` | Run the long-lived agent |
| `hearth chat` | Talk to the agent from the terminal |
| `hearth watch` | Follow live events from the daemon |
| `hearth status` | Ask the daemon for status |
| `hearth stats` | Show usage and cost summaries |
| `hearth prompt` | Inspect the exact prompt sent to the provider |
| `hearth service install\|status` | Install or inspect the systemd user unit |
| `hearth backup` | Snapshot the database into the home |
| `hearth restore <file>` | Restore a snapshot with the daemon stopped |
| `hearth skills` | Inspect the skill library |
| `hearth eval memory\|tokens` | Run the offline evaluations |

All commands accept `--home <path>`; the default is `HEARTH_HOME`, then `~/.hearth`.

## Memory

Memory is layered: the system prompt and identity, standing memory, recent conversation, recalled memory, and the current turn. Saves are extracted, deduplicated, and versioned in SQLite with full-text search. Recall combines full-text and recency ranks. Consolidation archives stale or duplicate rows and keeps episodes. The memory benchmark in `hearth eval memory` reports recall, precision, temporal accuracy, and supersession on a fixed question set.

## Security

Hearth trusts one Telegram user, stores everything locally, and never sends telemetry. Tools run without a sandbox and only on the machine that hosts the database. Secrets are environment references and are redacted from logs and status. See [SECURITY.md](SECURITY.md).

## Spec and license

The behavior is defined by `SPEC.md`; the schema is defined by `SCHEMA.md` and the migrations. The project is licensed under the MIT License; see [LICENSE](LICENSE).
