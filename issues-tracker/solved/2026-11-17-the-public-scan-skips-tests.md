# The credential scan skips every test and fixture

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-17 |
| Area | `scripts` |

## Observed

`scripts/public-scan.sh` excludes `*/tests/*`, `*/fixtures/*`, and
`scripts/test-scripts.sh` through `is_excluded`. The `git grep` passes for
history repeat the same path filters. A credential committed in a test file, a
fixture, or the script is therefore never reported, in the working tree or in
history.

CI runs `scripts/ci-secret-scan.sh`, which delegates to `public-scan.sh`. It
inherits the same blind spot, so a secret pasted into a test can pass both the
local gate and CI.

The exclusion exists because tests and fixtures contain fake credential shapes
and the scan script plants one. A path-wide exclusion is broader than that need.
It also hides a real key that was pasted into a test while debugging, which is a
common leak.

## Verification

Add a line containing a credential-shaped value to a file under
`crates/hearth-tools/tests/`, commit it, and run `HEARTH_PUBLIC_DENYLIST=... ./scripts/public-scan.sh`.
Both the tree scan and the history scan stay clean. `scripts/test-scripts.sh`
plants its value in `notes.txt`, which is not excluded.

## Resolution

The path-wide exclusions are gone from the tree and history scans. The scanner
now exempts only the known placeholder values the repository's own tests use
(`sk-test-not-a-real-key`, `sk-live-abcdefghijklmnop`,
`ghp_abcdefghijklmnopqrstuvwxyz`, `xoxb-1234567890-abcdefghij`,
`AKIAABCDEFGHIJKLMNOP`, `123456789:AAFakeTokenFakeTokenFakeTokenFake`) and the
planted value and allowlist lines in the two scanner scripts. The deny-list scan
has no exemptions at all.

Commits: `1a7154a` (test), `5f7c004` (fix). Verified by
`scripts/test-scripts.sh`, which commits a credential into
`crates/demo/tests/leak.rs` and requires the scan to fail and name that file,
and by a clean scan of this repository with an empty deny list.
