# A generation longer than two minutes is cut by the client timeout

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-model` |

## Observed

The blocking provider client sets a two-minute total request timeout. The timeout
covers the streamed response body, not just the connection and headers, so a
generation that runs longer than 120 seconds is cut even when the provider is
still producing a valid answer.

`crates/hearth-model/src/openai.rs:126`:

```rust
let client = reqwest::blocking::Client::builder()
    .timeout(Duration::from_secs(120))
    .build()
```

There is no read timeout and no per-request override. The configured
`models.max_output_tokens` is 32,000 by default, which at a modest 50 tokens per
second needs more than ten minutes. A slow provider, a long reasoning pass, or a
large `xhigh` answer reaches the ceiling well before the output limit.

The read error is surfaced as `ProviderError::Transport` (`openai.rs:512`), so
the run settles `Unanswered` with a provider-error notice rather than a silent
truncation. The failure is visible but unavoidable: the same answer would
succeed on a client that waits longer.

## Consequence

The product cannot produce an answer longer than two minutes of wall-clock
streaming, whatever the output budget says. A user who selects a long output
reserve or a reasoning level that thinks for minutes gets a provider error, and
the run's tokens are spent without an answer. The limit is not in settings, so
there is no way to raise it without rebuilding.

## Verification

Point the provider at a fake server that writes one SSE frame every ten seconds
for 150 seconds, with `[DONE]` at the end. Collect the stream; the read fails
with a transport error before `[DONE]`. The same server with a 90-second stream
completes. Assert the failure is `ProviderError::Transport`, not a protocol
error.

## Closure

**Not reproducible.** The stated mechanism is not what reqwest does in this
version. `ClientBuilder::timeout` is documented as a per-operation timeout
(`blocking/client.rs:383`, vendored reqwest 0.12.28: "Set a timeout for connect,
read and write operations"), and `blocking/response.rs:436-444` applies it to each
`Response::read` call, not to the response body as a whole.

`OpenAiStream` reads with `read_line`, one read per SSE frame, so the report's own
scenario (a frame every ten seconds for 150 seconds) does not hit the timeout.
Only a silent gap longer than 120 seconds fails, and that is the deliberate guard
against a connection that is dead rather than slow. A generation that keeps
producing output is not cut.

What was tried: read the vendored reqwest source at the two cited call sites and
traced `next()`/`finish()` in `crates/hearth-model/src/openai.rs`. No live test was
run: proving the negative needs a 150-second stream, and the source settles the
read boundary. A separate change to raise the silent-gap ceiling would be a
product choice, and the current ceiling is the stall guard.
