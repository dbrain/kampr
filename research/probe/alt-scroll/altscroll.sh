set -u
ROOT=$(mktemp -d /tmp/herdr-altscroll-XXXXXX)
mkdir -p "$ROOT/herdr"; printf 'onboarding = false\n' > "$ROOT/herdr/config.toml"
export XDG_CONFIG_HOME="$ROOT" HERDR_CONFIG_PATH="$ROOT/herdr/config.toml"
unset HERDR_SESSION
export HERDR_SOCKET_PATH="$ROOT/herdr/sessions/kampr-altscroll2/herdr.sock"
setsid herdr server --session kampr-altscroll2 </dev/null >"$ROOT/s.log" 2>&1 &
for _ in $(seq 1 40); do [ -S "$HERDR_SOCKET_PATH" ] && break; sleep 0.25; done
python3 research/probe/rpc.py workspace.create '{"label":"as","cwd":"/tmp"}' >/dev/null
P=w1:p1
head3() { python3 research/probe/rpc.py pane.read "{\"pane_id\":\"$P\",\"source\":\"visible\",\"format\":\"text\",\"strip_ansi\":true}" \
  | python3 -c 'import json,sys;t=[l for l in json.load(sys.stdin)["result"]["read"]["text"].splitlines() if l.strip()];print(" | ".join(x[:24] for x in t[:2]))'; }
send() { python3 research/probe/rpc.py pane.send_text "{\"pane_id\":\"$P\",\"text\":\"$1\"}" >/dev/null; }
rep() { for i in $(seq 1 $1); do send "$2"; sleep 0.1; done; sleep 1; }

echo "== less"
send "less /tmp/kampr-scrollprobe.txt\\n"; sleep 3
echo "  before:        $(head3)"
rep 6 "\\u001b[<64;40;20M"; echo "  after SGR x6:  $(head3)"
rep 6 "\\u001b[B";          echo "  after Down x6: $(head3)"
rep 1 "\\u001b[6~";         echo "  after PageDn:  $(head3)"
send "q"; sleep 2
echo "  after q:       $(head3)"

echo "== vim"
send "vim /tmp/kampr-scrollprobe.txt\\n"; sleep 3
echo "  before:        $(head3)"
rep 6 "\\u001b[<64;40;20M"; echo "  after SGR x6:  $(head3)"
rep 50 "\\u001b[B";         echo "  after Down x50:$(head3)"
rep 3 "\\u0005";            echo "  after ctrl-E:  $(head3)"
send "\\u001b:q!\\n"; sleep 2
echo "  after quit:    $(head3)"
herdr session stop kampr-altscroll2 >/dev/null 2>&1; sleep 1; rm -rf "$ROOT"
