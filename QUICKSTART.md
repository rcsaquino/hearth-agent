# Quick start

## 1. Create the bot

1. Open Telegram and talk to `@BotFather`.
2. Send `/newbot`, choose a name and a username, and copy the token.
3. Keep the token out of the repository and out of shell history.

Export it for this shell:

```bash
export HEARTH_TELEGRAM_BOT_TOKEN='your-token-here'
```

## 2. Find your Telegram id

Message `@userinfobot`, or send any message to your new bot and read the `from.id` in the update. That number is the only allowed user.

## 3. Choose a provider

Hearth speaks the OpenAI Chat Completions API. You need a base URL, a key, and a model name. Example:

```bash
export HEARTH_PROVIDER_KEY='your-key-here'
```

## 4. Create the home

Interactive:

```bash
hearth init
```

It prompts for the token, the allowed id, the base URL, the key, the model, the agent and user names, and the timezone, then offers optional image, speech-to-text, and text-to-speech blocks. Secrets are not echoed.

Non-interactive:

```bash
hearth init --non-interactive \
  --telegram-token '${HEARTH_TELEGRAM_BOT_TOKEN}' \
  --allowed-user-id 123456789 \
  --base-url https://example.com/v1 \
  --api-key '${HEARTH_PROVIDER_KEY}' \
  --model your-model \
  --agent-name Hearth \
  --user-name YourName \
  --timezone Asia/Manila
```

The `--telegram-token` and `--api-key` values may be `${VAR}` references; Hearth stores the reference and reads the variable at runtime. The default home is `~/.hearth`; pass `--home <path>` to use another.

## 5. Check it

```bash
hearth doctor --home ~/.hearth
hearth doctor --home ~/.hearth --live
```

The first is offline. `--live` calls Telegram `getMe` and makes one minimal provider request; failures are reported without printing the token or key.

## 6. Run it

Foreground, for a first look:

```bash
hearth daemon --home ~/.hearth
```

Or install the systemd user service:

```bash
hearth service install
hearth service status
systemctl --user status hearth.service
journalctl --user -u hearth.service -f
```

The unit is written to `~/.config/systemd/user/hearth.service` (or `$XDG_CONFIG_HOME/systemd/user`) and runs `hearth daemon --home <home>` with `Restart=on-failure`. Re-running `install` with a different unit requires `--force`.

Stop it before restoring a backup:

```bash
systemctl --user stop hearth.service
```

## 7. Talk to it

Send a Telegram message to the bot. Only the allowed id is answered. Text, images, voice notes, and files are accepted; replies use Telegram entities, and long replies are split at 4096 characters.

From the terminal:

```bash
hearth chat --home ~/.hearth
```

## 8. Inspect it

```bash
hearth status --home ~/.hearth
hearth watch --home ~/.hearth
hearth prompt --last --home ~/.hearth
hearth prompt --last --json --home ~/.hearth
hearth stats --home ~/.hearth
```

`watch` and `status` use the local socket `hearth.sock` (mode 0600). `prompt` shows the last assembled prompt, its segments, and its cache split. `stats` shows usage and cost; an unpriced model is reported as `unpriced`.

## 9. Memory and skills

Memory is managed by the agent in chat through the `memory` tool; ask it to remember or forget something. The memory benchmark is:

```bash
hearth eval memory --home ~/.hearth --json
```

Skills live in `skills/user/` and `skills/learned/`:

```bash
hearth skills list --home ~/.hearth
hearth skills show <name> --home ~/.hearth
hearth skills history <name> --home ~/.hearth
hearth skills diff <name> --home ~/.hearth
hearth skills stats --home ~/.hearth
```

## 10. Back up and restore

```bash
hearth backup --home ~/.hearth
hearth restore ~/.hearth/backups/hearth-20260101.db --home ~/.hearth
```

`backup` works while the daemon runs and keeps the newest 7 daily snapshots plus the newest snapshot of each of the 4 most recent weeks. A silent nightly backup runs at 03:00 local. `restore` requires the daemon stopped, verifies the snapshot, keeps a `pre-restore-<timestamp>.db` copy, and replaces the database atomically.

## 11. Packaging

```bash
./scripts/package.sh
./scripts/package.sh aarch64-unknown-linux-gnu
```

The first builds the host target. The second requires `rustup target add aarch64-unknown-linux-gnu` and a cross linker; the script fails clearly if either is missing. Packages are written to `dist/` with a SHA-256 checksum.

## Troubleshooting

- **Telegram 401 or `getMe` fails.** The token is wrong or revoked. Set `HEARTH_TELEGRAM_BOT_TOKEN`, confirm the settings reference, and run `hearth doctor --live`.
- **The bot ignores you.** The sender id is not in `allowed_user_ids`. Add it to the settings and restart the daemon.
- **Provider 401 or 404.** The base URL usually needs the `/v1` suffix, and the key must belong to that endpoint. Test the same URL and key with a small request.
- **Permission errors.** The home is created 0700, settings and logs 0600, and the socket 0600. Check ownership and `umask`; do not loosen the modes.
- **`database is locked` or a daemon start refusal.** Another daemon holds the lock. Stop it (`systemctl --user stop hearth.service`) before restoring or starting a second one. A stale lock is cleared by the next clean start.
- **Costs show `unpriced`.** The model has no price table entry. Add the model to the pricing settings; usage is still recorded.
- **A backup is refused.** The path must be a Hearth database with the expected schema version and a clean integrity check.
