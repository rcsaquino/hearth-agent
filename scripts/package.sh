#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
target="${1:-x86_64-unknown-linux-gnu}"
version="$(awk '/^\[workspace.package\]/{found=1; next} found && /^version = /{gsub(/"/, "", $3); print $3; exit}' Cargo.toml)"
if [ -z "$version" ]; then
    echo "package: workspace version not found in Cargo.toml" >&2
    exit 2
fi
if command -v rustup >/dev/null 2>&1; then
    if ! rustup target list --installed 2>/dev/null | grep -qx "$target"; then
        echo "package: target $target is not installed; run: rustup target add $target" >&2
        exit 2
    fi
fi
cargo build --release --locked --target "$target"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
cp "target/$target/release/hearth" "$stage/hearth"
cp README.md LICENSE QUICKSTART.md "$stage/"
mkdir -p dist
name="hearth-$version-$target"
tar -C "$stage" -czf "dist/$name.tar.gz" .
(
    cd dist
    sha256sum "$name.tar.gz" >"$name.tar.gz.sha256"
)
echo "package: dist/$name.tar.gz"
