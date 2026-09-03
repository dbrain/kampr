#!/usr/bin/env bash
# Does herdr forward OSC 52 — the clipboard write Claude Code's `/copy` emits — on any path a node
# can reach it by: the `terminal session observe` stream, a `pane.read` snapshot, or the fuller
# `terminal session control` attach?
#
# Runs in a throwaway XDG_CONFIG_HOME on a named session, so the operator's own herd is never a
# participant. Leaves nothing running.
set -u
cd "$(dirname "$0")/../../.." || exit 1

ROOT=$(mktemp -d /tmp/herdr-osc52-XXXXXX)
mkdir -p "$ROOT/herdr"
printf 'onboarding = false\n' > "$ROOT/herdr/config.toml"
export XDG_CONFIG_HOME="$ROOT"
export HERDR_CONFIG_PATH="$ROOT/herdr/config.toml"
unset HERDR_SESSION
NAME="kampr-osc52-$$"
SOCK="$ROOT/herdr/sessions/$NAME/herdr.sock"
export HERDR_SOCKET_PATH="$SOCK"
KIDS=

cleanup() {
  for p in $KIDS; do kill "$p" 2>/dev/null; done
  herdr session stop "$NAME" >/dev/null 2>&1
  sleep 0.5
  pkill -f "herdr.*$NAME" 2>/dev/null
  rm -rf "$ROOT"
}
trap cleanup EXIT

# The OSC 8 hyperlink is the positive control. perform.rs says it survives the observe stream, so
# a run that finds neither 8 nor 52 is a broken harness rather than a herdr that strips 52.
cat > "$ROOT/emit.sh" <<'EOS'
printf '\033]52;c;a2FtcHItb3NjNTItbWFya2Vy\007'
printf '\033]52;c;a2FtcHItb3NjNTItbWFya2Vy\033\\'
printf '\033]8;;https://kampr.example/osc52\033\\link\033]8;;\033\\\n'
echo DONE-OSC52
EOS

setsid herdr server --session "$NAME" </dev/null >/dev/null 2>&1 &
for _ in $(seq 60); do [ -S "$SOCK" ] && break; sleep 0.2; done
[ -S "$SOCK" ] || { echo "no socket"; exit 1; }
sleep 1

PANE=$(python3 research/probe/rpc.py pane.list '{}' | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["panes"][0]["pane_id"])')
echo "pane: $PANE"

fire() {
  herdr pane send-text "$PANE" "bash $ROOT/emit.sh" >/dev/null 2>&1
  herdr pane send-keys "$PANE" Enter >/dev/null 2>&1
  sleep 3
}

report() {
  python3 - "$1" "$2" <<'PY'
import base64, json, sys
label, path = sys.argv[1], sys.argv[2]
types = {}
esc = payload = osc8 = 0
echo = False
for line in open(path, errors="replace"):
    line = line.strip()
    if not line: continue
    try: rec = json.loads(line)
    except Exception:
        types["<not json>"] = types.get("<not json>", 0) + 1
        continue
    t = rec.get("type", "?")
    types[t] = types.get(t, 0) + 1
    blob = rec.get("bytes")
    if not blob: continue
    try: b = base64.b64decode(blob)
    except Exception: continue
    if b"\x1b]52" in b: esc += 1
    if b"\x1b]8;" in b: osc8 += 1
    if b"a2FtcHItb3NjNTItbWFya2Vy" in b: payload += 1
    if b"DONE-OSC52" in b: echo = True
print("  %s: records %s" % (label, types))
print("    frames carrying a real ESC ] 52 : %d ; carrying the payload in any form: %d" % (esc, payload))
print("    the command's own output (DONE-OSC52) arrived: %s" % echo)
print("    CONTROL — frames carrying OSC 8 (which is known to survive here): %d" % osc8)
PY
}

echo "== path 1: terminal session observe"
herdr terminal session observe "$PANE" --cols 80 --rows 24 > "$ROOT/obs.jsonl" 2>/dev/null &
KIDS="$KIDS $!"
sleep 1.5
fire
for p in $KIDS; do kill "$p" 2>/dev/null; wait "$p" 2>/dev/null; done; KIDS=
report observe "$ROOT/obs.jsonl"

echo "== path 2: pane.read"
for src in visible recent; do
  for fmt in ansi text; do
    herdr pane read "$PANE" --source "$src" --format "$fmt" --lines 200 2>/dev/null > "$ROOT/read.out"
    printf '  %-8s %-5s : real-ESC]52=%s  payload=%s  echo=%s\n' "$src" "$fmt" \
      "$(grep -ac $'\033]52' "$ROOT/read.out")" \
      "$(grep -ac 'a2FtcHItb3NjNTItbWFya2Vy' "$ROOT/read.out")" \
      "$(grep -ac 'DONE-OSC52' "$ROOT/read.out")"
  done
done

echo "== path 3: terminal session control (takes the PTY; only ever used behind pane.size)"
# stdin held open: a control child reading EOF releases the PTY and exits at once, which
# measures nothing. The fifo keeps it attached across the emit.
mkfifo "$ROOT/ctl.in"
sleep 30 > "$ROOT/ctl.in" &
KIDS="$KIDS $!"
herdr terminal session control "$PANE" --cols 80 --rows 24 > "$ROOT/ctl.jsonl" 2>/dev/null < "$ROOT/ctl.in" &
KIDS="$KIDS $!"
sleep 1.5
fire
for p in $KIDS; do kill "$p" 2>/dev/null; wait "$p" 2>/dev/null; done; KIDS=
report control "$ROOT/ctl.jsonl"

echo "== the pane's screen, to prove the command really ran:"
herdr pane read "$PANE" --source visible --format text --lines 200 2>/dev/null | grep -c DONE-OSC52
