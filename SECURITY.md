# Security

Hearth is a single-user, local-first agent. This document describes the trust
model and the boundaries it does not cross.

## Who can use it

Hearth accepts Telegram messages only from the numeric user ids in
`telegram.allowed_user_ids`. When `allowed_chat_ids` is non-empty, the chat must
also be allowed. An empty user list is a startup error, not an open bot. There is
no Hearth account and no multi-user permission system.

## What runs where

- The database, memory, transcripts, media, backups, and logs live under the
  Hearth home folder, by default `~/.hearth`.
- Model requests go to the configured provider. Optional transcription, speech,
  image, and search providers are separate choices.
- Telegram messages and attachments pass through Telegram's infrastructure.
  Telegram is a third-party transport. Do not send content you would not trust
  to that transport.
- There is no telemetry. Hearth does not contact a project server.

## Files and modes

`settings.json`, the database, the private socket, backups, and generated media
are created with user-only permissions where the platform supports them. The
settings file must be mode `0600`; the daemon refuses to start otherwise. Logs
are structured and exclude message bodies by default.

## Secrets

- Prefer environment references such as `${TELEGRAM_BOT_TOKEN}` over inline
  values in `settings.json`.
- Credentials are not written to prompts, memory, or transcripts by Hearth, and
  error messages, `hearth status`, `hearth prompt`, and the live watch redact
  configured secret values.
- `hearth prompt` and `hearth watch` are local views. Treat their output as
  sensitive anyway.
- The token and API keys are not committed. The repository's local scan checks
  the tree and history for common credential patterns and for a private
  deny-list kept outside the tree.

## The yolo trust model

Hearth ships without approval prompts and without a sandbox. The `shell` tool
runs commands on the host from the workspace, and the `files` tool reads and
writes within configured roots. The model can be influenced by untrusted web
pages, documents, forwarded messages, or tool output. Hearth labels web content
as untrusted data in the prompt, but that label is advisory.

Do not run Hearth with access to data or credentials you are not willing to
expose to the configured model provider. Guarded mode is deferred to a later
release.

## Untrusted content

Web pages and files are data, never instructions. Prompt injection is still a
risk. Review what the agent is asked to do, keep the workspace and secrets
separate, and use a dedicated machine or account when testing unfamiliar tools.

## Backups

Backups are complete database snapshots and contain conversations, memory, and
metadata. Store them with the same care as the database. The pre-restore copy is
kept beside the restored database. Media is not included in a database snapshot.

## Reporting a problem

Report security issues privately through the repository's Security tab using
GitHub private vulnerability reporting. Do not open a public issue.

## Public release

Publication is a separate decision. Before a public release, the history is
scanned against a private deny-list kept outside the tree, personal paths and
identifiers are removed from examples, and the release artifacts are checksummed.
