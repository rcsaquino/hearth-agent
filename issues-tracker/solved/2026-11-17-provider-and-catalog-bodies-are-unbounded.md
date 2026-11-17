# Provider and catalog response bodies are read without a bound

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-model` |

## Observed

The fetch layer bounds error bodies through `body::snippet`, which is the fix
recorded in `a-failed-fetch-buffers-the-error-body`. The provider and catalog
clients are outside that layer.

- `OpenAiCompatibleProvider::stream` calls `response.text().unwrap_or_default()`
  for every non-success status and keeps the whole body in
  `ProviderError::Request`.
- `list_models` does the same for a failed `/models` response.
- `catalog::fetch` calls `body.text()` for the whole models.dev catalog on
  success and does not cap it.

A configured endpoint or a compromised gateway can answer with a very large
body. The reqwest clients have 120 second or 20 second timeouts, which still
allows a large allocation before the timeout. The full body can also reach a
notice or a log after redaction, depending on the path.

## Verification

Point an `OpenAiCompatibleProvider` at a local fake server that announces a
large body with a `500` status, sends part of it, and stalls. The client keeps
reading until the timeout or the body end. The same server used with `web_fetch`
is bounded at 8 KiB by `body::snippet`.

## Resolution

`hearth_model::body` now holds the one bounded reader. Provider errors and the
wrong-content-type body are cut at 8 KiB, a `/models` success body is capped at
4 MiB, and the models.dev catalog is capped at 8 MiB. An oversized success body
is refused with a named limit instead of being parsed.

Commits: `b4bd511` (test), `a44abcf` (fix). Verified by
`a_large_provider_error_body_is_cut`, `list_models_rejects_a_body_over_its_cap`,
and `a_catalog_body_over_its_cap_is_refused`.
