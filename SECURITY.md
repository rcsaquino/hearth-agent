# Security

## Trust model

Hearth is built for one person. The Telegram allowlist is the only access control. Messages from any other id are ignored, and no command, tool, or schedule is exposed to them. Do not run Hearth for a group, a team, or an untrusted user.

The bot token is a full credential for the bot. Anyone who has it can read updates and send messages as the bot. Treat it like a password.

## File permissions

- The home directory is created 0700.
- `settings.json`, `logs/`, backups, and the local socket are 0600.
- The SQLite database is 0600.
- The systemd unit contains no secrets.

Hearth does not loosen these modes. If a file is readable by other users, check ownership and the umask rather than changing the mode.

## Secrets

Secrets belong in the environment, not in the repository. Settings store `${ENV_VAR}` references and resolve them at startup. The token, provider key, and media keys are redacted from logs, status, doctor output, and error messages. A missing variable is reported by name, never by value.

The model receives conversation content and tool results, and the provider receives the prompt. Do not put secrets in a message, a file the agent reads, or a memory row unless you accept that the provider will see them.

## Tools and the machine

Hearth runs tools with the privileges of its user and without a sandbox. `file_write` and `shell` can change anything that user can change. This is intentional: the agent is a personal operator on a machine the user controls. Run it on a dedicated account or a dedicated machine if that risk is not acceptable.

The model decides which tools to call. Prompt injection through web pages, files, or messages is possible; treat tool results as untrusted content. Hearth labels web and tool content separately from user instructions, but labels are not a security boundary.

## Runtime network access

The daemon contacts Telegram, the configured model provider, and the optional search, speech, image, and web-fetch services. It does not send telemetry or usage data anywhere else. All state stays in the local home.

## Backups

Snapshots contain the full conversation, memory, and ledger history. They are written 0600 and should be stored with the same care as the live database. `restore` verifies integrity and schema version, refuses while the daemon is running, and keeps a pre-restore copy.

## Reporting a vulnerability

Report privately to the maintainer; do not open a public issue with a working exploit, a token, a key, or personal data. Include the affected version, the smallest reproduction, and the impact. There is no bug bounty.
