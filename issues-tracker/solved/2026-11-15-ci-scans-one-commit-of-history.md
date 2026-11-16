# CI scans one commit of history, not the history it claims to scan

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `.github`, `scripts` |

## Observed

The workflow checks out with the default shallow clone, so `git rev-list --all`
returns one commit. Both history loops in `scripts/public-scan.sh:67` and `:81`
then scan the tip only, which repeats the tracked-file scan above them.

`.github/workflows/ci.yml:14`:

```yaml
      - uses: actions/checkout@v4
```

`scripts/public-scan.sh:67`:

```bash
mapfile -t commits < <(git rev-list --all)
for commit in "${commits[@]}"; do
```

`actions/checkout@v4` defaults to `fetch-depth: 1`. A shallow clone has one
commit and no earlier parents, so `git rev-list --all` yields one SHA. A value
that was committed and later removed is invisible, which is the case the history
scan exists to catch.

## Consequence

SPEC 19 claims a clean history and `SECURITY.md` says the history is scanned
against a private deny list before each release. The workflow that is supposed
to enforce that scans the tip twice. The local `./scripts/secret-scan.sh` works
on a full checkout, so the exposure is a maintainer who trusts the green CI job.

## Verification

Clone a scratch repository with `--depth 1` after planting a deny-list value in
an older commit, run `./scripts/public-scan.sh` with that deny list, and observe
it pass. Repeat with a full clone; it fails and names the older commit. Or add
`fetch-depth: 0` to the checkout and compare the number of commits the script
iterates.

## Resolution

The workflow's checkout now sets `fetch-depth: 0`. `scripts/test-scripts.sh`
fails when `ci.yml` does not declare that depth, so a later edit cannot quietly
return to a one-commit scan.

Commits: `c60cc43` (test), `55da6c3` (fix). Verified by `./scripts/test-scripts.sh`;
the history scan itself is exercised by the planted-credential-in-history case
in that script.
