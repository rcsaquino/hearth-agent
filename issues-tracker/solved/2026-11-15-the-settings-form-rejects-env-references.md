# The settings surface rejects the `${ENV_VAR}` form its own tooltips recommend

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-cli`, `hearth-config` |

## Observed

A home whose `settings.json` uses environment references cannot be edited from
the TUI. The form validates the literal `${NAME}` text without expanding it, so
`require` reports the variable as unset even when it is set.

`crates/hearth-cli/src/settings_form.rs:1694`:

```rust
fn validate(document: &Value) -> Result<(), String> {
    let settings: hearth_config::Settings = serde_json::from_value(document.clone())
        .map_err(|error| format!("Check this setting: {error}"))?;
    settings.validate().map_err(|error| error.to_string())
}
```

`crates/hearth-config/src/validate.rs:11`:

```rust
fn require(field: &str, value: &str) -> Result<(), ConfigError> {
    if let Some(name) = unresolved(value) {
        return Err(invalid(format!(
            "environment variable {name} is not set (referenced by {field})"
        )));
    }
```

`SettingsForm::open` reads the raw file (`settings_form.rs:146`) and never calls
`expand_value`. The daemon path does: `hearth_config::load` expands at
`lib.rs:64` before `validate` at `:67`. The raw editor has the same gap:
`crates/hearth-cli/src/app.rs:1333`, `validate_settings_text`, parses and
validates the text without expanding.

Every change goes through `apply_value` then `finish_change`
(`settings_form.rs:701`, `:712`), which reverts on a validation error. So the
form cannot save any change, and the raw editor cannot save either.

The form's own help text recommends the rejected form:
`settings_form.rs:1213` and `:1823` both say "You can also use a `${ENV_VAR}`
reference in settings.json."

## Consequence

A user who follows the recommended secret handling, and SPEC 10.1's own example
file, loses the whole settings surface. The only workaround the UI offers is
typing the secret inline, which is the practice the documentation discourages.
A home whose settings use `${...}` references is the common case for the
documented secret handling, so the fault is reachable on any such home. It is
not limited to a particular machine.

## Verification

Build a form from a document whose `models.api_key` is `${HEARTH_TEST_KEY}`,
export `HEARTH_TEST_KEY` with any value, and toggle a boolean row. Assert the
change is accepted. Today `FormAction::None` is returned and `form.error` names
`HEARTH_TEST_KEY` as unset. The same document loads through
`hearth_config::load`.

## Resolution

The form's `validate` expands `${NAME}` references before deserialising and
validating, using the public `hearth_config::expand_document`. The raw editor's
`validate_settings_text` does the same. A home whose settings use environment
references is editable again.

Commit: `b8a21a2`. Verified by `an_environment_reference_does_not_block_a_change`,
which toggles a row on a document whose API key is `${HOME}` and requires
`FormAction::Changed`.
