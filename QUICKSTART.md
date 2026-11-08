# Hearth Quick Start

This walks through a first install with one Telegram bot and one
OpenAI-compatible model provider. Replace `USER_NAME`, `123456789`,
`~/.hearth`, and `example.com` with your own values.

## 1. Create the Telegram bot

1. Open Telegram and talk to `@BotFather`.
2. Send `/newbot`, choose a display name and a username ending in `bot`.
3. Copy the bot token. Store it in an environment variable, not in a shell
   history file:

```bash
export HEARTH_TELEGRAM_BOT_TOKEN='your-bot-token'
```

## 2. Get your numeric Telegram id

Message `@userinfobot` and copy the numeric id it returns. Hearth accepts only
the ids listed in `telegram.allowed_user_ids`; an empty list is a startup error.

## 3. Build and install

```bash
cargo build --release
sudo install -m 755 target/release/hearth /usr/local/bin/hearth
```

You can skip the system install and run `./target/release/hearth` directly.

## 4. Initialize the home

Interactive setup asks for the token, your user id, the provider base URL, API
key, and model, then offers optional media blocks:

```bash
hearth init
```

For a scripted setup:

```bash
export PROVIDER_API_KEY='your-provider-key'

hearth init --non-interactive \
  --telegram-token '${HEARTH_TELEGRAM_BOT_TOKEN}' \
  --allowed-user-id 123456789 \
  --base-url 'https://example.com/v1' \
  --api-key '${PROVIDER_API_KEY}' \
  --model 'your-model-name' \
  --agent-name Hearth \
  --user-name USER_NAME \
  --timezone UTC
```

`hearth init` creates `~/.hearth`, writes `settings.json` with mode `0600`,
creates `SOUL.md`, and opens the database. Values written as `${NAME}` are
expanded from the environment when Hearth runs.

## 5. Check the install

```bash
hearth doctor
```

The offline check validates settings, the database, the schema, and the lock.
With a real token and provider you can add one live check:

```bash
hearth doctor --live
```

It sends a Telegram `getMe` and one small provider request. It never prints the
token or key.

## 6. Run the daemon

Install the systemd user unit:

```bash
hearth service install
systemctl --user start hearth
systemctl --user status hearth
```

Or run it in the foreground:

```bash
hearth daemon
```

While it runs:

```bash
hearth status
hearth watch
hearth watch --json
```

`hearth status` requires the daemon socket. `hearth watch` streams live events
from the active run; detaching does not cancel the run.

## 7. Talk to it

Open your bot in Telegram and send a message. There are no slash commands. Send
`stop` to cancel an active run. Hearth sends one final answer when the run
settles.

Send images, documents, or voice notes as you would to any contact. Voice notes
become text through the configured transcription service. The `message` tool can
send files, albums, and, when you explicitly ask for voice, a spoken reply.

For a terminal conversation without Telegram:

```bash
hearth chat
```

## 8. Backups

```bash
hearth backup
```

Snapshots go to `~/.hearth/backups/` and are pruned to seven daily and four
weekly copies. A nightly backup runs from the daemon. To restore, stop the
daemon first:

```bash
systemctl --user stop hearth
hearth restore ~/.hearth/backups/hearth-YYYYMMDD.db
systemctl --user start hearth
```

## 9. Troubleshooting

- **`another hearth daemon is already running`**: only one daemon may own a
  home. Stop the old one, or use a different `--home`.
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
- **Empty or odd answers**: inspect the exact prompt with
  `hearth prompt --last` or `hearth prompt --json`. Credentials are redacted.
- **Runaway tools**: send `stop`. The shell is not sandboxed; read
  [SECURITY.md](SECURITY.md).
- **Logs**: `~/.hearth/logs/hearth.log` is JSON and has no message bodies by
  default. `hearth doctor` is the first check.

The full behavior is in [SPEC.md](SPEC.md).
