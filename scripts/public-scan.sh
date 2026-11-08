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

credential_pattern='(sk-[A-Za-z0-9_-]{16,}|sk-or-v1-[A-Za-z0-9_-]{16,}|sk-ant-[A-Za-z0-9_-]{16,}|gh[pousr]_[A-Za-z0-9]{20,}|xox[baprs]-[A-Za-z0-9-]{10,}|AKIA[0-9A-Z]{16}|[0-9]{8,10}:[A-Za-z0-9_-]{30,}|-----BEGIN (RSA|OPENSSH|EC|PRIVATE) KEY-----)'

redact() {
    local value="$1"
    if [ "${#value}" -le 4 ]; then
        printf '***'
    else
        printf '%s***' "${value:0:4}"
    fi
}

is_excluded() {
    case "$1" in
        scripts/test-scripts.sh | */tests/* | */fixtures/* | */.devhome/*)
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

status=0
mapfile -t files < <(git ls-files)

for file in "${files[@]}"; do
    is_excluded "$file" && continue
    while IFS= read -r pattern; do
        case "$pattern" in
            '' | '#'*) continue ;;
        esac
        if grep -Fq -- "$pattern" "$file" 2>/dev/null; then
            echo "public scan: deny-list match in $file (match: $(redact "$pattern"))" >&2
            status=1
        fi
    done <"$deny_list"
done

for file in "${files[@]}"; do
    is_excluded "$file" && continue
    while IFS= read -r line; do
        [ -n "$line" ] || continue
        match="${line#*:}"
        echo "public scan: credential pattern in $file (match: $(redact "$match"))" >&2
        status=1
    done < <(grep -nE -- "$credential_pattern" "$file" 2>/dev/null || true)
done

if grep -qE '(^|/)\.env$' <<<"$(printf '%s\n' "${files[@]}")"; then
    echo "public scan: a tracked .env file is not allowed" >&2
    status=1
fi

mapfile -t commits < <(git rev-list --all)
for commit in "${commits[@]}"; do
    while IFS= read -r pattern; do
        case "$pattern" in
            '' | '#'*) continue ;;
        esac
        while IFS= read -r line; do
            [ -n "$line" ] || continue
            echo "public scan: deny-list match in $commit (match: $(redact "$pattern"))" >&2
            status=1
        done < <(git grep -I -n -F -- "$pattern" "$commit" -- . \
            ':(exclude)scripts/test-scripts.sh' ':(exclude)*/tests/*' ':(exclude)*/fixtures/*' 2>/dev/null || true)
    done <"$deny_list"
done

for commit in "${commits[@]}"; do
    while IFS= read -r line; do
        [ -n "$line" ] || continue
        rest="${line#*:}"
        file="${rest%%:*}"
        match="${line##*:}"
        echo "public scan: credential pattern in $commit:$file (match: $(redact "$match"))" >&2
        status=1
    done < <(git grep -I -n -E -- "$credential_pattern" "$commit" -- . \
        ':(exclude)scripts/test-scripts.sh' ':(exclude)*/tests/*' ':(exclude)*/fixtures/*' 2>/dev/null || true)
done

if [ "$status" -eq 0 ]; then
    echo "public scan: clean"
fi
exit "$status"
