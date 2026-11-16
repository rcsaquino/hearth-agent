# A settings parse error echoes the field value

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-config` |

## Observed

A `settings.json` value of the wrong type is reported with the value in the
message and no field name. When that value is a credential, the daemon prints it
to stderr and the journal before any redaction is installed.

`crates/hearth-config/src/lib.rs:32`:

```rust
#[error("settings.json is not valid JSON: {0}")]
Json(#[from] serde_json::Error),
```

`crates/hearth-config/src/lib.rs:64`:

```rust
expand_value(&mut value);
strip_legacy(&mut value);
let settings: Settings = serde_json::from_value(value)?;
settings.validate()?;
```

`serde_json`'s `invalid_type` Display includes the unexpected scalar. A file with
`"allowed_user_ids": "1234:AAH..."` (a quoted id, or the token pasted into the
wrong field) produces:

```text
settings.json is not valid JSON: invalid type: string "1234:AAH...", expected a sequence
```

`hearth_runtime::daemon` calls `hearth_config::load(home)?` as its first
statement (`crates/hearth-runtime/src/daemon.rs:68`). The error leaves `main` as
an `anyhow::Result` and is printed to stderr, which systemd records in the
journal. The redacting log writer is not on that path.

## Consequence

A token pasted into the wrong field is written verbatim to the journal and to
the terminal. `docs/SPEC.md:1627` says logs and error messages redact configured
credential values, and the ground rules forbid printing a token or a Telegram
id. The message is also hard to act on, because it names no field.

## Verification

Write a settings file whose `telegram.allowed_user_ids` is the bot token as a
quoted string. Run the daemon in a temporary home and read stderr. The token
appears in the message. A field with a bad type in `models` behaves the same
way.

## Resolution

`hearth_config::settings_from_value` replaces the raw serde error with
`ConfigError::Schema`, naming the section and field that failed. The field is
found by removing candidate keys and retrying, so no value is ever quoted. The
TUI's raw settings editor uses the same function, so both paths are covered.

Commits: `53d0915` (test), `78835e4` (fix). Verified by
`cargo test -p hearth-config --test error_text`, which asserts the token and the
id are absent from the message and `allowed_user_ids` is named.
