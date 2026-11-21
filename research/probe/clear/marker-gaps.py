"""Whether a running Claude's session marker is ever missing or half-written while the process lives.

    python3 research/probe/clear/marker-gaps.py

A real `claude --model haiku` on a bare pty with every CLAUDE_* variable stripped (#393), its
`~/.claude/sessions/<pid>.json` read every ~1 ms on a thread through two prompts, a `/clear`, an
idle spell and `/exit`. Prints each change of state with its time, and every sample that found the
file missing or unparsable while the process was alive.
"""

import glob, json, os, re, signal, sys, threading, time

sys.path.insert(0, "research/probe")
from ptyclient import spawn, drain

WORK = "/tmp/kampr-marker-gaps-probe"
os.makedirs(WORK, exist_ok=True)
SESS = os.path.expanduser("~/.claude/sessions")
ENV = {k: "" for k in os.environ if k.startswith("CLAUDE")}

samples = []
stop = threading.Event()
t0 = time.time()


def alive(pid):
    try:
        with open(f"/proc/{pid}/stat") as f:
            return f.read().split(") ")[1].split()[0] != "Z"
    except OSError:
        return False


def watch(pid):
    path = f"{SESS}/{pid}.json"
    while not stop.is_set():
        at = time.time() - t0
        try:
            raw = open(path).read()
            try:
                d = json.loads(raw)
                state = ("ok", d.get("sessionId", "")[:8], d.get("status"))
            except ValueError:
                state = ("torn", len(raw), None)
        except FileNotFoundError:
            state = ("missing", None, None)
        samples.append((at, state, alive(pid)))
        time.sleep(0.001)


def mark(label):
    samples.append((time.time() - t0, ("step", label, None), True))
    print(f"{time.time() - t0:8.3f} {label}", flush=True)


os.chdir(WORK)
for k in list(os.environ):
    if k.startswith("CLAUDE"):
        del os.environ[k]
pid, fd = spawn(["claude", "--model", "haiku"], 100, 40)
threading.Thread(target=watch, args=(pid,), daemon=True).start()
out = drain(fd, 12)
screen = re.sub(r"\x1b\[[0-9;?]*[a-zA-Z]", "", out.decode("utf8", "replace"))
if "trust" in screen.lower():
    os.write(fd, b"\r")
    drain(fd, 6)
mark("ready")
drain(fd, 5)
mark("prompt 1")
os.write(fd, b"reply with exactly the word banana and nothing else\r")
drain(fd, 30)
mark("/clear")
os.write(fd, b"/clear\r")
drain(fd, 8)
mark("prompt 2")
os.write(fd, b"reply with exactly the word cherry and nothing else\r")
drain(fd, 30)
mark("/exit")
os.write(fd, b"/exit\r")
drain(fd, 5)
stop.set()
time.sleep(0.05)
try:
    os.kill(pid, signal.SIGKILL)
except ProcessLookupError:
    pass

last = None
gaps = []
seen = False
exit_at = next(at for at, s, _ in samples if s == ("step", "/exit", None))
for at, state, live in samples:
    if state[0] == "step":
        continue
    if state != last:
        print(f"{at:8.3f} {state} alive={live}")
        last = state
    if state[0] != "ok" and live and at < exit_at and seen:
        gaps.append((at, state))
    seen |= state[0] == "ok"
print(f"samples={len(samples)}  gaps while alive before /exit={len(gaps)}")
for g in gaps[:20]:
    print("  gap", g)
