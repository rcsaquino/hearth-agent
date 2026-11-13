#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

./scripts/check-docs.sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
# Daemon tests spawn child processes; serializing the workspace tests keeps
# startup waits deterministic on a small host without changing product behavior.
cargo test --workspace -- --test-threads=1
