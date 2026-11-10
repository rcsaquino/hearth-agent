#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")" && pwd)"
deny_list="$(mktemp)"
trap 'rm -f "$deny_list"' EXIT

if [ -n "${PUBLIC_DENYLIST:-}" ]; then
    printf '%s\n' "$PUBLIC_DENYLIST" >"$deny_list"
fi

HEARTH_PUBLIC_DENYLIST="$deny_list" "$root/secret-scan.sh"
