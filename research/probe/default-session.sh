#!/usr/bin/env bash
# Probe #324-#326: what `herdr server` does about the default session, and how a node can tell.
#
# Runs entirely in a throwaway XDG_CONFIG_HOME so the operator's own default server is never a
# participant. Prints what it measured; leaves nothing running.
set -u

root() {
  local dir
  dir=$(mktemp -d /tmp/herdr-probe-XXXXXX)
  mkdir -p "$dir/herdr"
  printf 'onboarding = false\n' > "$dir/herdr/config.toml"
  echo "$dir"
}

use() {
  export XDG_CONFIG_HOME="$1"
  export HERDR_CONFIG_PATH="$1/herdr/config.toml"
  unset HERDR_SOCKET_PATH HERDR_SESSION
}

stop_all() {
  for name in $(herdr session list --json 2>/dev/null | grep -o '"name":"[^"]*"' | cut -d'"' -f4); do
    herdr session stop "$name" >/dev/null 2>&1
  done
  sleep 1
}

A=$(root); use "$A"
echo "== #324: is \`--session default\` the default session, or a namesake beside it?"
setsid herdr server --session default </dev/null >/dev/null 2>&1 &
sleep 1.5
echo "default socket:          $([ -S "$A/herdr/herdr.sock" ] && echo yes || echo no)"
echo "sessions/default socket: $([ -S "$A/herdr/sessions/default/herdr.sock" ] && echo yes || echo no)"
herdr session list --json

echo
echo "== #325: a second server for a default already running"
herdr server </dev/null >/dev/null 2>"$A/second.err"; echo "bare exit=$?  stderr: $(head -1 "$A/second.err")"
herdr server --session default </dev/null >/dev/null 2>"$A/third.err"; echo "named exit=$?  stderr: $(head -1 "$A/third.err")"
echo "the first is still listed:"; herdr session list --json
stop_all

B=$(root); use "$B"
echo
echo "== #326: the list before anything runs, then list vs socket vs an answered RPC"
herdr session list --json
python3 "$(dirname "$0")/herdr-ready.py" "$B"
stop_all

rm -rf "$A" "$B"
