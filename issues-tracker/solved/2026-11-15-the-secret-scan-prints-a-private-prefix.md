# The public scan prints a prefix of the private value it finds

| | |
| --- | --- |
| Kind | bug |
| Reported | 2026-11-15 |
| Area | `scripts` |

## Observed

When the scan finds a deny-list value, it prints the first four characters of that
value to stderr. CI keeps stderr in the job log.

`scripts/public-scan.sh:16`:

```bash
redact() {
    local value="$1"
    if [ "${#value}" -le 4 ]; then
        printf '***'
    else
        printf '%s***' "${value:0:4}"
    fi
}
```

The function is used for every hit, including deny-list matches:

```bash
echo "public scan: deny-list match in $file (match: $(redact "$pattern"))" >&2
```

`scripts/test-scripts.sh:30` asserts only that the whole value does not appear in
the output, so a four-character prefix passes the check.

## Consequence

The deny list holds the private strings the scan exists to keep out of the
repository: names, paths, and identifiers. The release gate writes part of each
one into the artifact that is retained longest. For a value of eight or more
characters, four characters are half the value or more; for a short private
string, the prefix is enough to confirm a guess. `docs/DECISIONS.md:896` says
the script "prints only the offending file name", and the ground rules forbid
echoing a private value.

## Verification

In a temporary repository with a tracked file containing `USER_NAME`, create a
deny list containing `USER_NAME`, run the scan, and read stderr. It contains
`USER***`. The whole-value assertion in `test-scripts.sh` does not catch it.

## Resolution

`redact()` in `scripts/public-scan.sh` now prints `***` and nothing of the
value. `scripts/test-scripts.sh` asserts that neither the deny-list value nor
its first four characters, and neither the credential nor its first four
characters, appears in the output.

Commits: `dfcc86a` (test), `833d434` (fix). Verified by `./scripts/test-scripts.sh`.
