#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

"$root/scripts/check-docs.sh"

mkdir -p "$work/repo"
cd "$work/repo"
git init -q
git config user.name "Hearth Test"
git config user.email "hearth-test@example.com"
printf 'ordinary notes\n' > notes.txt
git add notes.txt
git commit -qm initial

printf 'planted-private-value\n' > "$work/deny-list"
printf 'planted-private-value\n' > notes.txt

set +e
output="$(HEARTH_PUBLIC_DENYLIST="$work/deny-list" "$root/scripts/public-scan.sh" 2>&1)"
status=$?
set -e
if [ "$status" -eq 0 ]; then
    echo "public scan did not detect the planted value" >&2
    exit 1
fi
if printf '%s' "$output" | grep -q 'planted-private-value'; then
    echo "public scan printed the private value" >&2
    exit 1
fi
if ! printf '%s' "$output" | grep -q 'notes.txt'; then
    echo "public scan did not name the offending file" >&2
    exit 1
fi

printf 'clean notes\n' > notes.txt
git add notes.txt
git commit -qm clean
if ! HEARTH_PUBLIC_DENYLIST="$work/deny-list" "$root/scripts/public-scan.sh" >/dev/null 2>&1; then
    echo "public scan failed on a clean tree and history" >&2
    exit 1
fi

printf 'sk-test-planted-credential-value-1234567890\n' > credential.txt
git add credential.txt
git commit -qm credential
set +e
HEARTH_PUBLIC_DENYLIST="$work/deny-list" "$root/scripts/public-scan.sh" >/dev/null 2>&1
status=$?
set -e
if [ "$status" -eq 0 ]; then
    echo "public scan did not detect a credential pattern" >&2
    exit 1
fi

rm credential.txt
git add -A
git commit -qm clean-again
set +e
output="$(HEARTH_PUBLIC_DENYLIST="$work/deny-list" "$root/scripts/public-scan.sh" 2>&1)"
status=$?
set -e
if [ "$status" -eq 0 ]; then
    echo "public scan did not detect the credential in history" >&2
    exit 1
fi
if ! printf '%s' "$output" | grep -q 'credential.txt'; then
    echo "public scan did not name the historical credential file" >&2
    exit 1
fi
if printf '%s' "$output" | grep -q 'sk-test-planted-credential-value'; then
    echo "public scan printed the credential value" >&2
    exit 1
fi

mkdir -p "$work/ci-repo"
(
    cd "$work/ci-repo"
    git init -q
    git config user.name "Hearth Test"
    git config user.email "hearth-test@example.com"
    printf 'ordinary notes\n' > notes.txt
    git add notes.txt
    git commit -qm initial
    if ! PUBLIC_DENYLIST="" "$root/scripts/ci-secret-scan.sh" >/dev/null 2>&1; then
        echo "CI secret scan failed without a deny-list secret" >&2
        exit 1
    fi
)

cd "$root"
target="${1:-x86_64-unknown-linux-gnu}"
"$root/scripts/package.sh" "$target"
tarball="$(ls -t "$root"/dist/hearth-*-"$target".tar.gz | head -1)"
if [ -z "$tarball" ]; then
    echo "package script produced no tarball" >&2
    exit 1
fi
for entry in './hearth' './README.md' './QUICKSTART.md' './SECURITY.md' './CONTRIBUTING.md' './LICENSE' './hearth.service'; do
    if ! tar -tzf "$tarball" | grep -qx "$entry"; then
        echo "package tarball is missing $entry" >&2
        exit 1
    fi
done
(
    cd "$root/dist"
    sha256sum -c "$(basename "$tarball").sha256"
)

echo "script checks passed"
