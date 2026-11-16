# A failed onboarding cannot be retried

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `hearth-runtime`, `hearth-cli` |

## Observed

`init` writes `settings.json` before it validates the file and opens the
database. If a later step fails, the settings file exists, and every retry stops
at the "refusing to overwrite" guard. The wizard's retry key re-runs the same
`init`.

`crates/hearth-runtime/src/init.rs:53`:

```rust
let settings_path = home.join(SETTINGS_FILE);
if settings_path.exists() {
    return Err(InitError::Exists(settings_path));
}
...
let mut file = open_settings(&settings_path)?;      // :101
file.write_all(text.as_bytes())?;
...
let settings = hearth_config::load(home)?;          // :106
settings.validate_for_run()?;
let database = home.join("hearth.db");
hearth_storage::open(&database)?;                   // :110
```

`crates/hearth-cli/src/onboarding.rs:686`, the Failed step:

```rust
if let Some(options) = self.pending.clone() {
    self.step = Step::Creating;
    Action::Finish(Box::new(options))
```

`crates/hearth-cli/src/app.rs:1133` maps the failure back to `Step::Failed`, and
the retry runs `init` again with the same options.

A failure between `:101` and `:110` (a full disk, a database path that cannot be
opened, a storage error) leaves the home with settings and no database. On the
next launch, `App::new` sees the settings file and opens the Dashboard with a
banner error, so the wizard is not reachable either. The only exit is deleting
the file by hand.

## Consequence

A single mid-onboarding failure strands the home in a state the product offers no
recovery for, and the retry key gives the same "already exists" error forever.
The trigger is narrow, but the dead end is unconditional once it fires.

## Verification

Pre-create a home with no settings. Make `hearth_storage::open` fail by creating
a directory named `hearth.db`, run `init`, then run `init` again with valid
options. The second call returns `Exists`. The expected result is either no
settings file left behind on failure, or a retry that recognizes the file it
wrote and continues to the database step.

## Resolution

The settings write, the load-and-validate step, and the database open are one
fallible unit. If any of them fails, the settings file written by this attempt is
removed, so the wizard's retry finds a clean home instead of the
"already exists" guard.

Commit: `b096c86`. Verified by `a_failed_init_can_be_retried`, which blocks the
database path, runs `init`, asserts no settings file remains, clears the
blockage, and runs `init` again.
