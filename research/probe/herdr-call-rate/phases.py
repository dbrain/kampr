"""What the node asks herdr for, per unit time, as the herd changes state.

Stand the isolated herdr and node up with `keystroke-latency/up.sh`, with KAMPR_CALL_LOG
exported so the node's `Herdr::call` tap (call-tap.patch) has somewhere to write. Then
`. $ROOT/env; MARKS=... python3 phases.py`, and `tally.py <log> <marks>` for the table.

The phases are the states the call rate actually differs between: nobody connected, a client
connected but watching nothing, and one or more panes watched. Everything scales with the pane
count and the watcher count separately, which is why both move here and neither moves with the
other.
"""
import json, os, sys, threading, time, queue
sys.path.insert(0, "/home/dbrain/dev/kampr/research/probe")
from rpc import rpc
from ws import WS

PORT = int(os.environ["PORT"]); TOKEN = os.environ["TOKEN"]
MARKS = open(os.environ["MARKS"], "w")
DWELL = float(os.environ.get("DWELL", "60"))

def mark(name):
    MARKS.write(f"{time.time():.6f}\t{name}\n"); MARKS.flush()
    print(f"{name} @ {time.strftime('%H:%M:%S')}", flush=True)

def phase(name):
    mark(f"begin {name}")
    time.sleep(DWELL)
    mark(f"end {name}")

ws = WS("127.0.0.1", PORT, protocol=f"kampr.token.{TOKEN}")
inbox = queue.Queue()
def drain():
    try:
        while True:
            op, payload = ws.frame()
            if op == 0x1:
                inbox.put(json.loads(payload))
    except Exception as e:
        print("drain ended:", e, flush=True)
threading.Thread(target=drain, daemon=True).start()

ids = None
while ids is None:
    msg = inbox.get(timeout=20)
    if msg.get("t") == "herd":
        ids = [p["id"] for p in msg["panes"]]
mark(f"client connected, {len(ids)} panes")

phase("C 4 panes, client connected, no watch")

ws.send(json.dumps({"t": "watch", "pane": ids[0], "scrollback": True}))
time.sleep(5)
phase("D 4 panes, 1 watched")

for pid in ids[1:3]:
    ws.send(json.dumps({"t": "watch", "pane": pid, "scrollback": True}))
time.sleep(5)
phase("E 4 panes, 3 watched")

for pid in ids[:3]:
    ws.send(json.dumps({"t": "unwatch", "pane": pid}))
time.sleep(5)
phase("F 4 panes, client connected, unwatched")
mark("done")
