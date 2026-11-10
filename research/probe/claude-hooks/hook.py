import json
import os
import signal
import sys
import time

DIR = os.path.dirname(os.path.abspath(__file__))
LOG = os.path.join(DIR, "run", "events.jsonl")
t0 = time.time()
raw = sys.stdin.read()
try:
    data = json.loads(raw)
except ValueError:
    data = {"raw": raw}
event = data.get("hook_event_name", sys.argv[1] if len(sys.argv) > 1 else "?")
tag = sys.argv[1] if len(sys.argv) > 1 else ""


def log(kind, **extra):
    with open(LOG, "a") as f:
        f.write(json.dumps({"t": round(time.time(), 3), "pid": os.getpid(), "tag": tag, "kind": kind, "event": event, **extra}) + "\n")


def on_term(sig, _):
    log("killed", signal=sig, waited=round(time.time() - t0, 3))
    sys.exit(1)


signal.signal(signal.SIGTERM, on_term)
signal.signal(signal.SIGHUP, on_term)
log("in", input=data)

if event in ("PermissionRequest", "Elicitation"):
    decide = os.path.join(DIR, "run", "decide.json")
    deadline = t0 + float(os.environ.get("PR_WAIT", "90"))
    while time.time() < deadline:
        if os.path.exists(decide):
            out = open(decide).read()
            os.remove(decide)
            log("decided", output=out, waited=round(time.time() - t0, 3))
            print(out)
            sys.exit(0)
        time.sleep(0.1)
    log("gave-up", waited=round(time.time() - t0, 3))
log("out", took=round(time.time() - t0, 4))
