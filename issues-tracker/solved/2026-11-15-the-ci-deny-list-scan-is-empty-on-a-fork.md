# The deny-list half of the CI secret scan is empty on a fork

| | |
| --- | --- |
| Kind | change |
| Reported | 2026-11-15 |
| Area | `.github`, `scripts` |

## Observed

`scripts/public-scan.sh` refuses to run without a deny list and exits 2
(`public-scan.sh:4`), by design. The CI wrapper defeats that check by handing it
an empty file when the repository secret is unset.

`.github/workflows/ci.yml:14`:

```yaml
      - name: Secret scan
        env:
          PUBLIC_DENYLIST: ${{ secrets.HEARTH_PUBLIC_DENYLIST }}
        run: ./scripts/ci-secret-scan.sh
```

`scripts/ci-secret-scan.sh:8`:

```bash
deny_list="$(mktemp)"
...
if [ -n "${PUBLIC_DENYLIST:-}" ]; then
    printf '%s\n' "$PUBLIC_DENYLIST" >"$deny_list"
fi
HEARTH_PUBLIC_DENYLIST="$deny_list" "$root/secret-scan.sh"
```

The file always exists. With no repository secret, it is empty, so the deny-list
loop in `public-scan.sh:41` has nothing to search for and prints `public scan:
clean`. The separate credential-pattern scan still runs, so this is the private
string half only.

Repository secrets are not exposed to `pull_request` runs from forks, which is
the case a public repository most needs covered. `scripts/test-scripts.sh:88`
asserts that the wrapper passes with an empty `PUBLIC_DENYLIST`, so the fallback
is intentional and the claim is what conflicts with it.

SPEC 19 says "A required check fails on any private string or file".
`SECURITY.md` repeats the guarantee. On a fork PR the check passes for every
private string, because it never learns one.

## What should happen

Either the job must distinguish "no deny list was supplied" from "the deny list
is empty" and fail outside the pull_request-from-fork case, or the documentation
must state that the private-string check runs only where the secret exists and
the fork check covers credential shapes only. The current text promises a check
that cannot run.

## Verification

Run `PUBLIC_DENYLIST= ./scripts/ci-secret-scan.sh` on a tree containing a
private string that is in a local deny list. It exits 0 and prints `public scan:
clean`. Run `./scripts/secret-scan.sh` without `HEARTH_PUBLIC_DENYLIST`; it exits
2 with the missing-list message.

## Resolution

`scripts/ci-secret-scan.sh` now exits 2 when `PUBLIC_DENYLIST` is unset, instead
of scanning an empty list and reporting clean. The one exception is a fork pull
request, signalled by `HEARTH_FORK_PR=1` from
`github.event.pull_request.head.repo.fork`; that run prints why the private-string
half cannot run and checks credential shapes only. SPEC 19 and SECURITY.md state
the limitation and name the maintainer scan before merge as what covers it.

Commits: `6238ce3` (test), `62b48c2` (fix). Verified by `./scripts/test-scripts.sh`,
which asserts a missing list fails and a fork run succeeds with the notice.
