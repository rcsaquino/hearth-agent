#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

target="${1:-x86_64-unknown-linux-gnu}"
version="$(cargo metadata --no-deps --format-version 1 \
    | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "hearth-cli"))')"
if [ -z "$version" ]; then
    echo "package: could not read the hearth-cli version from cargo metadata" >&2
    exit 2
fi

if command -v rustup >/dev/null 2>&1; then
    if ! rustup target list --installed 2>/dev/null | grep -qx "$target"; then
        echo "package: target $target is not installed; run: rustup target add $target" >&2
        exit 2
    fi
fi

cargo build --release --locked --target "$target" --bin hearth

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
cp "target/$target/release/hearth" "$stage/hearth"
chmod 755 "$stage/hearth"
cp README.md QUICKSTART.md SECURITY.md CONTRIBUTING.md LICENSE "$stage/"
cp packaging/hearth.service "$stage/hearth.service"
cp scripts/fetch-embedding-model.sh "$stage/fetch-embedding-model.sh"
chmod 755 "$stage/fetch-embedding-model.sh"

mkdir -p dist
name="hearth-$version-$target"
tar -C "$stage" -czf "dist/$name.tar.gz" .
(
    cd dist
    sha256sum "$name.tar.gz" >"$name.tar.gz.sha256"
)

host="$(rustc -vV | sed -n 's/^host: //p')"
if [ "$target" = "$host" ]; then
    "$stage/hearth" --version >/dev/null
    "$stage/hearth" --help >/dev/null
    echo "package: smoke check passed"
else
    echo "package: built for $target; smoke run skipped because the host is $host" >&2
fi
echo "package: dist/$name.tar.gz"
