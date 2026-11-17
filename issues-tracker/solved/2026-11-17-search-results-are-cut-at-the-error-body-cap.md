# Search cuts a successful response at the error-body limit

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-tools` |

## Observed

`search_providers::response_text` reads every provider response through
`body::snippet(response, ERROR_BODY_BYTES)` and only then checks the status.
`ERROR_BODY_BYTES` is 8 KiB, a bound that was introduced for error messages
after `a-failed-fetch-buffers-the-error-body`.

A successful search response is not an error message. `SNIPPET_CHARS` allows 800
characters per result and `web.search.max_results` allows 50 results, so a
normal response with ten results can exceed 8 KiB before JSON syntax and the
provider's own metadata. The truncated text no longer parses, and every JSON
provider reports a permanent `invalid JSON` failure. The tool then falls through
to the next provider or fails the whole call.

## Verification

A fake Brave server returned ten results with 1,000-character descriptions, a
valid body of more than 8 KiB. `BraveSearch::search` accepted the connection and
returned `Err` with an invalid JSON message. Each provider's `response_text` call
has the same cap, and the Exa MCP path parses SSE through the same helper.

The old fetch issue was about reading too much before keeping 200 characters,
which is a different requirement from reading a valid JSON document.

## Resolution

`search_providers::response_text` checks the status before reading. A failed
response still goes through the 8 KiB error snippet; a successful response is
read as a document with a 4 MiB bound.

Commits: `25543e0` (test), `0609941` (fix). Verified by
`a_large_successful_search_response_is_not_cut`, which serves ten 1,000-character
results in a valid body larger than the old cap and requires all ten to parse.
