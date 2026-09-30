#!/usr/bin/env python3
"""What a real node publishes to a client watching a real Claude's conversation while the desk
types into its composer: `convo.composer` per keystroke (and how late), `pending`, and the pane's
status.

Stands up an isolated herdr and a real `kampr serve` with keystroke-latency/up.sh (its own
XDG_CONFIG_HOME, a session of its own, `sessions = []`), drives `claude --model haiku` in it, and
tears both down. Usage: node-view.py [ROOT]
"""
import json, os, subprocess, sys, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
from rpc import rpc
from ws import WS
from common import CLEAN_ENV

ROOT = sys.argv[1] if len(sys.argv) > 1 else "/tmp/kmir"
UP = os.path.join(os.path.dirname(HERE), "keystroke-latency")
PROMPT = ("Reply with exactly these two sentences and nothing else, no tools: "
          "I can keep the old config or replace it. Do you want to keep it, or would you like to replace it?")


class Feed:
    def __init__(self, ws, pane):
        self.ws, self.pane, self.log, self.lock = ws, pane, [], threading.Lock()
        threading.Thread(target=self.pump, daemon=True).start()

    def pump(self):
        while True:
            try:
                _op, payload = self.ws.frame()
            except Exception:
                return
            msg = json.loads(payload)
            t = msg.get("t")
            if t in ("pending", "convo.composer", "error") or t in ("herd", "herd.patch"):
                with self.lock:
                    self.log.append((time.perf_counter(), msg))

    def since(self, t0, kind):
        with self.lock:
            return [(at, m) for at, m in self.log if at >= t0 and m.get("t") == kind]

    def wait(self, t0, kind, pred, seconds=5.0):
        end = time.perf_counter() + seconds
        while time.perf_counter() < end:
            for at, m in self.since(t0, kind):
                if pred(m):
                    return at, m
            time.sleep(0.005)
        return None, None


def statuses(feed, pane):
    out = []
    with feed.lock:
        for at, m in feed.log:
            for bucket in (m.get("panes") or []) + (m.get("changed", {}).get("panes") or []):
                if bucket.get("id") == pane and "agent_status" in bucket:
                    out.append((at, bucket["agent_status"]))
    return out


def main():
    env = dict(CLEAN_ENV, NAME="kamprmir")
    subprocess.run(["bash", os.path.join(UP, "up.sh"), ROOT], env=env, check=True)
    cfg = {}
    for line in open(os.path.join(ROOT, "env")):
        k, v = line.replace("export ", "").strip().split("=", 1)
        cfg[k] = v.strip('"')
    sock = cfg["HERDR_SOCKET_PATH"]
    call = lambda m, p=None: rpc(m, p or {}, sock_path=sock)["result"]
    cwd = subprocess.run(["mktemp", "-d", "/tmp/kampr-mir-XXXXXX"], capture_output=True, text=True).stdout.strip()
    try:
        ws_ = call("workspace.create", {"label": "claude", "cwd": cwd, "focus": False})
        local = ws_["root_pane"]["pane_id"]
        send = lambda text: call("pane.send_text", {"pane_id": local, "text": text})
        screen = lambda: call("pane.read", {"pane_id": local, "source": "visible", "format": "text",
                                            "strip_ansi": True})["read"]["text"]
        time.sleep(1)
        send("claude --model haiku\r")
        for _ in range(120):
            time.sleep(0.5)
            s = screen()
            if "trust this folder" in s:
                send("\x1b[B"); time.sleep(0.4); send("\r"); time.sleep(3); continue
            if any(l.startswith("❯") for l in s.splitlines()):
                break
        time.sleep(2)
        send(PROMPT); time.sleep(0.5); send("\r")
        time.sleep(12)

        ws = WS("127.0.0.1", int(cfg["PORT"]), protocol=f"kampr.token.{cfg['TOKEN']}")
        pane = None
        while pane is None:
            _op, payload = ws.frame()
            msg = json.loads(payload)
            if msg.get("t") == "herd":
                for p in msg["panes"]:
                    if p["id"].endswith(local):
                        pane = p["id"]
        feed = Feed(ws, pane)
        ws.send(json.dumps({"t": "watch", "pane": pane, "conversation": True}))
        time.sleep(4)

        lat = []
        typed = ""
        for ch in "push the branch when":
            t0 = time.perf_counter()
            send(ch)
            typed += ch
            at, _ = feed.wait(t0, "convo.composer", lambda m, want=typed: m.get("text") == want, 3.0)
            lat.append(None if at is None else (at - t0) * 1000)
            time.sleep(0.25)
        got = [x for x in lat if x is not None]
        print(f"keystroke -> convo.composer: {len(got)}/{len(lat)} arrived; ms {sorted(round(x) for x in got)}")
        send("\x7f" * len(typed)); time.sleep(1.5)
        t_bang = time.perf_counter()
        send("!"); time.sleep(6)
        print("after '!':", [(round((a - t_bang) * 1000), m.get("question"), [o["label"][:40] for o in m.get("options", [])])
                             for a, m in feed.since(t_bang, "pending")])
        print("statuses after '!':", [(round((a - t_bang) * 1000), s) for a, s in statuses(feed, pane) if a >= t_bang])
        with open(os.path.join(HERE, "frames", "node-bang.txt"), "w") as f:
            f.write(screen())
        send("\x7f"); time.sleep(3)
        print("after erase:", [(m.get("question")) for a, m in feed.since(t_bang, "pending")][-1:])
        with feed.lock:
            print("composer frames:", [m.get("text") for _, m in feed.log if m.get("t") == "convo.composer"][-6:])
        ws.close()
    finally:
        subprocess.run(["bash", os.path.join(UP, "down.sh"), ROOT], env=env)
        subprocess.run(["rm", "-rf", cwd])


if __name__ == "__main__":
    main()
