# The Prompt tab keeps a stale request when its notice arrives

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-cli` |

## Observed

`PromptView::refresh_prompt` loads a new list of saved exchanges, and
`ui::prompt::render` rebuilds the cached chat only when
`ui::prompt::CacheKey` changes. `CacheKey::new` hashes the exchange
`prompt_hash` and `response["entry_id"]`. It does not include the exchange
`notice`.

A request loaded while it is in flight has no response and no notice. The cache
is built with that state. If the request later settles without an answer, a
notice entry is linked to the same prompt hash. `inspect::prompt_exchanges`
returns the request with `response: null` and a populated `notice`, and
`derive` is written to print `no reply · <cause>` from that notice. The cache
key is unchanged, so `derive` is not called and the tab continues to show the
in-flight state until something else changes the width, fold, zone, or loaded
set.

The notice text and reason can also change while the response stays null, and
the same key misses that update.

## Verification

Build a prompt exchange with a request and no response or notice, render the
Prompt tab, and capture the buffer. Add `notice: {"reason": "provider_error"}` to
that exchange and render again. The second buffer still lacks the notice line
because `CacheKey::new` returns the same value. A refresh triggered by a
`run_settled` event reaches this path in normal use.

## Resolution

`ui::prompt::CacheKey::new` includes the exchange's notice in its signature, so
a notice that arrives while the response is still null rebuilds the chat.

Commits: `2dae5ee` (test), `905e350` (fix). Verified by
`a_notice_that_arrives_later_redraws_the_chat`, which renders an in-flight
request, adds a provider-error notice to the same exchange, and requires the
second frame to show `no reply · provider error`.
