#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

for file in README.md QUICKSTART.md SECURITY.md CONTRIBUTING.md AGENTS.md LICENSE; do
    if [ ! -s "$file" ]; then
        echo "docs check: $file is missing or empty" >&2
        exit 1
    fi
done

for placeholder in 123456789 example.com '~/.hearth'; do
    if ! grep -q -- "$placeholder" README.md QUICKSTART.md SECURITY.md; then
        echo "docs check: placeholder $placeholder is missing from the docs" >&2
        exit 1
    fi
done

for script in scripts/package.sh scripts/public-scan.sh scripts/secret-scan.sh; do
    if [ ! -x "$script" ]; then
        echo "docs check: $script is not executable" >&2
        exit 1
    fi
done
if [ ! -s packaging/hearth.service ]; then
    echo "docs check: packaging/hearth.service is missing" >&2
    exit 1
fi

credential_pattern='(sk-[A-Za-z0-9_-]{16,}|gh[pousr]_[A-Za-z0-9]{20,}|[0-9]{8,10}:[A-Za-z0-9_-]{30,}|AKIA[0-9A-Z]{16})'
if grep -nE -- "$credential_pattern" README.md QUICKSTART.md SECURITY.md CONTRIBUTING.md AGENTS.md >/dev/null 2>&1; then
    echo "docs check: a credential-shaped value appears in the docs" >&2
    exit 1
fi

echo "docs check: ok"
