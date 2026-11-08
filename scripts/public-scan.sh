#!/usr/bin/env bash
set -euo pipefail

deny_list="${HEARTH_PUBLIC_DENYLIST:-$HOME/.hearth-public-denylist}"
if [ ! -f "$deny_list" ]; then
    echo "public scan: deny list not found at $deny_list; set HEARTH_PUBLIC_DENYLIST" >&2
    exit 2
fi
if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    echo "public scan: run inside the repository" >&2
    exit 2
fi
mapfile -t files < <(git ls-files)
if [ "${#files[@]}" -eq 0 ]; then
    echo "public scan: no tracked files" >&2
    exit 2
fi

status=0
while IFS= read -r pattern; do
    case "$pattern" in
        '' | '#'*) continue ;;
    esac
    for file in "${files[@]}"; do
        if grep -Fq -- "$pattern" "$file" 2>/dev/null; then
            echo "public scan: deny-list match in $file" >&2
            status=1
        fi
    done
done <"$deny_list"

credential_pattern='(sk-[A-Za-z0-9_-]{16,}|ghp_[A-Za-z0-9]{20,}|xox[baprs]-[A-Za-z0-9-]{10,}|[0-9]{8,10}:[A-Za-z0-9_-]{30,})'
for file in "${files[@]}"; do
    if grep -Eq -- "$credential_pattern" "$file" 2>/dev/null; then
        echo "public scan: credential pattern in $file" >&2
        status=1
    fi
done

if [ "$status" -eq 0 ]; then
    echo "public scan: clean"
fi
exit "$status"
