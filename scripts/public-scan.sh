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
    printf '***'
}

is_known_fixture() {
    path="$1"
    text="$2"
    case "$path" in
        crates/*/tests/* | crates/*/fixtures/*)
            case "$text" in
                *sk-test-not-a-real-key* | \
                *sk-live-abcdefghijklmnop* | \
                *ghp_abcdefghijklmnopqrstuvwxyz* | \
                *xoxb-1234567890-abcdefghij* | \
                *AKIAABCDEFGHIJKLMNOP* | \
                *123456789:AAFakeTokenFakeTokenFakeTokenFake*)
                    return 0
                    ;;
            esac
            ;;
        scripts/test-scripts.sh)
            case "$text" in
                *sk-test-planted-credential-value*) return 0 ;;
            esac
            ;;
        scripts/public-scan.sh)
            case "$text" in
                *sk-test-not-a-real-key* | \
                *sk-test-planted-credential-value* | \
                *sk-live-abcdefghijklmnop* | \
                *ghp_abcdefghijklmnopqrstuvwxyz* | \
                *xoxb-1234567890-abcdefghij* | \
                *AKIAABCDEFGHIJKLMNOP* | \
                *123456789:AAFakeTokenFakeTokenFakeTokenFake*)
                    return 0
                    ;;
            esac
            ;;
    esac
    return 1
}

status=0
mapfile -t files < <(git ls-files)

for file in "${files[@]}"; do
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
    while IFS= read -r line; do
        [ -n "$line" ] || continue
        is_known_fixture "$file" "$line" && continue
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
        done < <(git grep -I -n -F -- "$pattern" "$commit" -- . 2>/dev/null || true)
    done <"$deny_list"
done

for commit in "${commits[@]}"; do
    while IFS= read -r line; do
        [ -n "$line" ] || continue
        rest="${line#*:}"
        file="${rest%%:*}"
        is_known_fixture "$file" "$line" && continue
        match="${line##*:}"
        echo "public scan: credential pattern in $commit:$file (match: $(redact "$match"))" >&2
        status=1
    done < <(git grep -I -n -E -- "$credential_pattern" "$commit" -- . 2>/dev/null || true)
done

if [ "$status" -eq 0 ]; then
    echo "public scan: clean"
fi
exit "$status"
