#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")" && pwd)"
deny_list="$(mktemp)"
trap 'rm -f "$deny_list"' EXIT

if [ -n "${PUBLIC_DENYLIST:-}" ]; then
    printf '%s\n' "$PUBLIC_DENYLIST" >"$deny_list"
elif [ "${HEARTH_FORK_PR:-0}" = "1" ]; then
    echo "CI secret scan: a fork pull request cannot read the deny-list secret; checking credential shapes only" >&2
else
    echo "CI secret scan: PUBLIC_DENYLIST is not set, so the private-string scan cannot run" >&2
    exit 2
fi

HEARTH_PUBLIC_DENYLIST="$deny_list" "$root/secret-scan.sh"
