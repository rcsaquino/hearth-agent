#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

mkdir -p "$work/repo"
cd "$work/repo"
git init -q
git config user.name "Hearth Test"
git config user.email "hearth-test@example.com"
printf 'ordinary notes\n' > notes.txt
git add notes.txt
git commit -qm initial

printf 'planted-private-value\n' > "$work/deny-list"

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
    echo "public scan failed on a clean tree" >&2
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

cd "$root"
target="${1:-x86_64-unknown-linux-gnu}"
"$root/scripts/package.sh" "$target"
tarball="$(ls -t "$root"/dist/hearth-*-"$target".tar.gz | head -1)"
if [ -z "$tarball" ]; then
    echo "package script produced no tarball" >&2
    exit 1
fi
tar -tzf "$tarball" | grep -qx './hearth'
tar -tzf "$tarball" | grep -qx './README.md'
tar -tzf "$tarball" | grep -qx './QUICKSTART.md'
(
    cd "$root/dist"
    sha256sum -c "$(basename "$tarball").sha256"
)

echo "script checks passed"
