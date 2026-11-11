#!/usr/bin/env bash
# Local 16.2 acceptance matrix. Live Telegram evidence lives in the M3/M6 reports.
set -euo pipefail
cd "$(dirname "$0")/.."

# The private deny list for publication lives outside the tree. For this local
# acceptance run, use an empty list so the credential-pattern scan still runs.
deny_list="$(mktemp)"
trap 'rm -f "$deny_list"' EXIT
printf '# local acceptance; private deny list belongs outside the tree\n' >"$deny_list"
export HEARTH_PUBLIC_DENYLIST="$deny_list"

echo "== build =="
cargo build -p hearth-cli >/dev/null

echo "== crash points: before_commit, after_commit, mid_stream, during_effect =="
cargo test -p hearth-eval --test s7_crash

echo "== reminders and scheduler =="
cargo test -p hearth-runtime --test s7_scheduler

echo "== telegram fake end to end =="
cargo test -p hearth-cli --test s8_telegram_daemon

echo "== daemon lifecycle and tui state =="
cargo test -p hearth-cli --test daemon_lifecycle
cargo test -p hearth-cli --lib

echo "== memory benchmark =="
cargo test -p hearth-eval --test memory_bench

echo "== prompt cache and overhead =="
cargo test -p hearth-eval --test tokens_bench

echo "== backup, doctor, and restart soak =="
./scripts/soak.sh

echo "== secrets =="
./scripts/secret-scan.sh

echo "acceptance ok: fake Telegram coverage; live Telegram evidence is in artifacts/hearth-M3* and hearth-M6* reports; 7-day soak remains a future real-time item"
