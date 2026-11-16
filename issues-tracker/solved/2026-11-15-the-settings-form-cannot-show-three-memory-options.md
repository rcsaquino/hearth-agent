# The settings form cannot show three documented memory options

| | |
| --- | --- |
| Kind | feature |
| Reported | 2026-11-15 |
| Area | `hearth-cli`, `hearth-config` |

## Requested

The Settings tab should have inline rows for every documented memory setting, as
it does for the other sections. Three are missing, so they are reachable only by
editing JSON by hand.

`crates/hearth-config/src/settings.rs:289` defines five memory fields:

```rust
pub struct Memory {
    pub rules_max_tokens: u64,
    pub recall_max_tokens: u64,
    pub recall_max_items: u64,
    pub embeddings: bool,
    pub embedding_model_dir: String,
}
```

`default_settings_value` (`settings.rs:503`) lists only two of them:

```rust
"memory": {
    "rules_max_tokens": default_rules_max_tokens(),
    "recall_max_tokens": default_recall_max_tokens(),
},
```

`SettingsForm::with_defaults` (`crates/hearth-cli/src/settings_form.rs:849`)
merges this defaults object into the document, then `build_rows` walks the merged
document. The memory section therefore appears on a fresh install, but it has
only the two rows the defaults object carries. `recall_max_items`, `embeddings`,
and `embedding_model_dir` get no row.

SPEC 10.1 documents all five, and `QUICKSTART.md:118` says Settings "shows every
option inline". The `Advanced` section offers "Edit settings.json", so the
values are still reachable, but the inline surface does not show them and there
is no reset target for them.

## Why it is worth doing

A user who wants lexical-only recall (the `embeddings` switch) or a different
memory item cap has no inline control and no way to reset the value to its
default. Because `compact_value` treats an unknown default as "not a default",
a memory value typed by hand is also never stripped, and the row that would show
its changed marker does not exist.

## Verification

Build a form from `{"identity":{"agent_name":"Hearth","timezone":"UTC"}}` and
search the rows for the three paths. None exists. Adding the three entries to
`default_settings_value` makes them appear through the existing flattening.

## Resolution

`default_settings_value` now lists all five memory fields, so the form builds rows
for `recall_max_items`, `embeddings`, and `embedding_model_dir` and can show and
reset them. The form and the raw editor expand `${NAME}` before validating, using
the same expansion the daemon performs.

Commit: `b8a21a2`. Verified by `every_memory_option_has_a_row` and
`an_environment_reference_does_not_block_a_change`, which toggles a row on a
document whose API key is `${HOME}`.
