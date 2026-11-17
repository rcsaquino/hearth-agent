# The log redacts credential shapes but not configured secret values

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `hearth-runtime` |

## Observed

`init_logging` installs `RedactingWriter`, which calls
`hearth_core::secrets::redact_secrets` on every log chunk. That matcher knows
named prefixes such as `sk-`, `ghp_`, and `AKIA`, Telegram bot token shape, and
`key=value` pairs. It does not know the values configured in `settings.json`.

`daemon::collect_secrets` builds the list of configured credentials and passes it
to `EventBus`, where the event stream replaces those values. The same list is not
passed to logging. A configured API key of at least eight characters that has no
recognized prefix is therefore logged verbatim if it appears in a provider error,
an IO message, or a debug line.

SPEC 10.2 and `SECURITY.md` say every configured credential value of eight or
more characters is redacted from logs. The log test only uses a credential-shaped
value, so it does not exercise the configured-value path.

## Verification

In a temporary home, call `init_logging`, then emit a line containing a custom
value such as `customsecret123456`, which has no recognized prefix. Read
`logs/hearth.log`. The value appears unchanged. Emit a `sk-test` value and it is
replaced, which shows the two paths differ.

## Resolution

`init_logging` loads the home settings, collects the configured credential values
with the same `collect_secrets` the event stream uses, and gives them to
`RedactingWriter`. `hearth_core::secrets::redact_with_values` applies the shape
matcher and then replaces every configured value of at least eight characters,
longest first.

Commits: `76048ae` (test), `15b96a4` (fix). Verified by
`the_daemon_log_redacts_shape_and_configured_credentials`, which writes a home
whose model key has no recognizable prefix, logs that key and a `sk-` value, and
requires neither in `logs/hearth.log`.
