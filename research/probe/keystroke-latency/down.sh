#!/usr/bin/env bash
set -u
ROOT=${1:?root dir}
[ -f "$ROOT/env" ] && . "$ROOT/env"
for p in $(pgrep -f "kampr serve" 2>/dev/null); do tr "\0" " " < /proc/$p/environ 2>/dev/null | grep -q "$ROOT" && kill "$p"; done
if [ -n "${HERDR_SOCKET_PATH:-}" ] && [ -S "$HERDR_SOCKET_PATH" ]; then
  HERDR_SOCKET_PATH="$HERDR_SOCKET_PATH" python3 $(dirname "$0")/../rpc.py server.stop '{}' >/dev/null 2>&1
fi
sleep 1
pkill -f "server --session ${NAME:-kamprlat}" 2>/dev/null
sleep 0.5
rm -rf "$ROOT"
echo "down"
