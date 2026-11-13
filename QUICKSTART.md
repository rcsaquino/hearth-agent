# Hearth Quick Start

This walks through a first install with one Telegram bot and one
OpenAI-compatible model provider. Replace `123456789`, `~/.hearth`, and `example.com` with your own values.

## 1. Create the Telegram bot

1. Open Telegram and talk to `@BotFather`.
2. Send `/newbot`, choose a display name and a username ending in `bot`.
3. Copy the bot token. Keep it out of shell history and committed files.

## 2. Get your numeric Telegram id

Message `@userinfobot` and copy the numeric id it returns. Hearth accepts only
the ids listed in `telegram.allowed_user_ids`; an empty list is a startup error.

## 3. Build and install

```bash
cargo build --release
sudo install -m 755 target/release/hearth /usr/local/bin/hearth
```

You can skip the system install and run `./target/release/hearth` directly.

## 4. Open Hearth

Run the binary with no arguments:

```bash
hearth
```

The first launch opens the setup wizard. It asks for:

- the Telegram bot token from `@BotFather` and your numeric `@userinfobot` id;
- a provider: DeepSeek, OpenCode, OpenCode Go, Command Code, or another
  OpenAI-compatible endpoint;
- the provider API key, with a short hint for where to create it;
- a model, loaded from the provider's `/models` endpoint and searchable; if the
  list cannot be loaded, type the model name;
- the default thinking level: provider default, off, low, medium, high, xhigh, or
  max;
- the agent name, prefilled with `Hearth`;
- optional image (OpenRouter), transcription (Groq), and speech (ElevenLabs)
  services;
- when Hearth should start: when you open the TUI, at login, or at startup;
- whether the daemon should keep running after the window closes.

Paste keys with Ctrl+Shift+V (Ctrl+V also works in terminals that forward it).

The TUI is also the control plane: **Settings** shows every option inline, with `r`
to reset the selected row and `*` marking values that differ from the default. It
also edits `SOUL.md` and `HEARTH.md` directly.
**Memory** browses saved memories, and **Skills** can create or edit your own skills.

Finish writes `~/.hearth/settings.json` with mode `0600`, keeping only values that
differ from the defaults. It creates `SOUL.md`, `HEARTH.md`, and the database,
applies the start mode, and starts or attaches to the daemon. Values written as
`${NAME}` are expanded from the environment when Hearth runs.

The timezone comes from the system (`TZ`, `/etc/localtime`, `/etc/timezone`, then
UTC). There is no prompt.

If your gateway routes requests by session, set `models.session_header` to the
header it expects, for example `x-opencode-session`; Hearth sends the
conversation id in that header. Setup adds it automatically for opencode-go
URLs. Leave it out for providers that do not need it.

### Guidance files

- `HEARTH.md` in the home holds your rules and workspace guidance. Edit it freely;
  it is read on every run and starts empty. It is for instructions, not identity.
- `AGENTS.md` files hold project rules. Hearth reads every `AGENTS.md` from the
  filesystem root down to the workspace, most specific last. Put the project file at
  the workspace root or above it.
- Nested `AGENTS.md` files below the workspace are not loaded automatically. Ask
  Hearth to read one if a subproject has its own rules.
- `SOUL.md` holds Hearth's identity and voice. `HEARTH.md` and `AGENTS.md` tell it
  what to do.

## 5. Check the install

Open the Doctor tab with `6` (or `Tab`). Press `Enter` for offline checks, or `l`
for live checks. The offline check validates settings, the database, the schema,
and the lock. The live check sends a Telegram `getMe` and one small provider
request. Neither prints the token or key.

## 6. The TUI

Running `hearth` opens the control plane:

- **Overview** combines runtime, model, context, turn limit, usage and cache,
  tasks and memory, provider connections, and recent activity.
- **Activity** streams live runs, including Telegram messages and tool calls.
- **Memory** browses saved memories.
- **Skills** browses the skill library read-only.
- **Prompt** is a live, scrollable export of the newest saved session: system
  prompts, tool declarations, the full conversation, the reply, and token counts.
  It refreshes after every reply. Use `[`/`]` for older snapshots and `f` to
  resume following the tail.
- **Doctor** runs offline and live checks.
- **Settings** is the last tab and shows every option inline. Use `r` to reset the
  selected row, `*` to spot changes, and the daemon controls to start, stop, or
  restart the daemon. It also edits `SOUL.md` and `HEARTH.md`.

Use `Tab`/`Shift+Tab` or `1`..`8` to move, `?` for help, and `q` to quit.
Conversations happen in Telegram; the TUI does not host a local chat. A
TUI-started daemon runs detached, so closing the window does not stop it when
keep-running is on. A service-managed daemon is never stopped by the TUI.

## 7. Talk to it

Open your bot in Telegram and send a message. There are no slash commands. Send
`stop` to cancel an active run. Hearth sends one final answer when the run
settles.

Send images, documents, or voice notes as you would to any contact. Voice notes
become text through the configured transcription service. The `message` tool can
send files, albums, and, when you explicitly ask for voice, a spoken reply.

## 8. Backups

Snapshots go to `~/.hearth/backups/` and are pruned to seven daily and four
weekly copies. The daemon writes them automatically.

## 9. Troubleshooting

- **`another hearth daemon is already running`**: only one daemon may own a
  home. Open the TUI and use Settings to stop it, or use a different
  `HEARTH_HOME`.
- **Telegram conflict or duplicate polling**: only one process may call
  `getUpdates` for a bot. Do not run two daemons with the same token.
- **Bot ignores you**: check `telegram.allowed_user_ids` and, for groups,
  `allowed_chat_ids`; both must allow the message.
- **`settings.json has mode ...`**: run `chmod 600 ~/.hearth/settings.json`.
- **Provider 401 or 404**: check the base URL, key, and model name. Most
  OpenAI-compatible gateways use `https://example.com/v1` and no trailing
  `/chat/completions`.
- **Telegram 429**: Hearth honors `retry_after`; a busy bot may still pause.
- **Rich reply rejected**: Hearth falls back to plain messages with entities.
  The answer is still delivered once.
- **Empty or odd answers**: open the Prompt tab to inspect the whole session and
  the exact prompt the model received.
- **Runaway tools**: send `stop`. The shell is not sandboxed; read
  [SECURITY.md](SECURITY.md).
- **The window is too small**: resize to at least 80 columns by 24 rows.
- **Logs**: `~/.hearth/logs/hearth.log` is JSON and has no message bodies by
  default. The Doctor tab is the first check.

The full behavior is in [SPEC.md](docs/SPEC.md).
