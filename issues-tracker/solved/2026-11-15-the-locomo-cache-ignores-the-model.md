# The LoCoMo harness reuses another model's cached answers

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-eval` |

## Observed

The end-to-end LoCoMo runner caches answers by sample and question position. The
model name is not in the key, and the default cache path is shared across runs,
so changing `BENCH_MODEL` reuses the previous model's answers and reports them
under the new model's name.

`crates/hearth-eval/src/bin/locomo_e2e.rs:234`:

```rust
// Answers are cached so changing the scorer never costs another model call.
let cache_path = PathBuf::from(
    std::env::var("BENCH_CACHE")
        .unwrap_or_else(|_| "/tmp/hearth-temp/locomo-e2e-cache.json".to_owned()),
);
let mut cache: BTreeMap<String, String> = std::fs::read_to_string(&cache_path)
```

`locomo_e2e.rs:316`:

```rust
let key = format!("{sample}:{position}");
```

`locomo_e2e.rs:374` writes the answers back under the same key. The comment
scopes the cache to scorer changes, so cross-model reuse is unintended.

## Consequence

The one thing a LoCoMo harness is used for is comparing models. A run with a new
`BENCH_MODEL` prints `model: <new model>`, prints `cached N of N answers`, and
reports the previous model's F1 unchanged. A stale cache from another checkout
on the same host does the same silently. The numbers look like a model result
and are not one.

## Verification

Run `BENCH_MODEL=<model-a> locomo_e2e <data> 5`, then
`BENCH_MODEL=<model-b> locomo_e2e <data> 5` without deleting the cache. The
second run reports `cached 5 of 5` and identical per-category scores. Including
the model name (and ideally the prompt) in the key makes the second run call the
provider.

## Resolution

The cache key includes the model, so a different `BENCH_MODEL` calls the provider
instead of reusing another model's answers.

Commit: `92e717d`. Verified by the `a_cache_entry_belongs_to_one_model` unit test
in the binary.
