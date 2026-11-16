# The packaged unit is not the unit the code writes or recognizes

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-15 |
| Area | `packaging`, `hearth-runtime` |

## Observed

The tarball ships `packaging/hearth.service`, but no code reads it. The service
module generates its own text, and its parser cannot recognize the packaged
file's home.

`packaging/hearth.service:9`:

```text
Environment="HEARTH_HOME=%h/.hearth"
ExecStart=/usr/local/bin/hearth
```

`crates/hearth-runtime/src/service.rs:93`, `status_at`:

```rust
let installed = unit.is_file() && unit_home(&unit).is_some_and(|named| named == home);
```

`unit_home` (`service.rs:322`) strips the quotes and reads the value literally.
It does not expand systemd specifiers, so for the packaged file it returns
`%h/.hearth`, which never equals a resolved home path. On a host that copied the
packaged unit into `~/.config/systemd/user/`, Doctor reports the service as not
installed for that home.

`service.rs:169`, `apply_start_mode` for `login` and `startup`, calls
`install_at(home, config_dir, true, run)` with `force = true`. That overwrites
the packaged file with the generated text (`unit_content` at `service.rs:292`,
built from `current_exe()` and the resolved home).

No test or script compares the packaged file with the generated text.
`scripts/check-docs.sh` only checks that the file is non-empty;
`scripts/test-scripts.sh:102` only checks that it is in the tarball.

## What should happen

The shipped file and the generated file should be one artifact. Either the
packager stages the text the code generates, or the code recognizes and uses the
packaged form, or the file and the sentence that describes it are removed. The
current state gives an operator a working unit that the product disowns and then
silently replaces.

## Verification

Copy `packaging/hearth.service` into a temporary config directory and call
`service::status_at(dir, &home)` with the resolved home. It returns
`installed: false`. Apply the `login` start mode to the same directory and read
the file: it now contains the generated `ExecStart` with an absolute executable
path, not `/usr/local/bin/hearth`.

## Resolution

`unit_home` expands systemd's `%h` to the user's home and `%%` to a literal
percent, so `Environment="HEARTH_HOME=%h/.hearth"` in the packaged file names the
same home the generated unit names. Doctor now reports the packaged unit as
installed for that home.

Commit: `6d1fe99`. Verified by `the_packaged_unit_is_recognized_for_the_users_home`,
which copies `packaging/hearth.service` into a temporary config directory.
