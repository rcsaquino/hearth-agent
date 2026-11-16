# An unknown timezone is accepted and silently becomes UTC

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-config`, `hearth-core` |

## Observed

`identity.timezone` is validated only as a non-empty string. An unknown name
passes, and every render falls back to UTC without telling the user.

`crates/hearth-config/src/validate.rs:50`:

```rust
require("identity.agent_name", &self.identity.agent_name)?;
require("identity.timezone", &self.identity.timezone)?;
```

`require` checks only for an unresolved environment reference and an empty
string (`validate.rs:11`). It does not resolve an IANA zone. `Zone::resolve`
(`crates/hearth-core/src/zone.rs:28`) does:

```rust
match TimeZone::get(trimmed) {
    Ok(zone) => Self { name: trimmed.to_owned(), zone },
    Err(_) => Self::utc(),
}
```

`init` copies the environment's `TZ` verbatim
(`crates/hearth-runtime/src/init.rs:169`), so common shell forms such as
`TZ=:Asia/Manila`, `TZ=UTC-8`, or a path form are stored as the setting and
resolve to UTC forever.

SPEC 10.2 lists "`identity.timezone` is an IANA zone name" under what is
checked. The fallback is deliberate for display safety, but the setting is never
validated, so a typo or a non-IANA `TZ` value produces a record where every
timestamp, the nightly backup slot at 03:00, and the skill-review slot at 04:30
are in the wrong zone with no warning in the Doctor or the settings form.

## Consequence

A user in a non-UTC zone whose value is mistyped sees every rendered timestamp
shift by the offset, and backups and skill review run at UTC wall-clock times.
Nothing surfaces the fallback: `Zone::name()` returns `UTC`, which looks like a
deliberate choice rather than a failed lookup.

## Verification

Load a settings file with `"identity": {"timezone": "Europe/Lundun"}`. It loads
and `Zone::resolve` returns UTC. The expected behavior is a validation error
naming the field, as the specification says.

## Resolution

`identity.timezone` is resolved against the IANA database during validation.
`hearth-config` uses the same `jiff` database and features as `hearth-core`, so
there is one zone authority. The display path stays lenient.

Commit: `32eb3f7`. Verified by `an_unknown_zone_is_refused_by_validation`, which
loads `Europe/Lundun` and requires an error naming the field.
