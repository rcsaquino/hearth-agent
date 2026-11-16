# Credential redaction eats ordinary hyphenated words

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-core`, `hearth-memory` |

## Observed

Any word containing the substring `sk-` is treated as a credential, so ordinary
text is rewritten in the permanent record and legitimate memory writes are
refused.

`crates/hearth-core/src/secrets.rs:108`, `contains_word_secret`:

```rust
fn contains_word_secret(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    for prefix in ["sk-", "ghp_", "xoxb-", "akia", "-----begin"] {
        if lower.contains(prefix) {
            return true;
        }
    }
```

The check is a substring test, not a prefix test on the token. `task-id`,
`task-level`, `disk-space`, `risk-free`, `desk-top`, and `asterisk-based` all
contain `sk-`. `redact_secrets` (`secrets.rs:58`) replaces the whole
whitespace-delimited word with `[redacted]`. A direct call makes the effect
plain:

```text
"open the task-id file"          -> "open the [redacted] file"
"prefers a disk-backed cache"    -> "prefers a [redacted] cache"
"the risk-free path"             -> "the [redacted] path"
```

The save path applies it to every entry. `crates/hearth-core/src/session.rs:193`,
`Session::save`:

```rust
let content_value = crate::secrets::redact_value(&entry.content);
```

That value is what lands in `entries.content_json`, which is what the history,
the Prompt tab, and the next request read. `contains_secret` (`secrets.rs:21`)
uses the same substring list and gates memory acceptance at
`crates/hearth-memory/src/store.rs:203`, so a statement such as "prefers a
disk-backed cache" is refused with `Code::Credential`.

## Consequence

The record silently rewrites ordinary engineering prose, and the model loses
those memory writes. The user's message is affected too: `Session::save` stores
the redacted entry, and the next turn builds its history from that row, so a
message such as "what does task-id mean" reaches the model as "what does
[redacted] mean". The documented intent is narrower: `docs/DECISIONS.md:945`
says the matcher "replaces the whole credential token with `[redacted]` and
preserves the surrounding text". The repository's own vocabulary collides:
`docs/DECISIONS.md:851` contains `spill/helper-<task-id>.txt`, and a user pasting
that path gets `helper-<task-[redacted]` back.

## Verification

Add `assert_eq!(redact_secrets("open the task-id file"), "open the task-id file")`
to `crates/hearth-core/tests/s1_secrets.rs`. It fails today with
`"open the [redacted] file"`. The existing tests cover only tokens shaped like a real key (a `sk-` prefix, or
`ghp_`, followed by a dummy body) and a clean string with no hyphenated word.
`contains_secret("prefers a disk-backed cache")` returns true, which is the same
condition that refuses the memory write.

## Related

`open/2026-11-15-prompt-blobs-are-destroyed-by-redaction.md` is a separate
consumer of the same matcher: serialized JSON has no whitespace, so one hit
takes the whole blob.

## Resolution

`contains_secret` and `redact_secrets` no longer test prefixes as substrings.
`credential_span` counts a prefix only at the start of a token and returns the
byte range of the credential; `redaction` replaces that range, so punctuation
and surrounding words survive. `task-id`, `disk-space`, `risk-free`,
`desk-top`, and `asterisk-based` are ordinary text, and a statement such as
"prefers a disk-backed cache" is accepted by the memory store.

Commits: `81978f9` (test), `bb38cea` (fix). Verified by
`cargo test -p hearth-core --test s1_secrets`, including the added
`ordinary_hyphenated_words_are_not_credentials` and
`redaction_keeps_a_serialized_document_parseable`.
