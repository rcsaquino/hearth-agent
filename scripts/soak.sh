#!/usr/bin/env bash
# Bounded local soak: daemon restarts, backup/restore, doctor, and a memory run.
set -euo pipefail
cd "$(dirname "$0")/.."

BIN="${HEARTH_BIN:-./target/debug/hearth}"
ITERATIONS="${HEARTH_SOAK_ITERATIONS:-3}"
HOME_DIR="$(mktemp -d)"
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

"$BIN" init --non-interactive --home "$HOME_DIR" \
  --telegram-token fake-token --allowed-user-id 1 \
  --base-url http://127.0.0.1:9/v1 --api-key fake-key \
  --model test-model --agent-name Hearth \
  --timezone UTC >/dev/null

# Remove Telegram so the soak never contacts a real service.
python3 - "$HOME_DIR" <<'PY'
import json, sys
path = sys.argv[1] + "/settings.json"
with open(path) as handle:
    settings = json.load(handle)
settings.pop("telegram", None)
with open(path, "w") as handle:
    json.dump(settings, handle, indent=2)
    handle.write("\n")
PY
chmod 600 "$HOME_DIR/settings.json"

started=$(date +%s)
completed=0
for iteration in $(seq 1 "$ITERATIONS"); do
  "$BIN" daemon --home "$HOME_DIR" >/dev/null 2>&1 &
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

  "$BIN" status --home "$HOME_DIR" --json >/dev/null
  "$BIN" backup --home "$HOME_DIR" >/dev/null
  kill -TERM "$DAEMON_PID"
  wait "$DAEMON_PID" 2>/dev/null || true
  DAEMON_PID=""

  backup="$(find "$HOME_DIR/backups" -name '*.db' -type f | sort | tail -1)"
  [ -n "$backup" ] || { echo "no backup was written" >&2; exit 1; }
  "$BIN" restore "$backup" --home "$HOME_DIR" >/dev/null
  "$BIN" doctor --home "$HOME_DIR" >/dev/null
  completed=$((completed + 1))
done

"$BIN" eval memory --json >/dev/null
duration=$(( $(date +%s) - started ))
echo "soak ok: iterations=$completed duration=${duration}s home=$HOME_DIR"
