# A prompt blob is destroyed when redaction meets serialized JSON

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-core` |

## Observed

The Prompt tab can lose the very exchanges it exists to make inspectable. A
prompt containing any credential-shaped word is stored as a blob that is no
longer JSON, and every reader skips it.

`crates/hearth-core/src/session.rs:729`, `save_prompt_blob`:

```rust
let text = crate::secrets::redact_secrets(text);
let hash = Uuid::new_v5(&Uuid::NAMESPACE_URL, text.as_bytes()).to_string();
```

The caller passes compact JSON. `crates/hearth-runtime/src/agent.rs:549`:

```rust
let prompt_text = serde_json::to_string(&prompt.document)
...
let prompt_hash = self.session.save_prompt_blob(&prompt_text)?;
```

`redact_secrets` splits on whitespace (`secrets.rs:65`). A hit replaces the
whole whitespace-delimited run, and in a serialized document that run can carry
JSON punctuation. The result depends on the value's context:

```text
{"text":"open the task-id file"}     -> stays valid JSON, one word replaced
{"text":"task-id file"}              -> [redacted] file"}          invalid JSON
{"path":"spill/task-id.txt"}          -> [redacted]                  invalid JSON
{"files":["task-id"]}                 -> [redacted]                  invalid JSON
```

Any of those words contains `sk-`, `ghp_`, `xoxb-`, `akia`, or `-----begin`.
Ordinary text such as `task-id` in a tool argument, a path, or an id is enough.
The prompt document carries the full transcript and tool declarations, so a
single unspaced value is sufficient.

The readers reject a blob that no longer starts with `{` or does not parse:
`crates/hearth-runtime/src/inspect.rs:32` and `:81` both `continue` on that
condition. The entry's `prompt_hash` and the task checkpoint still point at the
unreadable blob.

## Consequence

The Prompt tab shows no snapshot for any request whose prompt contains one of
these words, and the audit trail for that turn is gone. SPEC 6.6 says the exact
prompt text is saved in `prompt_blobs` before the request. The save runs, but the
stored value is not the prompt.

## Verification

Build a `PromptInput` whose tool declaration or transcript contains the value
`task-id` with no surrounding space (a path `spill/task-id.txt` works), assemble
it, serialize the document, and pass it to `save_prompt_blob`. Then call
`latest_prompt()` and `prompt_exchanges()`. The stored text does not parse, so
both readers skip it. Asserting that the stored text parses as JSON fails today.

## Related

`open/2026-11-15-secret-redaction-eats-ordinary-hyphenated-words.md`
covers the matcher that causes both faults. Fixing the word test alone would
narrow this case but not remove it: a genuine token glued to JSON punctuation
would still consume the punctuation.

## Resolution

Redaction now replaces only the credential's byte range inside a word, so the
JSON punctuation around it is preserved and `save_prompt_blob` stores a
document the readers can parse. A tool path such as `spill/task-id.txt` is no
longer a match at all.

Commits: `9404cb7` (test), `bb38cea` (fix). Verified by the new
`latest_prompt_survives_redaction_of_an_ordinary_path` in
`crates/hearth-runtime/tests/inspect.rs`, which failed on the parent commit and
passes now, plus the round-trip parse assertion in `s1_secrets.rs`.
