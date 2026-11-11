#!/usr/bin/env bash
# Bounded local soak: daemon restarts, backup/restore, doctor, and a memory run.
set -euo pipefail
cd "$(dirname "$0")/.."

BIN="${HEARTH_BIN:-./target/debug/hearth}"
ITERATIONS="${HEARTH_SOAK_ITERATIONS:-3}"
HOME_DIR="$(mktemp -d /tmp/hearth-soak.XXXXXX)"
DAEMON_PID=""
cleanup() {
  if [ -n "$DAEMON_PID" ] && kill -0 "$DAEMON_PID" 2>/dev/null; then
    kill -TERM "$DAEMON_PID" 2>/dev/null || true
    wait "$DAEMON_PID" 2>/dev/null || true
  fi
  rm -rf "$HOME_DIR"
}
trap cleanup EXIT

[ -x "$BIN" ] || { echo "missing binary: $BIN" >&2; exit 1; }

cat >"$HOME_DIR/settings.json" <<JSON
{
  "identity": {"agent_name": "Hearth", "timezone": "UTC"},
  "models": {
    "kind": "openai-compatible",
    "base_url": "http://127.0.0.1:9/v1",
    "api_key": "fake-key",
    "model": "test-model",
    "thinking": "default"
  },
  "daemon": {"start": "tui", "keep_running": true}
}
JSON
chmod 600 "$HOME_DIR/settings.json"

started=$(date +%s)
completed=0
echo "== backup and restore =="
cargo test -p hearth-runtime --test backup
echo "== doctor =="
cargo test -p hearth-cli --test doctor_offline
for iteration in $(seq 1 "$ITERATIONS"); do
  HEARTH_DAEMON=1 HEARTH_HOME="$HOME_DIR" "$BIN" >/dev/null 2>&1 &
  DAEMON_PID=$!
  ready=0
  for _ in $(seq 1 100); do
    if [ -S "$HOME_DIR/hearth.sock" ] && kill -0 "$DAEMON_PID" 2>/dev/null; then
      ready=1
      break
    fi
    sleep 0.1
  done
  [ "$ready" -eq 1 ] || { echo "daemon did not become ready" >&2; exit 1; }

  [ "$(cat "$HOME_DIR/hearth.lock")" = "$DAEMON_PID" ] || {
    echo "the lock does not name the daemon" >&2
    exit 1
  }

  kill -TERM "$DAEMON_PID"
  wait "$DAEMON_PID" 2>/dev/null || true
  DAEMON_PID=""
  completed=$((completed + 1))
done

echo "== memory benchmark =="
cargo test -p hearth-eval --test memory_bench
duration=$(( $(date +%s) - started ))
echo "soak ok: iterations=$completed duration=${duration}s home=$HOME_DIR"
