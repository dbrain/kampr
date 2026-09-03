#!/usr/bin/env bash
# Probe #442 follow-up: stand up an isolated herdr + an out-of-process kampr node, and print
# everything a measuring client needs. Leaves both running; `down.sh` takes them away.
#
# Keep ROOT short — /tmp/kl, not a scratch path under a session id. The session socket lands at
# $ROOT/hconf/herdr/sessions/$NAME/herdr.sock and sun_path is 108 bytes; herdr refuses to start
# with "local socket name length exceeds capacity" and nothing else says why.
set -eu
ROOT=${1:?root dir}
KAMPR=${KAMPR:-$(cd "$(dirname "$0")/../../.." && pwd)/target/debug/kampr}
HERDR=${HERDR:-herdr}
NAME=${NAME:-kamprlat}

rm -rf "$ROOT"; mkdir -p "$ROOT/hconf/herdr" "$ROOT/kconf" "$ROOT/kstate"
printf 'onboarding = false\n' > "$ROOT/hconf/herdr/config.toml"
export XDG_CONFIG_HOME="$ROOT/hconf"
unset HERDR_SOCKET_PATH HERDR_SESSION || true

setsid "$HERDR" server --session "$NAME" </dev/null >"$ROOT/herdr.log" 2>&1 &
SOCK="$ROOT/hconf/herdr/sessions/$NAME/herdr.sock"
for _ in $(seq 200); do [ -S "$SOCK" ] && break; sleep 0.1; done
[ -S "$SOCK" ] || { echo "no herdr socket" >&2; exit 1; }

HERDR_SOCKET_PATH="$SOCK" python3 $(dirname "$0")/../rpc.py workspace.create '{"label":"kampr","cwd":"/tmp"}' >/dev/null

PORT=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')
export KAMPR_CONFIG_DIR="$ROOT/kconf" KAMPR_STATE_DIR="$ROOT/kstate"
"$KAMPR" init --name latnode --bind "127.0.0.1:$PORT" --origin "http://127.0.0.1:$PORT" >"$ROOT/init.log" 2>&1
python3 - "$ROOT/kconf/config.toml" "$SOCK" <<'PY'
import sys, re
path, sock = sys.argv[1], sys.argv[2]
text = open(path).read()
text = re.sub(r'(?m)^socket = .*$', f'socket = "{sock}"', text)
if 'sessions =' not in text:
    text = text.replace(f'socket = "{sock}"', f'socket = "{sock}"\nsessions = []')
else:
    text = re.sub(r'(?m)^sessions = .*$', 'sessions = []', text)
text = re.sub(r'(?m)^check = true$', 'check = false', text)
open(path, 'w').write(text)
PY
CODE=$("$KAMPR" pair </dev/null | head -1)
nohup "$KAMPR" serve </dev/null >"$ROOT/node.log" 2>&1 &
NODE_PID=$!
disown || true
for _ in $(seq 200); do curl -sf "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1 && break; sleep 0.1; done
TOKEN=$(curl -s -X POST "http://127.0.0.1:$PORT/auth/pair" -H 'Content-Type: application/json' \
  -d "{\"code\":\"$CODE\",\"device_name\":\"probe\"}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["token"])')
cat > "$ROOT/env" <<ENV
export ROOT="$ROOT"
export HERDR_SOCKET_PATH="$SOCK"
export XDG_CONFIG_HOME="$ROOT/hconf"
export PORT=$PORT
export TOKEN=$TOKEN
export NAME=$NAME
export NODE_PID=$NODE_PID
ENV
echo "ok port=$PORT sock=$SOCK"
