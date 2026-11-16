# A failed fetch buffers the whole error body before keeping 200 characters

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-tools` |

## Observed

On a non-2xx response, the direct fetcher reads the entire body into memory and
only then takes the first 200 characters. The success path already has a capped
reader; the error path does not use it.

`crates/hearth-tools/src/fetch.rs:329`:

```rust
if !status.is_success() {
    let retryable = status.as_u16() == 429 || status.is_server_error();
    let snippet: String = response
        .text()
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect();
```

`read_capped` is defined 30 lines later (`fetch.rs:363`) and used only after the
status check. The same late cap appears in `JinaFetch` (`fetch.rs:220`),
`json_response` for the TinyFish, Firecrawl, and DeepSeek providers
(`fetch.rs:697`), all five search providers
(`crates/hearth-tools/src/search_providers.rs:224`), and the browser snapshot and
screenshot reads (`crates/hearth-tools/src/browser.rs:65`, `:119`, `:185`).

The regression test `a_large_error_body_is_shortened`
(`crates/hearth-tools/tests/fetch.rs:346`) uses a 1000-byte body, so it proves
truncation of the message and not the bound on the read.

## Consequence

The target URL comes from a web page or a message, so a hostile or merely broken
site can answer `500` with a multi-gigabyte body and the daemon allocates it. The
reqwest client timeout is 30 seconds, which is a large streaming window on a fast
link. On a small host the kernel kills the daemon mid-run. A search provider and
the browser path have the same exposure through their own error handling.

## Verification

Point `HttpFetch::new("http", true, 2 * 1024 * 1024, 3, ...)` at a fake server
that writes 500 MB with a `500` status and watch the process memory. The success
path caps at 2 MB; the error path reads the whole body.

## Resolution

A new `body::snippet` reads at most 8 KiB from a response before a message is
built, and every late-truncation site uses it: the direct and Jina fetchers, the
TinyFish, Firecrawl, and DeepSeek JSON responses, all search providers, and the
browser error paths. The status and retryable classification are unchanged.

Commits: `b662614` (test), `5f4d093` (fix). Verified by
`an_enormous_error_body_is_not_buffered`, where a fake server announces a 512 MiB
body, sends 16 KiB, and then stalls; the fetch must return within three seconds.
