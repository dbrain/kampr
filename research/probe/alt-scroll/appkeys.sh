set -u
ROOT=$(mktemp -d /tmp/herdr-appkeys-XXXXXX)
mkdir -p "$ROOT/herdr"; printf 'onboarding = false\n' > "$ROOT/herdr/config.toml"
export XDG_CONFIG_HOME="$ROOT" HERDR_CONFIG_PATH="$ROOT/herdr/config.toml"
unset HERDR_SESSION
export HERDR_SOCKET_PATH="$ROOT/herdr/sessions/kampr-appkeys/herdr.sock"
setsid herdr server --session kampr-appkeys </dev/null >"$ROOT/s.log" 2>&1 &
for _ in $(seq 1 40); do [ -S "$HERDR_SOCKET_PATH" ] && break; sleep 0.25; done
python3 research/probe/rpc.py workspace.create '{"label":"ak","cwd":"/tmp"}' >/dev/null
P=w1:p1
head1() { python3 research/probe/rpc.py pane.read "{\"pane_id\":\"$P\",\"source\":\"visible\",\"format\":\"text\",\"strip_ansi\":true}" \
  | python3 -c 'import json,sys;t=[l for l in json.load(sys.stdin)["result"]["read"]["text"].splitlines() if l.strip()];print(t[0][:22] if t else None)'; }
send() { python3 research/probe/rpc.py pane.send_text "{\"pane_id\":\"$P\",\"text\":\"$1\"}" >/dev/null; }
rep() { for i in $(seq 1 $1); do send "$2"; sleep 0.1; done; sleep 1; }

send "less /tmp/kampr-scrollprobe.txt\\n"; sleep 3
echo "less before:              $(head1)"
rep 6 "\\u001bOB"; echo "less after 6 app-Down:    $(head1)"
rep 3 "\\u001bOA"; echo "less after 3 app-Up:      $(head1)"
send "q"; sleep 2

send "man ls\\n"; sleep 3
echo "man before:               $(head1)"
rep 6 "\\u001bOB"; echo "man after 6 app-Down:     $(head1)"
send "q"; sleep 2

send "vim /tmp/kampr-scrollprobe.txt\\n"; sleep 3
echo "vim before:               $(head1)"
rep 6 "\\u001bOB"; echo "vim after 6 app-Down:     $(head1)"
send "\\u001b:q!\\n"; sleep 2
herdr session stop kampr-appkeys >/dev/null 2>&1; sleep 1; rm -rf "$ROOT"
