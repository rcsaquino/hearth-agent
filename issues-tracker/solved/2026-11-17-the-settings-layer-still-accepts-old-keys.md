# The settings layer still accepts keys from earlier builds

| | |
| --- | --- |
| Kind | chore |
| Reported | 2026-11-17 |
| Area | `hearth-config`, `hearth-cli` |

## Observed

Several removed settings are still accepted or rewritten.

- `hearth_config::strip_legacy` removes `identity.soul_file` and `models.pricing`
  before parsing, so a file that sets them loads. `compact_settings` drops both
  when it writes. `docs/DECISIONS.md` records their removal on 2026-11-12.
- `hearth_cli::settings_form::from_document_at` removes `identity.soul_file`,
  `models.pricing`, and `tasks.eta_reminder_minutes`. The form then saves a file
  that silently lost the keys.
- `hearth_config::Tasks` still has `eta_reminder_minutes` and
  `hearth-config/tests/tasks_settings.rs` has `the_old_minutes_setting_still_parses`.
  Nothing reads the field. SPEC 10.2 says the old key is accepted but ignored.
- `hearth_cli::settings_form::normalize_fetch_aliases` rewrites `https` to
  `http`, and `is_fetch_provider` plus `build_fetch_providers` still accept
  `https`. SPEC 10.2 says the alias remains accepted for existing files.
- `app::migrate_daemon` writes a `daemon` section when one is absent and a unit
  is installed. That is a settings migration from a build before `daemon`
  existed, and it writes the settings file during TUI startup.

## Wanted

One settings schema with no removed-key exceptions. An old key should fail with
the same unknown-field error as a typo, or a one-time migration should be
explicit and separate from every load. Remove the legacy branch, the alias, the
unused field, and the tests that pin them.

## Verification

A settings file containing each key loads through `hearth_config::load`; the
form opens with the key removed from its document. `https` in
`web.fetch.order` validates and builds a direct fetcher. A home with a unit and
no `daemon` section gets one written on TUI startup. Each is a previous-build
path, not current behavior.

## Resolution

The load path no longer has removed-key exceptions. `identity.soul_file`,
`models.pricing`, and `tasks.eta_reminder_minutes` are unknown fields, and the
Settings form no longer strips them or rewrites `https` to `http`.
`tasks.eta_reminder_minutes` is removed from `Tasks`, the `https` fetch alias is
removed from validation and the fetch chain, and the TUI no longer adds a
`daemon` section on startup. The SPEC and decision log record the strict schema.

Commit: `ae667c1`. Verified by `a_legacy_soul_file_setting_is_rejected`,
`the_old_minutes_setting_is_rejected`, `web_fetch_rejects_the_removed_https_alias`,
and the existing `hearth-cli` form tests.
