# A queued Telegram submission that cannot run blocks every chat

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-runtime` |

## Observed

`TelegramWorker::run_one` returns early without settling or recording a notice
when it cannot resolve the submission's conversation, when `parse_target` rejects
the conversation's `transport_key`, or when `build_tools` fails. The submission
stays `queued`.

`Session::next_queued_telegram_submission` selects the globally oldest queued
Telegram submission, and `TelegramWorker::run` calls it again after every early
return. The same row is therefore selected on every iteration, and every newer
message in every chat waits behind it. The existing busy-run wedge has a
different cause but the same global-queue effect.

## Consequence

One malformed conversation, one bad transport key, or one tool construction
failure stops all Telegram traffic until someone edits the database or
restarts with the condition fixed. There is no user-visible notice and no
retry limit.

## Verification

Insert a queued submission whose `conversation_id` does not exist, start the
worker with a valid submission in another conversation behind it, and observe
that the second submission is never selected. A pair of fake chats, one with a
malformed `transport_key`, shows the same result. The failing branches are the
`run_one` early returns before `Agent::new`.

## Resolution

`TelegramWorker::run_one` settles a submission it cannot run as `unanswered`
with a reason, and stores a visible notice when the conversation still exists.
The three early returns for a missing conversation, an unreadable conversation,
an invalid transport key, and failed tool construction all use that path, so the
globally oldest queued submission always leaves the queue.

Commits: `904ecc3` (test), `69b7534` (fix). Verified by
`a_poisoned_submission_does_not_block_the_next_chat`, which queues a submission
with an invalid transport key ahead of a valid chat, starts the daemon, requires
the valid chat to be answered, and then requires the first submission to be
`Unanswered` with a notice.
