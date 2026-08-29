#!/usr/bin/env python3
"""How long between a known agent label appearing on a pane and herdr publishing
default_known_agent_idle_fallback -> idle, when nothing on screen matches a rule?"""
import json, os, shutil, subprocess, sys, time
sys.path.insert(0, "/home/dbrain/dev/kampr/research/probe")
from rpc import rpc
SP = os.path.dirname(os.path.abspath(__file__))
NAME = "kampr-probe-drop-%d" % os.getpid()
SOCK = os.path.join(os.path.expanduser("~/.config"), "herdr", "sessions", NAME, "herdr.sock")
BIN = os.path.join(SP, "bin")
CLEAN_ENV = {k: v for k, v in os.environ.items() if not k.startswith("HERDR_") and k != "TERM_PROGRAM"}

def call(m, p=None):
    r = rpc(m, p or {}, sock_path=SOCK)
    if not r or "error" in r: raise SystemExit("%s: %s" % (m, r))
    return r["result"]

def start():
    subprocess.Popen(["herdr", "server", "--session", NAME], env=CLEAN_ENV,
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(200):
        if os.path.exists(SOCK): time.sleep(0.8); return
        time.sleep(0.1)
    raise SystemExit("no server")

def stop():
    try: rpc("server.stop", {}, sock_path=SOCK)
    except Exception: pass
    for _ in range(80):
        if not os.path.exists(SOCK): break
        time.sleep(0.1)
    shutil.rmtree(os.path.dirname(SOCK), ignore_errors=True)

def main():
    os.makedirs(BIN, exist_ok=True)
    shutil.copy("/usr/bin/sleep", os.path.join(BIN, "sleep"))
    shutil.copy("/bin/bash", os.path.join(BIN, "claude"))
    start()
    res = []
    try:
        for run in range(5):
            pane = call("workspace.create", {"label": "g%d" % run, "cwd": SP})["root_pane"]["pane_id"]
            time.sleep(1.5)
            call("pane.send_text", {"pane_id": pane, "text": "%s/sleep %d; %s/claude -c '%s/sleep 25; :'\r" % (BIN, 2 + 3 * run, BIN, BIN)})
            t_label = t_idle = None; t0 = time.time(); trace = []
            while time.time() - t0 < 14.0 + 3 * run:
                t = time.time()
                g = rpc("pane.get", {"pane_id": pane}, sock_path=SOCK)["result"]["pane"]
                a, s = g.get("agent"), g.get("agent_status")
                if trace and trace[-1][1:] == (a, s): pass
                else: trace.append((t, a, s))
                if a == "claude" and t_label is None: t_label = t
                if t_label and s == "idle" and t_idle is None: t_idle = t; break
                d = 0.04 - (time.time() - t)
                if d > 0: time.sleep(d)
            if t_label and t_idle:
                res.append(t_idle - t_label)
                print("  run %d: label->idle %.2f s   trace=%s" % (run, t_idle - t_label,
                      [(round(x - t0, 2), a, s) for x, a, s in trace]), flush=True)
            else:
                print("  run %d: no transition (label=%s idle=%s)" % (run, t_label, t_idle), flush=True)
            call("pane.close", {"pane_id": pane}); time.sleep(0.8)
        if res:
            print("\nlabel -> idle fallback: n=%d min %.2f max %.2f mean %.2f"
                  % (len(res), min(res), max(res), sum(res) / len(res)), flush=True)
    finally:
        stop()

main()
