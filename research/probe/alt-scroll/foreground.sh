set -u
ROOT=$(mktemp -d /tmp/herdr-fg-XXXXXX)
mkdir -p "$ROOT/herdr"; printf 'onboarding = false\n' > "$ROOT/herdr/config.toml"
export XDG_CONFIG_HOME="$ROOT" HERDR_CONFIG_PATH="$ROOT/herdr/config.toml"
unset HERDR_SESSION
export HERDR_SOCKET_PATH="$ROOT/herdr/sessions/kampr-fg/herdr.sock"
setsid herdr server --session kampr-fg </dev/null >"$ROOT/s.log" 2>&1 &
for _ in $(seq 1 40); do [ -S "$HERDR_SOCKET_PATH" ] && break; sleep 0.25; done
python3 research/probe/rpc.py workspace.create '{"label":"fg","cwd":"/tmp"}' >/dev/null
P=w1:p1
fg() { python3 research/probe/rpc.py pane.process_info "{\"pane_id\":\"$P\"}" \
  | python3 -c 'import json,sys;d=json.load(sys.stdin);print(json.dumps(d.get("result") or d)[:400])'; }
send() { python3 research/probe/rpc.py pane.send_text "{\"pane_id\":\"$P\",\"text\":\"$1\"}" >/dev/null; }
echo "-- at a bare shell prompt:"; fg
send "less /tmp/kampr-scrollprobe.txt\\n"; sleep 3
echo "-- under less:"; fg
send "q"; sleep 2
send "vim /tmp/kampr-scrollprobe.txt\\n"; sleep 3
echo "-- under vim:"; fg
send "\\u001b:q!\\n"; sleep 2
send "seq 1 400\\n"; sleep 2
echo "-- back at the prompt, ring now:"
python3 research/probe/rpc.py pane.get "{\"pane_id\":\"$P\"}" | python3 -c 'import json,sys;p=json.load(sys.stdin)["result"].get("pane",{});print(p.get("scroll"))'
echo "-- and its process info:"; fg
herdr session stop kampr-fg >/dev/null 2>&1; sleep 1; rm -rf "$ROOT"
