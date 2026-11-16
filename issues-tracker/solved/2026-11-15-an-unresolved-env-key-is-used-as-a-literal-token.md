# An unresolved environment reference becomes a literal provider key

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-config`, `hearth-runtime` |

## Observed

A missing environment variable is left in the settings value as the literal
`${NAME}`. Required fields reject that literal, but optional provider keys do
not, so the provider is built with the placeholder as its credential.

`crates/hearth-config/src/lib.rs:183`, `expand_string`:

```rust
match std::env::var(name) {
    Ok(value) => result.push_str(&value),
    Err(_) => result.push_str(&rest[start..start + 2 + end + 1]),
}
```

`crates/hearth-runtime/src/tools.rs:253`, `build_web_search`:

```rust
let key = search
    .providers
    .get(name)
    .and_then(|provider| provider.api_key.clone())
    .filter(|key| !key.is_empty());
```

`build_fetch_providers` does the same at `tools.rs:322`. Neither calls
`hearth_config::unresolved`, so `${BRAVE_API_KEY}` passes the non-empty filter
and `BraveSearch::new` receives it as the key. The provider joins the chain.

SPEC 10.2 says "A provider with no key is skipped, not an error", and
`docs/DECISIONS.md:373` records keyed providers as joining "only when their key
is present and non-empty". SPEC 10.1's complete example file uses `${...}` for
Brave, DeepSeek, Firecrawl, and TinyFish, so the documented install is the case
that hits this.

## Consequence

Every search attempt sends `Authorization: Bearer ${BRAVE_API_KEY}` (or the
equivalent) to a real provider, gets a 401, and only then falls through to the
next provider. That costs a round trip per query, consumes one of the
`parallel` slots, and makes Doctor unable to distinguish an absent key from a
present but bogus one. The literal placeholder also appears in any request
trace the provider keeps.

## Verification

Build a `Settings` from `{"web":{"search":{"order":["exa","brave"],"providers":
{"brave":{"api_key":"${HEARTH_TEST_UNSET}"}}}}}`, ensure
`HEARTH_TEST_UNSET` is unset, and call `build_tools`. Inspect the search
provider list. Exa is present and Brave is present with the literal placeholder.
The expected list contains only Exa.

## Resolution

`hearth_config::is_resolved` is the shared test for a usable configured value:
non-empty and free of an unresolved `${NAME}` reference. Both the search and
fetch chains filter provider keys through it, so a provider whose variable was
unset is skipped instead of joining with the literal placeholder.

Commits: `8de1a0a` (test), `241d0ce` (fix). Verified by the
`an_unresolved_environment_key_does_not_join_a_chain` unit test, which showed
`["exa", "brave"]` and `["tinyfish", "http"]` before the fix.
