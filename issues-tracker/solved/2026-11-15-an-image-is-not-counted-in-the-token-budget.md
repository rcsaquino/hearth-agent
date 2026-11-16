# An image is not counted in the prompt token budget

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-core`, `hearth-model` |

## Observed

The token estimator counts an image entry as its file path. The provider request
sends a base64 PNG data URL, which costs thousands of provider tokens for a
normal photo.

`crates/hearth-core/src/prompt.rs:469`, `estimate_content`:

```rust
ContentBlock::Image { path, .. } => path.clone(),
```

The adapter reads the file, downscales to 1568 px, re-encodes PNG, and base64s it
on every request (`crates/hearth-model/src/openai.rs:555`):

```rust
let base64 = base64::engine::general_purpose::STANDARD.encode(encoded.into_inner());
Ok(format!("data:image/png;base64,{base64}"))
```

The estimated total drives the shrink loop, the hard-compaction trigger, and the
`usable` failure in `crates/hearth-runtime/src/agent.rs:509-538`. Telegram photo
blocks reach this path through `crates/hearth-telegram/src/poller.rs:264`.

## Consequence

An image-heavy conversation passes every local budget check and then fails at
the provider, or compacts later than it should. The image's provider cost in the
low thousands is not recorded locally, so `tokens_est` and the Overview understate
what the request actually spends. A single large photo can move a turn from under
the soft threshold to over the provider's context window with no local warning.

## Verification

`estimate_content` of `{"type":"image","path":"/home/u/media/x.jpg","mime":
"image/jpeg"}` returns a count near the path length. Serialize the same entry
through `wire_body` for a request and compare the image part's length. The two
differ by orders of magnitude.

## Resolution

An image entry now contributes 12,000 placeholder characters, about 3,300
tokens, the worst case for the 1568-pixel long edge the adapter sends. The
estimate feeds `tokens_est`, compaction, and the shrink loop.

Commits: `1e74da6` (test), `b455e01` (fix). Verified by
`an_image_carries_its_provider_cost`, which saved an image entry and found 6
tokens on the parent commit and at least 3,000 now.
